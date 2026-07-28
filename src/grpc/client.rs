//! Yellowstone gRPC 客户端 - 超低延迟 DEX 事件订阅
//!
//! 支持多种事件输出模式：
//! - Unordered: 10-20μs 极低延迟
//! - MicroBatch: 50-200μs 微批次有序
//! - StreamingOrdered: 0.1-5ms 流式有序
//! - Ordered: 1-50ms 完全有序

use super::buffers::{MicroBatchBuffer, SlotBuffer};
use super::types::*;
use crate::core::{now_micros, EventMetadata}; // 导入高性能时钟
use crate::instr::read_pubkey_fast;
use crate::logs::timestamp_to_microseconds;
use crate::DexEvent;
use crossbeam_queue::ArrayQueue;
use futures::{SinkExt, StreamExt};
use log::error;
use memchr::memmem;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::sync::{oneshot, watch};
use tokio::time::{Duration, Instant};
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::*;

static PROGRAM_DATA_FINDER: Lazy<memmem::Finder> =
    Lazy::new(|| memmem::Finder::new(b"Program data: "));

#[derive(Clone, Default)]
struct SubscriptionFilters {
    transaction: Vec<TransactionFilter>,
    account: Vec<AccountFilter>,
    acknowledgement: Option<String>,
}

enum StreamExit {
    Stopped,
    Closed,
}

struct SubscriptionLease(Arc<AtomicBool>);

impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct AcknowledgementGuard {
    acknowledgement: String,
    filters_tx: watch::Sender<SubscriptionFilters>,
    waiters: Arc<Mutex<HashMap<String, oneshot::Sender<Result<SubscriptionActivation, String>>>>>,
    armed: bool,
}

