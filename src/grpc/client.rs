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
use std::sync::Arc;
use tokio::sync::watch;
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
}

// ==================== YellowstoneGrpc 客户端 ====================

#[derive(Clone)]
pub struct YellowstoneGrpc {
    endpoint: String,
    token: Option<String>,
    config: ClientConfig,
    filters_tx: watch::Sender<SubscriptionFilters>,
    stop_tx: watch::Sender<()>,
}

impl YellowstoneGrpc {
    pub fn new(
        endpoint: String,
        token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        crate::warmup::warmup_parser();
        let (filters_tx, _) = watch::channel(SubscriptionFilters::default());
        let (stop_tx, _) = watch::channel(());
        Ok(Self { endpoint, token, config: ClientConfig::default(), filters_tx, stop_tx })
    }

    pub fn new_with_config(
        endpoint: String,
        token: Option<String>,
        config: ClientConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        crate::warmup::warmup_parser();
        let (filters_tx, _) = watch::channel(SubscriptionFilters::default());
        let (stop_tx, _) = watch::channel(());
        Ok(Self { endpoint, token, config, filters_tx, stop_tx })
    }

    /// 订阅 DEX 事件（自动重连）
    pub async fn subscribe_dex_events(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
        event_type_filter: Option<EventTypeFilter>,
    ) -> Result<Arc<ArrayQueue<DexEvent>>, Box<dyn std::error::Error>> {
        self.replace_subscription_filters(transaction_filters, account_filters);
        let queue = Arc::new(ArrayQueue::new(100_000));
        let queue_clone = Arc::clone(&queue);
        let self_clone = self.clone();
        let mut filters_rx = self.filters_tx.subscribe();
        let mut stop_rx = self.stop_tx.subscribe();

        tokio::spawn(async move {
            let mut delay = 1u64;
            loop {
                let filters = filters_rx.borrow_and_update().clone();
                let result = tokio::select! {
                    _ = stop_rx.changed() => break,
                    result = self_clone.stream_events(
                        &filters,
                        &event_type_filter,
                        &queue_clone,
                        &mut filters_rx,
                    ) => result,
                };

                match result {
                    Ok(_) => delay = 1,
                    Err(e) => error!("gRPC stream failed; retrying in {}s: {}", delay, e),
                }

                tokio::select! {
                    _ = stop_rx.changed() => break,
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
        self.replace_subscription_filters(transaction_filters, account_filters);
        Ok(())
    }

    pub async fn stop(&self) {
        self.stop_tx.send_replace(());
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
        });
    }

    async fn stream_events(
        &self,
        filters: &SubscriptionFilters,
        event_filter: &Option<EventTypeFilter>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        filters_rx: &mut watch::Receiver<SubscriptionFilters>,
    ) -> Result<(), String> {
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

        let mut client = builder.connect().await.map_err(|e| e.to_string())?;
        let request = build_subscribe_request(&filters.transaction, &filters.account);

        let (mut subscribe_tx, mut stream) =
            client.subscribe_with_request(Some(request)).await.map_err(|e| e.to_string())?;

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
                // Periodic Ping
                _ = tokio::time::sleep_until((next_ping).into()), if Instant::now() >= next_ping => {
                    next_ping = Instant::now() + ping_interval;
                    let ping_request = SubscribeRequest {
                        ping: Some(SubscribeRequestPing { id: 1 }),
                        ..Default::default()
                    };
                    if let Err(e) = subscribe_tx.send(ping_request).await {
                        error!("Failed to send ping: {}", e);
                    }
                }

                msg = stream.next() => {
                    match msg {
                        Some(Ok(update)) => {
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
                            error!("Stream error: {:?}", e);
                            self.flush_on_disconnect(order_mode, &mut slot_buffer, queue);
                            return Err(e.to_string());
                        }
                        None => {
                            self.flush_on_disconnect(order_mode, &mut slot_buffer, queue);
                            return Ok(());
                        }
                    }
                }
                changed = filters_rx.changed() => {
                    changed.map_err(|_| "Subscription filter channel closed".to_string())?;
                    let filters = filters_rx.borrow_and_update().clone();
                    subscribe_tx
                        .send(build_subscribe_request(&filters.transaction, &filters.account))
                        .await
                        .map_err(|e| e.to_string())?;
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
                        let _ = queue.push(e);
                    }
                }
            }
            OrderMode::StreamingOrdered => {
                if slot_buf.should_timeout(timeout_ms) {
                    for e in slot_buf.flush_streaming_timeout() {
                        let _ = queue.push(e);
                    }
                }
            }
            OrderMode::MicroBatch => {
                // Periodic flush for MicroBatch mode
                let now_us = get_timestamp_us();
                if micro_buf.should_flush(now_us, batch_us) {
                    for e in micro_buf.flush() {
                        let _ = queue.push(e);
                    }
                }
            }
            OrderMode::Unordered => {}
        }
    }

    fn flush_on_disconnect(
        &self,
        mode: OrderMode,
        buffer: &mut SlotBuffer,
        queue: &Arc<ArrayQueue<DexEvent>>,
    ) {
        if matches!(mode, OrderMode::Ordered | OrderMode::StreamingOrdered) {
            let events = match mode {
                OrderMode::StreamingOrdered => buffer.flush_streaming_timeout(),
                _ => buffer.flush_all(),
            };
            for e in events {
                let _ = queue.push(e);
            }
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
                Self::handle_account(acc, filter, queue, grpc_recv_us, block_time_us);
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
                    let _ = queue.push(e);
                }
            }
            OrderMode::Ordered => {
                if slot > *last_slot && *last_slot > 0 {
                    for e in slot_buf.flush_before(slot) {
                        let _ = queue.push(e);
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
                        let _ = queue.push(evt);
                    }
                }
            }
            OrderMode::MicroBatch => {
                for (idx, e) in
                    parse_transaction_to_vec(&tx, grpc_us, Some(block_us), filter.as_ref())
                {
                    if micro_buf.push(slot, idx, e, grpc_us, batch_us) {
                        for evt in micro_buf.flush() {
                            let _ = queue.push(evt);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn handle_account(
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
            block_time_us: block_us,
            grpc_recv_us: grpc_us,
        };
        if let Some(e) = crate::accounts::parse_account_unified(&data, meta, filter.as_ref()) {
            let _ = queue.push(e);
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
        slots: HashMap::new(),
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
fn parse_transaction_core(
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

    for log in logs {
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
        }
    }

    fn transaction_filter(account: &str) -> TransactionFilter {
        TransactionFilter::new().require_account(account)
    }

    fn account_filter(account: &str) -> AccountFilter {
        AccountFilter::new().add_account(account)
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
}