impl AcknowledgementGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AcknowledgementGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.acknowledgement);
        self.filters_tx.send_modify(|current| {
            if current.acknowledgement.as_deref() == Some(self.acknowledgement.as_str()) {
                current.acknowledgement = None;
            }
        });
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubscriptionHealth {
    pub connected: bool,
    pub stream_epoch: u64,
    pub reconnect_count: u64,
    pub last_error: Option<String>,
    pub last_receive_timestamp_us: Option<i64>,
    pub last_receive_slot: Option<u64>,
    pub input_queue_drop_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionActivation {
    pub stream_epoch: u64,
    pub activated_after_slot: u64,
    pub acknowledged_at_us: i64,
}

#[derive(Default)]
struct SubscriptionHealthState {
    connected: AtomicBool,
    stream_epoch: AtomicU64,
    reconnect_count: AtomicU64,
    last_error: Mutex<Option<String>>,
    last_receive_timestamp_us: AtomicI64,
    last_receive_slot: AtomicU64,
    has_received_slot: AtomicBool,
    input_queue_drop_count: AtomicU64,
}

// ==================== YellowstoneGrpc 客户端 ====================

#[derive(Clone)]
pub struct YellowstoneGrpc {
    endpoint: String,
    token: Option<String>,
    config: ClientConfig,
    filters_tx: watch::Sender<SubscriptionFilters>,
    stop_tx: watch::Sender<()>,
    subscription_active: Arc<AtomicBool>,
    health: Arc<SubscriptionHealthState>,
    acknowledgement_nonce: Arc<AtomicU64>,
    acknowledgement_waiters:
        Arc<Mutex<HashMap<String, oneshot::Sender<Result<SubscriptionActivation, String>>>>>,
}

impl YellowstoneGrpc {
    pub fn new(
        endpoint: String,
        token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        crate::warmup::warmup_parser();
        let (filters_tx, _) = watch::channel(SubscriptionFilters::default());
        let (stop_tx, _) = watch::channel(());
        Ok(Self {
            endpoint,
            token,
            config: ClientConfig::default(),
            filters_tx,
            stop_tx,
            subscription_active: Arc::new(AtomicBool::new(false)),
            health: Arc::new(SubscriptionHealthState::default()),
            acknowledgement_nonce: Arc::new(AtomicU64::new(0)),
            acknowledgement_waiters: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn new_with_config(
        endpoint: String,
        token: Option<String>,
        config: ClientConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        crate::warmup::warmup_parser();
        let (filters_tx, _) = watch::channel(SubscriptionFilters::default());
        let (stop_tx, _) = watch::channel(());
        Ok(Self {
            endpoint,
            token,
            config,
            filters_tx,
            stop_tx,
            subscription_active: Arc::new(AtomicBool::new(false)),
            health: Arc::new(SubscriptionHealthState::default()),
            acknowledgement_nonce: Arc::new(AtomicU64::new(0)),
            acknowledgement_waiters: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 订阅 DEX 事件（自动重连）
    pub async fn subscribe_dex_events(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
        event_type_filter: Option<EventTypeFilter>,
    ) -> Result<Arc<ArrayQueue<DexEvent>>, Box<dyn std::error::Error>> {
        let (subscription_lease, mut filters_rx, mut stop_rx) =
            self.prepare_subscription(transaction_filters, account_filters)?;
        let queue = Arc::new(ArrayQueue::new(100_000));
        let queue_clone = Arc::clone(&queue);
        let self_clone = self.clone();

        tokio::spawn(async move {
            let _subscription_lease = subscription_lease;
            let mut delay = 1u64;
            let mut first_attempt = true;
            loop {
                if first_attempt {
                    first_attempt = false;
                } else {
                    self_clone.record_reconnect();
                }
                let result = self_clone
                    .stream_events(&event_type_filter, &queue_clone, &mut filters_rx, &mut stop_rx)
                    .await;

                match result {
                    Ok(StreamExit::Stopped) => {
                        self_clone.record_stopped();
                        break;
                    }
                    Ok(StreamExit::Closed) => {
                        self_clone.record_disconnected("gRPC stream closed");
                        delay = 1;
                    }
                    Err(e) => {
                        let error = self_clone.record_disconnected(&e);
                        error!("gRPC stream failed; retrying in {}s: {}", delay, error);
                    }
                }

                tokio::select! {
                    _ = stop_rx.changed() => {
                        self_clone.record_stopped();
                        break;
                    },
                    _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                }
                delay = (delay * 2).min(60);
            }
        });

        Ok(queue)
    }

    /// 动态更新订阅过滤器
    pub async fn update_subscription(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.fail_acknowledgements("subscription update superseded");
        self.replace_subscription_filters(transaction_filters, account_filters);
        Ok(())
    }

    /// Replace the active filters and wait until Yellowstone proves they were installed.
    ///
    /// The proof is a slot update carrying a unique temporary filter label. The
    /// temporary slot filter is sent atomically with the requested filters and
    /// removed before this method returns.
    pub async fn update_subscription_acknowledged(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
        timeout: Duration,
    ) -> Result<SubscriptionActivation, Box<dyn std::error::Error>> {
        self.fail_acknowledgements("subscription update superseded");
        let acknowledgement = format!(
            "__sol_parser_subscription_ack_{}",
            self.acknowledgement_nonce.fetch_add(1, Ordering::AcqRel)
        );
        let (tx, rx) = oneshot::channel();
        self.acknowledgement_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(acknowledgement.clone(), tx);
        self.filters_tx.send_replace(SubscriptionFilters {
            transaction: transaction_filters,
            account: account_filters,
            acknowledgement: Some(acknowledgement.clone()),
        });
        let mut guard = AcknowledgementGuard {
            acknowledgement,
            filters_tx: self.filters_tx.clone(),
            waiters: Arc::clone(&self.acknowledgement_waiters),
            armed: true,
        };

        let result = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(activation))) => {
                guard.disarm();
                Ok(activation)
            }
            Ok(Ok(Err(error))) => Err(error.into()),
            Ok(Err(_)) => Err("subscription acknowledgement channel closed".into()),
            Err(_) => Err("subscription update acknowledgement timed out".into()),
        };
        result
    }

    pub async fn stop(&self) {
        self.record_stopped();
        self.stop_tx.send_replace(());
    }

    pub fn subscription_health(&self) -> SubscriptionHealth {
        let last_error =
            self.health.last_error.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        let last_receive_timestamp_us =
            self.health.last_receive_timestamp_us.load(Ordering::Acquire);
        let last_receive_slot = self
            .health
            .has_received_slot
            .load(Ordering::Acquire)
            .then(|| self.health.last_receive_slot.load(Ordering::Acquire));

        SubscriptionHealth {
            connected: self.health.connected.load(Ordering::Acquire),
            stream_epoch: self.health.stream_epoch.load(Ordering::Acquire),
            reconnect_count: self.health.reconnect_count.load(Ordering::Acquire),
            last_error,
            last_receive_timestamp_us: (last_receive_timestamp_us != 0)
                .then_some(last_receive_timestamp_us),
            last_receive_slot,
            input_queue_drop_count: self.health.input_queue_drop_count.load(Ordering::Acquire),
        }
    }

    // ==================== 核心事件流处理 ====================

    fn replace_subscription_filters(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
    ) {
        self.filters_tx.send_replace(SubscriptionFilters {
            transaction: transaction_filters,
            account: account_filters,
            acknowledgement: None,
        });
    }

    fn fail_acknowledgements(&self, error: &str) {
        self.filters_tx.send_modify(|current| current.acknowledgement = None);
        for (_, waiter) in self
            .acknowledgement_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
        {
            let _ = waiter.send(Err(error.to_string()));
        }
    }

    fn claim_subscription(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
    ) -> Result<SubscriptionLease, Box<dyn std::error::Error>> {
        self.subscription_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A gRPC subscription is already active")?;
        self.replace_subscription_filters(transaction_filters, account_filters);
        Ok(SubscriptionLease(Arc::clone(&self.subscription_active)))
    }

    fn prepare_subscription(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
    ) -> Result<
        (SubscriptionLease, watch::Receiver<SubscriptionFilters>, watch::Receiver<()>),
        Box<dyn std::error::Error>,
    > {
        let filters_rx = self.filters_tx.subscribe();
        let stop_rx = self.stop_tx.subscribe();
        let lease = self.claim_subscription(transaction_filters, account_filters)?;
        Ok((lease, filters_rx, stop_rx))
    }

    fn record_connected(&self) {
        self.health.stream_epoch.fetch_add(1, Ordering::AcqRel);
        self.health.connected.store(true, Ordering::Release);
    }

    fn record_reconnect(&self) {
        self.fail_acknowledgements("gRPC stream reconnected before subscription acknowledgement");
        self.health.reconnect_count.fetch_add(1, Ordering::AcqRel);
        self.health.connected.store(false, Ordering::Release);
    }

    fn record_stopped(&self) {
        self.fail_acknowledgements("gRPC subscription stopped before acknowledgement");
        self.health.connected.store(false, Ordering::Release);
    }

    fn record_disconnected(&self, error: &str) -> String {
        self.fail_acknowledgements("gRPC stream disconnected before subscription acknowledgement");
        let error = self.sanitize_error(error);
        *self.health.last_error.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(error.clone());
        self.health.connected.store(false, Ordering::Release);
        error
    }

    fn record_receive(&self, timestamp_us: i64, slot: Option<u64>) {
        self.health.last_receive_timestamp_us.store(timestamp_us, Ordering::Release);
        if let Some(slot) = slot {
            self.health.last_receive_slot.store(slot, Ordering::Release);
            self.health.has_received_slot.store(true, Ordering::Release);
        }
    }

    fn sanitize_error(&self, error: &str) -> String {
        let mut error = if self.endpoint.is_empty() {
            error.to_string()
        } else {
            error.replace(&self.endpoint, "[endpoint]")
        };
        if let Some(token) = self.token.as_deref().filter(|token| !token.is_empty()) {
            error = error.replace(token, "[redacted]");
        }
        error
    }

    #[inline]
    fn push_to_queue(&self, queue: &ArrayQueue<DexEvent>, mut event: DexEvent) {
        event.set_stream_epoch(self.health.stream_epoch.load(Ordering::Acquire));
        if queue.push(event).is_err() {
            self.health.input_queue_drop_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn stream_events(
        &self,
        event_filter: &Option<EventTypeFilter>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        filters_rx: &mut watch::Receiver<SubscriptionFilters>,
        stop_rx: &mut watch::Receiver<()>,
    ) -> Result<StreamExit, String> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // 构建客户端
        let mut builder = GeyserGrpcClient::build_from_shared(self.endpoint.clone())
            .map_err(|e| e.to_string())?
            .x_token(self.token.clone())
            .map_err(|e| e.to_string())?
            .max_decoding_message_size(1024 * 1024 * 1024);

        if self.config.connection_timeout_ms > 0 {
            builder =
                builder.connect_timeout(Duration::from_millis(self.config.connection_timeout_ms));
        }
        if self.config.keep_alive_interval_ms > 0 {
            builder = builder.http2_keep_alive_interval(Duration::from_millis(
                self.config.keep_alive_interval_ms,
            ));
        }
        if self.config.keep_alive_timeout_ms > 0 {
            builder = builder
                .keep_alive_timeout(Duration::from_millis(self.config.keep_alive_timeout_ms));
        }

        if self.config.enable_tls {
            builder = builder
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(|e| e.to_string())?;
        }

        let mut client = tokio::select! {
            _ = stop_rx.changed() => return Ok(StreamExit::Stopped),
            result = builder.connect() => result.map_err(|e| e.to_string())?,
        };

        let (mut subscribe_tx, mut stream) = loop {
            let filters = filters_rx.borrow_and_update().clone();
            let request = build_subscribe_request(
                &filters.transaction,
                &filters.account,
                filters.acknowledgement.as_deref(),
            );
            tokio::select! {
                _ = stop_rx.changed() => return Ok(StreamExit::Stopped),
                changed = filters_rx.changed() => {
                    changed.map_err(|_| "Subscription filter channel closed".to_string())?;
                }
                result = client.subscribe_with_request(Some(request)) => {
                    let subscription = result.map_err(|e| e.to_string())?;
                    if consume_establishment_filter_change(filters_rx)? {
                        drop(subscription);
                        continue;
                    }
                    break subscription;
                }
            }
        };

        self.record_connected();
        self.print_mode_info();

        // 初始化缓冲区
        let mut slot_buffer = SlotBuffer::new();
        let mut micro_batch = MicroBatchBuffer::new();
        let mut last_slot = 0u64;

        let order_mode = self.config.order_mode;
        let timeout_ms = self.config.order_timeout_ms;
        let batch_us = self.config.micro_batch_us;
        let check_interval = Duration::from_millis(timeout_ms / 2);
        let mut next_check = Instant::now() + check_interval;

        // Ping intervals
        let ping_interval = Duration::from_millis(self.config.keep_alive_interval_ms.max(10000));
        let mut next_ping = Instant::now() + ping_interval;

        loop {
            // Periodic timeout check for ordered modes and MicroBatch
            self.check_timeout(
                order_mode,
                &mut slot_buffer,
                &mut micro_batch,
                queue,
                timeout_ms,
                batch_us,
                &mut next_check,
                check_interval,
            );

            tokio::select! {
                _ = stop_rx.changed() => {
                    self.flush_pending(
                        order_mode,
                        &mut slot_buffer,
                        &mut micro_batch,
                        queue,
                    );
                    return Ok(StreamExit::Stopped);
                }
                // Periodic Ping
                _ = tokio::time::sleep_until((next_ping).into()), if Instant::now() >= next_ping => {
                    next_ping = Instant::now() + ping_interval;
                    let ping_request = SubscribeRequest {
                        ping: Some(SubscribeRequestPing { id: 1 }),
                        ..Default::default()
                    };
                    let Some(send_result) =
                        run_until_stopped(stop_rx, subscribe_tx.send(ping_request)).await
                    else {
                        self.flush_pending(
                            order_mode,
                            &mut slot_buffer,
                            &mut micro_batch,
                            queue,
                        );
                        return Ok(StreamExit::Stopped);
                    };
                    if let Err(e) = send_result {
                        self.flush_pending(
                            order_mode,
                            &mut slot_buffer,
                            &mut micro_batch,
                            queue,
                        );
                        return Err(e.to_string());
                    }
                }

                msg = stream.next() => {
                    match msg {
                        Some(Ok(update)) => {
                            let receive_slot = match update.update_oneof.as_ref() {
                                Some(subscribe_update::UpdateOneof::Transaction(tx)) => Some(tx.slot),
                                Some(subscribe_update::UpdateOneof::Account(account)) => {
                                    Some(account.slot)
                                }
                                Some(subscribe_update::UpdateOneof::Slot(slot)) => Some(slot.slot),
                                Some(subscribe_update::UpdateOneof::TransactionStatus(status)) => {
                                    Some(status.slot)
                                }
                                Some(subscribe_update::UpdateOneof::Block(block)) => Some(block.slot),
                                Some(subscribe_update::UpdateOneof::BlockMeta(block)) => {
                                    Some(block.slot)
                                }
                                Some(subscribe_update::UpdateOneof::Entry(entry)) => Some(entry.slot),
                                _ => None,
                            };
                            let acknowledged_at_us = now_micros();
                            self.record_receive(acknowledged_at_us, receive_slot);

                            let acknowledgement = filters_rx.borrow().acknowledgement.clone();
                            if let Some((acknowledgement, activated_after_slot)) =
                                acknowledgement.and_then(|acknowledgement| {
                                    acknowledgement_slot(&update, &acknowledgement)
                                        .map(|slot| (acknowledgement, slot))
                                })
                            {
                                let filters = filters_rx.borrow().clone();
                                let request = build_subscribe_request(
                                    &filters.transaction,
                                    &filters.account,
                                    None,
                                );
                                let Some(send_result) =
                                    run_until_stopped(stop_rx, subscribe_tx.send(request)).await
                                else {
                                    return Ok(StreamExit::Stopped);
                                };
                                if let Err(error) = send_result {
                                    return Err(error.to_string());
                                }
                                self.filters_tx.send_modify(|current| {
                                    if current.acknowledgement.as_deref()
                                        == Some(acknowledgement.as_str())
                                    {
                                        current.acknowledgement = None;
                                    }
                                });
                                filters_rx.borrow_and_update();
                                if let Some(waiter) = self
                                    .acknowledgement_waiters
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .remove(&acknowledgement)
                                {
                                    let _ = waiter.send(Ok(SubscriptionActivation {
                                        stream_epoch: self
                                            .health
                                            .stream_epoch
                                            .load(Ordering::Acquire),
                                        activated_after_slot,
                                        acknowledged_at_us,
                                    }));
                                }
                                continue;
                            }

                            // Check if it's a pong
                            if let Some(subscribe_update::UpdateOneof::Ping(_)) = update.update_oneof {
                                // Pong received (it's actually called Ping in the response too sometimes, or handled as update)
                                // Actually, Yellowstone has a dedicated Pong message in the proto
                                continue;
                            }

                            self.handle_update(
                                update, order_mode, event_filter, queue,
                                &mut slot_buffer, &mut micro_batch, &mut last_slot, batch_us
                            );
                        }
                        Some(Err(e)) => {
                            self.flush_pending(
                                order_mode,
                                &mut slot_buffer,
                                &mut micro_batch,
                                queue,
                            );
                            return Err(e.to_string());
                        }
                        None => {
                            self.flush_pending(
                                order_mode,
                                &mut slot_buffer,
                                &mut micro_batch,
                                queue,
                            );
                            return Ok(StreamExit::Closed);
                        }
                    }
                }
                changed = filters_rx.changed() => {
                    if changed.is_err() {
                        self.flush_pending(
                            order_mode,
                            &mut slot_buffer,
                            &mut micro_batch,
                            queue,
                        );
                        return Err("Subscription filter channel closed".to_string());
                    }
                    let filters = filters_rx.borrow_and_update().clone();
                    let request =
                        build_subscribe_request(
                            &filters.transaction,
                            &filters.account,
                            filters.acknowledgement.as_deref(),
                        );
                    let Some(send_result) =
                        run_until_stopped(stop_rx, subscribe_tx.send(request)).await
                    else {
                        self.flush_pending(
                            order_mode,
                            &mut slot_buffer,
                            &mut micro_batch,
                            queue,
                        );
                        return Ok(StreamExit::Stopped);
                    };
                    if let Err(e) = send_result {
                        self.flush_pending(
                            order_mode,
                            &mut slot_buffer,
                            &mut micro_batch,
                            queue,
                        );
                        return Err(e.to_string());
                    }
                }
            }
        }
    }

    fn print_mode_info(&self) {
        match self.config.order_mode {
            OrderMode::Unordered => println!("✅ Unordered Mode (10-20μs)"),
            OrderMode::Ordered => {
                println!("✅ Ordered Mode (timeout={}ms)", self.config.order_timeout_ms)
            }
            OrderMode::StreamingOrdered => {
                println!("✅ StreamingOrdered Mode (timeout={}ms)", self.config.order_timeout_ms)
            }
            OrderMode::MicroBatch => {
                println!("✅ MicroBatch Mode (window={}μs)", self.config.micro_batch_us)
            }
        }
    }

    #[inline]
    fn check_timeout(
        &self,
        mode: OrderMode,
        slot_buf: &mut SlotBuffer,
        micro_buf: &mut MicroBatchBuffer,
        queue: &Arc<ArrayQueue<DexEvent>>,
        timeout_ms: u64,
        batch_us: u64,
        next_check: &mut Instant,
        interval: Duration,
    ) {
        if Instant::now() < *next_check {
            return;
        }
        *next_check = Instant::now() + interval;

        match mode {
            OrderMode::Ordered => {
                if slot_buf.should_timeout(timeout_ms) {
                    for e in slot_buf.flush_all() {
                        self.push_to_queue(queue, e);
                    }
                }
            }
            OrderMode::StreamingOrdered => {
                if slot_buf.should_timeout(timeout_ms) {
                    for e in slot_buf.flush_streaming_timeout() {
                        self.push_to_queue(queue, e);
                    }
                }
            }
            OrderMode::MicroBatch => {
                // Periodic flush for MicroBatch mode
                let now_us = get_timestamp_us();
                if micro_buf.should_flush(now_us, batch_us) {
                    for e in micro_buf.flush() {
                        self.push_to_queue(queue, e);
                    }
                }
            }
            OrderMode::Unordered => {}
        }
    }

    fn flush_pending(
        &self,
        mode: OrderMode,
        slot_buffer: &mut SlotBuffer,
        micro_batch: &mut MicroBatchBuffer,
        queue: &Arc<ArrayQueue<DexEvent>>,
    ) {
        let events = match mode {
            OrderMode::Ordered => slot_buffer.flush_all(),
            OrderMode::StreamingOrdered => slot_buffer.flush_streaming_timeout(),
            OrderMode::MicroBatch => micro_batch.flush(),
            OrderMode::Unordered => Vec::new(),
        };
        for event in events {
            self.push_to_queue(queue, event);
        }
    }

    #[inline]
    fn handle_update(
        &self,
        update_msg: SubscribeUpdate,
        mode: OrderMode,
        filter: &Option<EventTypeFilter>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        slot_buf: &mut SlotBuffer,
        micro_buf: &mut MicroBatchBuffer,
        last_slot: &mut u64,
        batch_us: u64,
    ) {
        let block_time_us =
            timestamp_to_microseconds(&update_msg.created_at.unwrap_or_default()) as i64;
        let grpc_recv_us = get_timestamp_us();

        let Some(update) = update_msg.update_oneof else { return };

        match update {
            subscribe_update::UpdateOneof::Transaction(tx) => {
                self.handle_transaction(
                    tx,
                    mode,
                    filter,
                    queue,
                    slot_buf,
                    micro_buf,
                    last_slot,
                    batch_us,
                    grpc_recv_us,
                    block_time_us,
                );
            }
            subscribe_update::UpdateOneof::Account(acc) => {
                self.handle_account(acc, filter, queue, grpc_recv_us, block_time_us);
            }
            _ => {}
        }
    }

    #[inline]
    fn handle_transaction(
        &self,
        tx: SubscribeUpdateTransaction,
        mode: OrderMode,
        filter: &Option<EventTypeFilter>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        slot_buf: &mut SlotBuffer,
        micro_buf: &mut MicroBatchBuffer,
        last_slot: &mut u64,
        batch_us: u64,
        grpc_us: i64,
        block_us: i64,
    ) {
        let slot = tx.slot;

        match mode {
            OrderMode::Unordered => {
                for e in parse_transaction_core(&tx, grpc_us, Some(block_us), filter.as_ref()) {
                    self.push_to_queue(queue, e);
                }
            }
            OrderMode::Ordered => {
                if slot > *last_slot && *last_slot > 0 {
                    for e in slot_buf.flush_before(slot) {
                        self.push_to_queue(queue, e);
                    }
                }
                *last_slot = slot;
                for (idx, e) in
                    parse_transaction_to_vec(&tx, grpc_us, Some(block_us), filter.as_ref())
                {
                    slot_buf.push(slot, idx, e);
                }
            }
            OrderMode::StreamingOrdered => {
                for (idx, e) in
                    parse_transaction_to_vec(&tx, grpc_us, Some(block_us), filter.as_ref())
                {
                    for evt in slot_buf.push_streaming(slot, idx, e) {
                        self.push_to_queue(queue, evt);
                    }
                }
            }
            OrderMode::MicroBatch => {
                for (idx, e) in
                    parse_transaction_to_vec(&tx, grpc_us, Some(block_us), filter.as_ref())
                {
                    if micro_buf.push(slot, idx, e, grpc_us, batch_us) {
                        for evt in micro_buf.flush() {
                            self.push_to_queue(queue, evt);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn handle_account(
        &self,
        acc: SubscribeUpdateAccount,
        filter: &Option<EventTypeFilter>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        grpc_us: i64,
        block_us: i64,
    ) {
        let Some(info) = acc.account else { return };
        let data = crate::accounts::AccountData {
            pubkey: read_pubkey_fast(&info.pubkey),
            executable: info.executable,
            lamports: info.lamports,
            owner: read_pubkey_fast(&info.owner),
            rent_epoch: info.rent_epoch,
            data: info.data,
        };
        let meta = EventMetadata {
            signature: Default::default(),
            slot: acc.slot,
            tx_index: 0,
            event_ordinal: 0,
            stream_epoch: 0,
            block_time_us: block_us,
            grpc_recv_us: grpc_us,
        };
        if let Some(e) = crate::accounts::parse_account_unified(&data, meta, filter.as_ref()) {
            self.push_to_queue(queue, e);
        }
    }
}

// ==================== 辅助函数 ====================

/// 获取当前时间戳（微秒）
///
/// 使用高性能时钟，避免系统调用开销
///
/// # 性能优势
/// - 旧实现：使用 libc::clock_gettime，每次调用约 1-2μs
/// - 新实现：使用高性能时钟，每次调用约 10-50ns
/// - 性能提升：20-100 倍
#[inline(always)]
fn get_timestamp_us() -> i64 {
    now_micros()
}

fn build_subscribe_request(
    tx_filters: &[TransactionFilter],
    acc_filters: &[AccountFilter],
    acknowledgement: Option<&str>,
) -> SubscribeRequest {
    let transactions = tx_filters
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                format!("tx_{}", i),
                SubscribeRequestFilterTransactions {
                    vote: Some(false),
                    failed: Some(false),
                    signature: None,
                    account_include: f.account_include.clone(),
                    account_exclude: f.account_exclude.clone(),
                    account_required: f.account_required.clone(),
                },
            )
        })
        .collect();

    let accounts = acc_filters
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                format!("acc_{}", i),
                SubscribeRequestFilterAccounts {
                    account: f.account.clone(),
                    owner: f.owner.clone(),
                    filters: f.filters.clone(),
                    nonempty_txn_signature: None,
                },
            )
        })
        .collect();

    SubscribeRequest {
        slots: acknowledgement
            .map(|label| {
                HashMap::from([(label.to_string(), SubscribeRequestFilterSlots::default())])
            })
            .unwrap_or_default(),
        accounts,
        transactions,
        transactions_status: HashMap::new(),
        blocks: HashMap::new(),
        blocks_meta: HashMap::new(),
        entry: HashMap::new(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: Vec::new(),
        ping: None,
        from_slot: None,
    }
}

fn consume_establishment_filter_change(
    filters_rx: &mut watch::Receiver<SubscriptionFilters>,
) -> Result<bool, String> {
    if filters_rx.has_changed().map_err(|_| "Subscription filter channel closed".to_string())? {
        filters_rx.borrow_and_update();
        return Ok(true);
    }
    Ok(false)
}

fn acknowledgement_slot(update: &SubscribeUpdate, expected: &str) -> Option<u64> {
    if !update.filters.iter().any(|actual| actual == expected) {
        return None;
    }
    match update.update_oneof.as_ref() {
        Some(subscribe_update::UpdateOneof::Slot(slot)) => Some(slot.slot),
        _ => None,
    }
}

async fn run_until_stopped<T>(
    stop_rx: &mut watch::Receiver<()>,
    operation: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = stop_rx.changed() => None,
        result = operation => Some(result),
    }
}

// ==================== 交易解析 ====================

#[inline]
fn parse_transaction_to_vec(
    tx: &SubscribeUpdateTransaction,
    grpc_us: i64,
    block_us: Option<i64>,
    filter: Option<&EventTypeFilter>,
) -> Vec<(u64, DexEvent)> {
    let idx = tx.transaction.as_ref().map(|t| t.index).unwrap_or(0);
    parse_transaction_core(tx, grpc_us, block_us, filter).into_iter().map(|e| (idx, e)).collect()
}

#[inline]
pub(crate) fn parse_transaction_core(
    tx: &SubscribeUpdateTransaction,
    grpc_us: i64,
    block_us: Option<i64>,
    filter: Option<&EventTypeFilter>,
) -> Vec<DexEvent> {
    let Some(info) = &tx.transaction else { return Vec::new() };
    let Some(meta) = &info.meta else { return Vec::new() };

    let sig = extract_signature(&info.signature);
    let slot = tx.slot;
    let idx = info.index;

    // 并行解析 logs 和 instructions
    let (log_events, instr_events) = rayon::join(
        || {
            parse_logs(
                meta,
                &info.transaction,
                &meta.log_messages,
                sig,
                slot,
                idx,
                block_us,
                grpc_us,
                filter,
            )
        },
        || parse_instructions(meta, &info.transaction, sig, slot, idx, block_us, grpc_us, filter),
    );

    let mut result = Vec::with_capacity(log_events.len() + instr_events.len());
    result.extend(log_events);
    result.extend(instr_events);
    result
}

#[inline(always)]
fn extract_signature(bytes: &[u8]) -> solana_sdk::signature::Signature {
    let mut arr = [0u8; 64];
    arr.copy_from_slice(bytes);
    solana_sdk::signature::Signature::from(arr)
}

#[inline]
fn parse_logs(
    meta: &TransactionStatusMeta,
    transaction: &Option<yellowstone_grpc_proto::prelude::Transaction>,
    logs: &[String],
    sig: solana_sdk::signature::Signature,
    slot: u64,
    tx_idx: u64,
    block_us: Option<i64>,
    grpc_us: i64,
    filter: Option<&EventTypeFilter>,
) -> Vec<DexEvent> {
    let needs_pumpfun = filter.map(|f| f.includes_pumpfun()).unwrap_or(true);
    let has_create = needs_pumpfun && crate::logs::optimized_matcher::detect_pumpfun_create(logs);

    let mut outer_idx: i32 = -1;
    let mut inner_idx: i32 = -1;
    let mut invokes: HashMap<&str, Vec<(i32, i32)>> = HashMap::with_capacity(8);
    let mut result = Vec::with_capacity(4);

    for (log_index, log) in logs.iter().enumerate() {
        if let Some((pid, depth)) = crate::logs::optimized_matcher::parse_invoke_info(log) {
            if depth == 1 {
                inner_idx = -1;
                outer_idx += 1;
            } else {
                inner_idx += 1;
            }
            invokes.entry(pid).or_default().push((outer_idx, inner_idx));
        }

        if PROGRAM_DATA_FINDER.find(log.as_bytes()).is_none() {
            continue;
        }

        if let Some(mut e) =
            crate::logs::parse_log(log, sig, slot, tx_idx, block_us, grpc_us, filter, has_create)
        {
            e.set_event_ordinal(log_index as u64);
            crate::core::account_dispatcher::fill_accounts_from_transaction_data(
                &mut e,
                meta,
                transaction,
                &invokes,
            );
            crate::core::common_filler::fill_data(&mut e, meta, transaction, &invokes);
            result.push(e);
        }
    }
    result
}

#[inline]
fn parse_instructions(
    meta: &TransactionStatusMeta,
    transaction: &Option<yellowstone_grpc_proto::prelude::Transaction>,
    sig: solana_sdk::signature::Signature,
    slot: u64,
    tx_idx: u64,
    block_us: Option<i64>,
    grpc_us: i64,
    filter: Option<&EventTypeFilter>,
) -> Vec<DexEvent> {
    // 使用增强的 instruction 解析器
    // 支持：
    // - 主指令解析（8字节 discriminator）
    // - Inner instruction 解析（16字节 discriminator）
    // - 自动事件合并（instruction + inner instruction）
    crate::grpc::instruction_parser::parse_instructions_enhanced(
        meta,
        transaction,
        sig,
        slot,
        tx_idx,
        block_us,
        grpc_us,
        filter,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> YellowstoneGrpc {
        let (filters_tx, _) = watch::channel(SubscriptionFilters::default());
        let (stop_tx, _) = watch::channel(());
        YellowstoneGrpc {
            endpoint: "https://example.invalid".to_string(),
            token: None,
            config: ClientConfig::default(),
            filters_tx,
            stop_tx,
            subscription_active: Arc::new(AtomicBool::new(false)),
            health: Arc::new(SubscriptionHealthState::default()),
            acknowledgement_nonce: Arc::new(AtomicU64::new(0)),
            acknowledgement_waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn transaction_filter(account: &str) -> TransactionFilter {
        TransactionFilter::new().require_account(account)
    }

    fn account_filter(account: &str) -> AccountFilter {
        AccountFilter::new().add_account(account)
    }

    fn buffered_event(slot: u64, tx_index: u64) -> DexEvent {
        DexEvent::PumpFunTrade(crate::core::PumpFunTradeEvent {
            metadata: EventMetadata { slot, tx_index, ..Default::default() },
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn update_subscription_is_atomic_while_disconnected() {
        let grpc = client();

        grpc.update_subscription(
            vec![transaction_filter("mint-a")],
            vec![account_filter("pool-a")],
        )
        .await
        .unwrap();

        let filters = grpc.filters_tx.borrow().clone();
        assert_eq!(filters.transaction.len(), 1);
        assert_eq!(filters.transaction[0].account_required, ["mint-a"]);
        assert_eq!(filters.account.len(), 1);
        assert_eq!(filters.account[0].account, ["pool-a"]);
    }

    #[tokio::test]
    async fn connected_receiver_observes_latest_complete_replacement() {
        let grpc = client();
        let mut receiver = grpc.filters_tx.subscribe();

        grpc.update_subscription(
            vec![transaction_filter("mint-a")],
            vec![account_filter("pool-a")],
        )
        .await
        .unwrap();
        grpc.update_subscription(
            vec![transaction_filter("mint-b")],
            vec![account_filter("pool-b")],
        )
        .await
        .unwrap();

        tokio::time::timeout(Duration::from_millis(100), receiver.changed())
            .await
            .unwrap()
            .unwrap();
        let filters = receiver.borrow_and_update().clone();
        assert_eq!(filters.transaction[0].account_required, ["mint-b"]);
        assert_eq!(filters.account[0].account, ["pool-b"]);
    }

    #[test]
    fn acknowledgement_slot_filter_is_atomic_with_target_filters() {
        let request = build_subscribe_request(
            &[transaction_filter("mint-a")],
            &[account_filter("pool-a")],
            Some("__ack_7"),
        );

        assert!(request.slots.contains_key("__ack_7"));
        assert_eq!(request.transactions["tx_0"].account_required, ["mint-a"]);
        assert_eq!(request.accounts["acc_0"].account, ["pool-a"]);

        let clean = build_subscribe_request(
            &[transaction_filter("mint-a")],
            &[account_filter("pool-a")],
            None,
        );
        assert!(clean.slots.is_empty());
        assert_eq!(clean.transactions, request.transactions);
        assert_eq!(clean.accounts, request.accounts);
    }

    #[test]
    fn only_a_matching_slot_update_acknowledges_filters() {
        let slot = SubscribeUpdate {
            filters: vec!["__ack_7".to_string()],
            update_oneof: Some(subscribe_update::UpdateOneof::Slot(SubscribeUpdateSlot {
                slot: 42,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(acknowledgement_slot(&slot, "__ack_7"), Some(42));
        assert_eq!(acknowledgement_slot(&slot, "__ack_8"), None);

        let pong = SubscribeUpdate {
            filters: vec!["__ack_7".to_string()],
            update_oneof: Some(subscribe_update::UpdateOneof::Pong(SubscribeUpdatePong::default())),
            ..Default::default()
        };
        assert_eq!(acknowledgement_slot(&pong, "__ack_7"), None);
    }

    #[tokio::test]
    async fn acknowledged_update_times_out_and_removes_temporary_filter() {
        let grpc = client();
        let error = grpc
            .update_subscription_acknowledged(
                vec![transaction_filter("mint-a")],
                vec![account_filter("pool-a")],
                Duration::from_millis(1),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert!(grpc.filters_tx.borrow().acknowledgement.is_none());
        assert!(grpc
            .acknowledgement_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
    }

    #[tokio::test]
    async fn dropping_acknowledged_update_removes_only_the_temporary_filter() {
        let grpc = client();
        let mut update = Box::pin(grpc.update_subscription_acknowledged(
            vec![transaction_filter("mint-a")],
            vec![account_filter("pool-a")],
            Duration::from_secs(1),
        ));
        tokio::select! {
            result = &mut update => panic!("update unexpectedly finished: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        assert!(grpc.filters_tx.borrow().acknowledgement.is_some());

        drop(update);

        let filters = grpc.filters_tx.borrow();
        assert!(filters.acknowledgement.is_none());
        assert_eq!(filters.transaction[0].account_required, ["mint-a"]);
        assert_eq!(filters.account[0].account, ["pool-a"]);
        assert!(grpc
            .acknowledgement_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
    }

    #[tokio::test]
    async fn acknowledged_update_returns_activation_proof() {
        let grpc = client();
        let mut update = Box::pin(grpc.update_subscription_acknowledged(
            vec![transaction_filter("mint-a")],
            vec![account_filter("pool-a")],
            Duration::from_secs(1),
        ));
        tokio::select! {
            result = &mut update => panic!("update unexpectedly finished: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        let acknowledgement = grpc.filters_tx.borrow().acknowledgement.clone().unwrap();
        let waiter = grpc
            .acknowledgement_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&acknowledgement)
            .unwrap();
        waiter
            .send(Ok(SubscriptionActivation {
                stream_epoch: 3,
                activated_after_slot: 42,
                acknowledged_at_us: 123_456,
            }))
            .unwrap();

        assert_eq!(
            update.await.unwrap(),
            SubscriptionActivation {
                stream_epoch: 3,
                activated_after_slot: 42,
                acknowledged_at_us: 123_456,
            }
        );
    }

    #[tokio::test]
    async fn a_new_update_fails_an_outstanding_acknowledgement() {
        let grpc = client();
        let waiting = grpc.update_subscription_acknowledged(
            vec![transaction_filter("mint-a")],
            vec![account_filter("pool-a")],
            Duration::from_secs(1),
        );
        let replacing = async {
            tokio::task::yield_now().await;
            grpc.update_subscription(
                vec![transaction_filter("mint-b")],
                vec![account_filter("pool-b")],
            )
            .await
            .unwrap();
        };
        let (result, ()) = tokio::join!(waiting, replacing);

        assert!(result.unwrap_err().to_string().contains("superseded"));
        let filters = grpc.filters_tx.borrow();
        assert!(filters.acknowledgement.is_none());
        assert_eq!(filters.transaction[0].account_required, ["mint-b"]);
    }

    #[tokio::test]
    async fn connection_setup_reads_latest_filters_after_updates() {
        let grpc = client();
        let mut receiver = grpc.filters_tx.subscribe();

        grpc.update_subscription(
            vec![transaction_filter("mint-old")],
            vec![account_filter("pool-old")],
        )
        .await
        .unwrap();
        grpc.update_subscription(
            vec![transaction_filter("mint-latest")],
            vec![account_filter("pool-latest")],
        )
        .await
        .unwrap();

        let filters = receiver.borrow_and_update().clone();
        assert_eq!(filters.transaction[0].account_required, ["mint-latest"]);
        assert_eq!(filters.account[0].account, ["pool-latest"]);
    }

    #[tokio::test]
    async fn establishment_retries_when_filters_change_while_subscribe_is_pending() {
        let grpc = client();
        grpc.update_subscription(
            vec![transaction_filter("mint-old")],
            vec![account_filter("pool-old")],
        )
        .await
        .unwrap();
        let mut receiver = grpc.filters_tx.subscribe();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let subscribe = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            finish_rx.await.unwrap();
        });

        started_rx.await.unwrap();
        grpc.update_subscription(
            vec![transaction_filter("mint-latest")],
            vec![account_filter("pool-latest")],
        )
        .await
        .unwrap();
        finish_tx.send(()).unwrap();
        subscribe.await.unwrap();

        assert!(consume_establishment_filter_change(&mut receiver).unwrap());
        let filters = receiver.borrow().clone();
        assert_eq!(filters.transaction[0].account_required, ["mint-latest"]);
        assert_eq!(filters.account[0].account, ["pool-latest"]);
        assert!(!consume_establishment_filter_change(&mut receiver).unwrap());
    }

    #[tokio::test]
    async fn reconnect_receiver_starts_with_latest_filters() {
        let grpc = client();
        grpc.update_subscription(
            vec![transaction_filter("mint-latest")],
            vec![account_filter("pool-latest")],
        )
        .await
        .unwrap();

        let receiver = grpc.filters_tx.subscribe();
        let filters = receiver.borrow().clone();
        assert_eq!(filters.transaction[0].account_required, ["mint-latest"]);
        assert_eq!(filters.account[0].account, ["pool-latest"]);
    }

    #[tokio::test]
    async fn stop_notifies_current_reconnect_loop_only() {
        let grpc = client();
        let mut current = grpc.stop_tx.subscribe();

        grpc.stop().await;

        tokio::time::timeout(Duration::from_millis(100), current.changed()).await.unwrap().unwrap();
        let mut future = grpc.stop_tx.subscribe();
        assert!(tokio::time::timeout(Duration::from_millis(10), future.changed()).await.is_err());
    }

    #[test]
    fn second_subscription_is_rejected_while_first_is_active() {
        let grpc = client();
        let first = grpc
            .claim_subscription(vec![transaction_filter("mint-a")], vec![account_filter("pool-a")])
            .unwrap();

        assert!(grpc
            .claim_subscription(vec![transaction_filter("mint-b")], vec![account_filter("pool-b")],)
            .is_err());
        let filters = grpc.filters_tx.borrow().clone();
        assert_eq!(filters.transaction[0].account_required, ["mint-a"]);
        assert_eq!(filters.account[0].account, ["pool-a"]);

        drop(first);
    }

    #[tokio::test]
    async fn subscription_can_restart_only_after_stopped_task_exits() {
        let grpc = client();
        let (lease, _filters_rx, mut stop_rx) =
            grpc.prepare_subscription(Vec::new(), Vec::new()).unwrap();
        let (allow_exit_tx, allow_exit_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _lease = lease;
            stop_rx.changed().await.unwrap();
            allow_exit_rx.await.unwrap();
        });

        grpc.stop().await;
        assert!(grpc.claim_subscription(Vec::new(), Vec::new()).is_err());

        allow_exit_tx.send(()).unwrap();
        task.await.unwrap();
        assert!(grpc.claim_subscription(Vec::new(), Vec::new()).is_ok());
    }

    #[tokio::test]
    async fn stop_after_preparing_subscription_is_observed_before_worker_start() {
        let grpc = client();
        let (_lease, _filters_rx, mut stop_rx) =
            grpc.prepare_subscription(Vec::new(), Vec::new()).unwrap();

        grpc.stop().await;

        tokio::time::timeout(Duration::from_millis(100), stop_rx.changed()).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_outbound_operation() {
        let grpc = client();
        let mut stop_rx = grpc.stop_tx.subscribe();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let stopper = grpc.clone();
        let stop_task = tokio::spawn(async move {
            started_rx.await.unwrap();
            stopper.stop().await;
        });
        let operation = async move {
            started_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        };

        assert!(run_until_stopped(&mut stop_rx, operation).await.is_none());
        stop_task.await.unwrap();
    }

    #[tokio::test]
    async fn health_snapshot_tracks_transport_and_queue_state_without_secrets() {
        let mut grpc = client();
        grpc.token = Some("top-secret".to_string());
        assert_eq!(grpc.subscription_health(), SubscriptionHealth::default());

        grpc.record_connected();
        grpc.record_receive(123_456, Some(42));
        grpc.record_reconnect();
        let error =
            grpc.record_disconnected("failed https://example.invalid with token top-secret");
        let queue = ArrayQueue::new(1);
        grpc.push_to_queue(&queue, buffered_event(42, 0));
        grpc.push_to_queue(&queue, buffered_event(42, 1));

        let health = grpc.subscription_health();
        assert!(!health.connected);
        assert_eq!(health.stream_epoch, 1);
        assert_eq!(health.reconnect_count, 1);
        assert_eq!(health.last_receive_timestamp_us, Some(123_456));
        assert_eq!(health.last_receive_slot, Some(42));
        assert_eq!(health.input_queue_drop_count, 1);
        assert_eq!(health.last_error.as_deref(), Some(error.as_str()));
        assert!(!error.contains("example.invalid"));
        assert!(!error.contains("top-secret"));

        grpc.record_receive(123_457, Some(0));
        assert_eq!(grpc.subscription_health().last_receive_slot, Some(0));

        grpc.record_connected();
        grpc.stop().await;
        let stopped = grpc.subscription_health();
        assert!(!stopped.connected);
        assert_eq!(stopped.stream_epoch, 2);
    }

    #[test]
    fn enqueue_stamps_events_with_their_stream_epoch_across_reconnect() {
        let grpc = client();
        let queue = ArrayQueue::new(1);
        grpc.record_connected();
        let mut old_stream_event = buffered_event(42, 0);
        old_stream_event.set_event_ordinal(7);

        grpc.push_to_queue(&queue, old_stream_event);
        let old_stream_event = queue.pop().unwrap();
        assert_eq!(old_stream_event.metadata().stream_epoch, 1);
        assert_eq!(old_stream_event.metadata().event_ordinal, 7);

        grpc.record_disconnected("reconnect");
        grpc.record_reconnect();
        grpc.record_connected();
        let mut new_stream_event = buffered_event(43, 0);
        new_stream_event.set_event_ordinal(9);

        grpc.push_to_queue(&queue, new_stream_event);
        let new_stream_event = queue.pop().unwrap();
        assert_eq!(new_stream_event.metadata().stream_epoch, 2);
        assert_eq!(new_stream_event.metadata().event_ordinal, 9);
    }

    #[test]
    fn stop_cleanup_flushes_every_buffered_mode() {
        let grpc = client();

        for mode in [OrderMode::Ordered, OrderMode::StreamingOrdered] {
            let queue = Arc::new(ArrayQueue::new(2));
            let mut slot_buffer = SlotBuffer::new();
            let mut micro_batch = MicroBatchBuffer::new();
            slot_buffer.push(42, 0, buffered_event(42, 0));

            grpc.flush_pending(mode, &mut slot_buffer, &mut micro_batch, &queue);

            assert_eq!(queue.len(), 1);
        }

        let queue = Arc::new(ArrayQueue::new(2));
        let mut slot_buffer = SlotBuffer::new();
        let mut micro_batch = MicroBatchBuffer::new();
        micro_batch.push(42, 0, buffered_event(42, 0), 100, 1_000);

        grpc.flush_pending(OrderMode::MicroBatch, &mut slot_buffer, &mut micro_batch, &queue);

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn disconnect_cleanup_flushes_microbatch_and_counts_queue_overflow() {
        let grpc = client();
        let queue = Arc::new(ArrayQueue::new(1));
        let mut slot_buffer = SlotBuffer::new();
        let mut micro_batch = MicroBatchBuffer::new();
        micro_batch.push(42, 0, buffered_event(42, 0), 100, 1_000);
        micro_batch.push(42, 1, buffered_event(42, 1), 101, 1_000);

        grpc.flush_pending(OrderMode::MicroBatch, &mut slot_buffer, &mut micro_batch, &queue);

        assert_eq!(queue.len(), 1);
        assert_eq!(grpc.subscription_health().input_queue_drop_count, 1);
    }
}
