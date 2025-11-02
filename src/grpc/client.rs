use super::types::*;
use crate::core::EventMetadata;
use crate::instr::read_pubkey_fast;
use crate::logs::timestamp_to_microseconds;
use crate::DexEvent;
use crossbeam_queue::ArrayQueue;
use futures::{SinkExt, StreamExt};
use log::error;
use memchr::memmem;
use once_cell::sync::Lazy;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::*;

static PROGRAM_DATA_FINDER: Lazy<memmem::Finder> =
    Lazy::new(|| memmem::Finder::new(b"Program data: "));

#[derive(Clone)]
pub struct YellowstoneGrpc {
    endpoint: String,
    token: Option<String>,
    config: ClientConfig,
    /// 控制通道发送器，用于动态更新订阅
    control_tx: Arc<Mutex<Option<mpsc::Sender<SubscribeRequest>>>>,
}

impl YellowstoneGrpc {
    pub fn new(
        endpoint: String,
        token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            endpoint,
            token,
            config: ClientConfig::default(),
            control_tx: Arc::new(Mutex::new(None)),
        })
    }

    pub fn new_with_config(
        endpoint: String,
        token: Option<String>,
        config: ClientConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self { endpoint, token, config, control_tx: Arc::new(Mutex::new(None)) })
    }

    /// 订阅DEX事件（零拷贝无锁队列）
    pub async fn subscribe_dex_events(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
        event_type_filter: Option<EventTypeFilter>,
    ) -> Result<Arc<ArrayQueue<DexEvent>>, Box<dyn std::error::Error>> {
        let queue = Arc::new(ArrayQueue::new(100_000));
        let queue_clone = Arc::clone(&queue);

        let self_clone = self.clone();
        tokio::spawn(async move {
            // 带自动重连的订阅循环
            let mut reconnect_delay_secs = 1u64;
            let max_reconnect_delay_secs = 60u64;

            loop {
                println!("🔄 尝试建立GRPC流连接...");

                match self_clone
                    .stream_to_queue(
                        transaction_filters.clone(),
                        account_filters.clone(),
                        event_type_filter.clone(),
                        queue_clone.clone(),
                    )
                    .await
                {
                    Ok(_) => {
                        // 流正常结束（断开），准备重连
                        println!("⚠️ GRPC流已断开，{}秒后重连...", reconnect_delay_secs);
                        tokio::time::sleep(tokio::time::Duration::from_secs(reconnect_delay_secs))
                            .await;

                        // 重连成功后重置延迟
                        reconnect_delay_secs = 1;
                    }
                    Err(e) => {
                        // 连接失败，指数退避重试
                        println!("❌ GRPC连接失败: {} - {}秒后重试", e, reconnect_delay_secs);
                        tokio::time::sleep(tokio::time::Duration::from_secs(reconnect_delay_secs))
                            .await;

                        // 指数退避，最大60秒
                        reconnect_delay_secs =
                            (reconnect_delay_secs * 2).min(max_reconnect_delay_secs);
                    }
                }
            }
        });

        Ok(queue)
    }

    /// 动态更新订阅过滤器（无需重连）
    pub async fn update_subscription(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 获取控制通道发送器
        let control_sender = {
            let control_guard = self.control_tx.lock().await;
            control_guard.as_ref().ok_or("No active subscription to update")?.clone()
        };

        // 构建新的订阅请求
        let mut transactions: HashMap<String, SubscribeRequestFilterTransactions> = HashMap::new();
        for (i, filter) in transaction_filters.iter().enumerate() {
            transactions.insert(
                format!("transaction_filter_{}", i),
                SubscribeRequestFilterTransactions {
                    vote: Some(false),
                    failed: Some(false),
                    signature: None,
                    account_include: filter.account_include.clone(),
                    account_exclude: filter.account_exclude.clone(),
                    account_required: filter.account_required.clone(),
                },
            );
        }

        let mut accounts: HashMap<String, SubscribeRequestFilterAccounts> = HashMap::new();
        for (i, filter) in account_filters.iter().enumerate() {
            accounts.insert(
                format!("account_filter_{}", i),
                SubscribeRequestFilterAccounts {
                    account: filter.account.clone(),
                    owner: filter.owner.clone(),
                    filters: filter.filters.clone(),
                    nonempty_txn_signature: None,
                },
            );
        }

        let request = SubscribeRequest {
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
        };

        // 发送更新请求
        control_sender.send(request).await.map_err(|e| format!("Failed to send update: {}", e))?;

        Ok(())
    }

    pub async fn stop(&self) {
        println!("🛑 Stopping gRPC subscription...");
    }
    async fn stream_to_queue(
        &self,
        transaction_filters: Vec<TransactionFilter>,
        account_filters: Vec<AccountFilter>,
        event_type_filter: Option<EventTypeFilter>,
        queue: Arc<ArrayQueue<DexEvent>>,
    ) -> Result<(), String> {
        println!("🚀 Starting Zero-Copy DEX event subscription...");

        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut builder = GeyserGrpcClient::build_from_shared(self.endpoint.clone())
            .map_err(|e| e.to_string())?
            .x_token(self.token.clone())
            .map_err(|e| e.to_string())?
            .max_decoding_message_size(1024 * 1024 * 1024);

        if self.config.connection_timeout_ms > 0 {
            builder = builder.connect_timeout(std::time::Duration::from_millis(
                self.config.connection_timeout_ms,
            ));
        }

        // 添加 TLS 配置
        if self.config.enable_tls {
            let tls_config = ClientTlsConfig::new().with_native_roots();
            builder = builder.tls_config(tls_config).map_err(|e| e.to_string())?;
        }

        println!("🔗 Connecting to gRPC endpoint: {}", self.endpoint);
        println!("⏱️  Connection timeout: {}ms", self.config.connection_timeout_ms);

        let mut client = match builder.connect().await {
            Ok(c) => {
                println!("✅ Connection established");
                c
            }
            Err(e) => {
                let err_msg = e.to_string();
                println!("❌ Connection failed: {:?}", err_msg);
                return Err(err_msg);
            }
        };
        println!("✅ Connected to Yellowstone gRPC");

        println!("📝 Building subscription filters...");
        let mut accounts: HashMap<String, SubscribeRequestFilterAccounts> = HashMap::new();
        for (i, filter) in account_filters.iter().enumerate() {
            let key = format!("account_filter_{}", i);
            accounts.insert(
                key,
                SubscribeRequestFilterAccounts {
                    account: filter.account.clone(),
                    owner: filter.owner.clone(),
                    filters: filter.filters.clone(),
                    nonempty_txn_signature: None,
                },
            );
        }

        let mut transactions: HashMap<String, SubscribeRequestFilterTransactions> = HashMap::new();
        for (i, filter) in transaction_filters.iter().enumerate() {
            let key = format!("transaction_filter_{}", i);
            transactions.insert(
                key,
                SubscribeRequestFilterTransactions {
                    vote: Some(false),
                    failed: Some(false),
                    signature: None,
                    account_include: filter.account_include.clone(),
                    account_exclude: filter.account_exclude.clone(),
                    account_required: filter.account_required.clone(),
                },
            );
        }

        let request = SubscribeRequest {
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
        };

        println!("📡 Subscribing to stream...");
        let (subscribe_tx, mut stream) =
            client.subscribe_with_request(Some(request)).await.map_err(|e| e.to_string())?;
        println!("✅ Subscribed successfully - Zero Copy Mode");
        println!("👂 Listening for events...");

        // 创建控制通道
        let (control_tx, mut control_rx) = mpsc::channel::<SubscribeRequest>(100);
        *self.control_tx.lock().await = Some(control_tx);

        // 使用 Arc<Mutex<>> 包装 subscribe_tx 以支持并发发送
        let subscribe_tx = Arc::new(Mutex::new(subscribe_tx));
        let subscribe_tx_clone = Arc::clone(&subscribe_tx);

        let mut msg_count = 0u64;
        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(update_msg)) => {
                            let block_time = update_msg.created_at.unwrap_or_default();
                            let block_time_us = timestamp_to_microseconds(&block_time);
                            msg_count += 1;
                            // if msg_count % 100 == 0 {
                            //     println!("📨 Received {} messages", msg_count);
                            // }

                            if let Some(update) = update_msg.update_oneof {
                                let grpc_recv_us = unsafe {
                                    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                                    libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
                                    (ts.tv_sec as i64) * 1_000_000 + (ts.tv_nsec as i64) / 1_000
                                };
                                match update {
                                    subscribe_update::UpdateOneof::Transaction(transaction_update) => {
                                        Self::parse_transaction(
                                            &transaction_update,
                                            grpc_recv_us,
                                            Some(block_time_us as i64),
                                            &queue,
                                            event_type_filter.as_ref(),
                                        )
                                        .await;
                                    }
                                    subscribe_update::UpdateOneof::Account(account_update) => {
                                        Self::parse_account(
                                            &account_update,
                                            grpc_recv_us,
                                            Some(block_time_us as i64),
                                            &queue,
                                            event_type_filter.as_ref(),
                                        )
                                        .await;
                                    }
                                    subscribe_update::UpdateOneof::Ping(_) => {
                                        // 响应 ping 以保持连接活跃
                                        if let Ok(mut tx) = subscribe_tx_clone.try_lock() {
                                            let pong_request = SubscribeRequest {
                                                ping: Some(SubscribeRequestPing { id: 1 }),
                                                ..Default::default()
                                            };
                                            let _ = tx.send(pong_request).await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!("Stream error: {:?}", e);
                            println!("❌ Stream error: {:?}", e);
                            break;
                        }
                        None => {
                            println!("⚠️  Stream ended");
                            break;
                        }
                    }
                }
                Some(update_request) = control_rx.recv() => {
                    // 接收到动态订阅更新请求
                    println!("🔄 Updating subscription filters dynamically...");
                    if let Err(e) = subscribe_tx.lock().await.send(update_request).await {
                        error!("Failed to send subscription update: {}", e);
                        println!("❌ Failed to send subscription update: {}", e);
                        break;
                    }
                    println!("✅ Subscription filters updated successfully");
                }
            }
        }

        println!("⚠️  Stream ended");

        Ok(())
    }

    /// 解析账户事件
    async fn parse_account(
        account_update: &SubscribeUpdateAccount,
        grpc_recv_us: i64,
        block_time_us: Option<i64>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        event_type_filter: Option<&EventTypeFilter>,
    ) {
        if let Some(account_info) = &account_update.account {
            // 构建账户数据
            let account_data = crate::accounts::AccountData {
                pubkey: read_pubkey_fast(&account_info.pubkey),
                executable: account_info.executable,
                lamports: account_info.lamports,
                owner: read_pubkey_fast(&account_info.owner),
                rent_epoch: account_info.rent_epoch,
                data: account_info.data.clone(),
            };
            // 构建元数据
            let metadata = EventMetadata {
                signature: Default::default(), // Account updates don't have signatures
                slot: account_update.slot,
                tx_index: 0,
                block_time_us: block_time_us.unwrap_or(0),
                grpc_recv_us,
            };
            // 使用新的统一账户解析器
            if let Some(event) =
                crate::accounts::parse_account_unified(&account_data, metadata, event_type_filter)
            {
                let _ = queue.push(event);
            }
        }
    }

    /// 解析交易事件
    async fn parse_transaction(
        transaction_update: &SubscribeUpdateTransaction,
        grpc_recv_us: i64,
        block_time_us: Option<i64>,
        queue: &Arc<ArrayQueue<DexEvent>>,
        event_type_filter: Option<&EventTypeFilter>,
    ) {
        if let Some(transaction_info) = &transaction_update.transaction {
            // 从 transaction_info.index 获取交易索引
            let tx_index = transaction_info.index;
            let transaction = &transaction_info.transaction;
            let mut sig_array = [0u8; 64];
            sig_array.copy_from_slice(&transaction_info.signature);
            let signature = solana_sdk::signature::Signature::from(sig_array);
            if let Some(meta) = &transaction_info.meta {
                let logs = &meta.log_messages;
                // 解析 logs 事件
                // pumpfun \ pumpswap
                Self::parse_logs_events(
                    meta,
                    transaction,
                    logs,
                    signature,
                    transaction_update.slot,
                    tx_index,
                    block_time_us,
                    grpc_recv_us,
                    queue,
                    event_type_filter,
                );
                // 解析指令事件
                // pumpfun/migrate
                // metaora damm v2
                Self::parse_transaction_events(
                    meta,
                    transaction,
                    signature,
                    transaction_update.slot,
                    tx_index,
                    block_time_us,
                    grpc_recv_us,
                    queue,
                    event_type_filter,
                );
            }
        }
    }

    /// 解析日志事件到队列
    #[inline]
    fn parse_logs_events(
        meta: &TransactionStatusMeta,
        transaction: &Option<yellowstone_grpc_proto::prelude::Transaction>,
        logs: &[String],
        signature: solana_sdk::signature::Signature,
        slot: u64,
        tx_index: u64,
        block_time_us: Option<i64>,
        grpc_recv_us: i64,
        queue: &Arc<ArrayQueue<DexEvent>>,
        event_type_filter: Option<&EventTypeFilter>,
    ) {
        // 优化: 先检查 filter，如果不需要 pumpfun，直接跳过昂贵的 detect 操作
        let needs_pumpfun_check = event_type_filter.map(|f| f.includes_pumpfun()).unwrap_or(true);
        let has_create =
            needs_pumpfun_check && crate::logs::optimized_matcher::detect_pumpfun_create(logs);

        // 外层指令索引
        let mut outer_index = -1;
        // 内层指令索引
        let mut inner_index = -1;
        // 记录每个程序的调用栈位置 - 只是为了查找【填充账户信息】的指令的位置（如果有更好的其他办法，后续可优化）
        let mut program_invokes: HashMap<&str, Vec<(i32, i32)>> = HashMap::new();

        for log in logs.iter() {
            if let Some((program_id, depth)) =
                crate::logs::optimized_matcher::parse_invoke_info(log)
            {
                if depth == 1 {
                    // 外层指令
                    inner_index = -1;
                    outer_index += 1;
                } else {
                    // 内层指令
                    inner_index += 1;
                }
                program_invokes.entry(program_id).or_default().push((outer_index, inner_index));
            }

            let log_bytes = log.as_bytes();

            if PROGRAM_DATA_FINDER.find(log_bytes).is_none() {
                continue;
            }

            if let Some(mut log_event) = crate::logs::parse_log(
                log,
                signature,
                slot,
                tx_index,
                block_time_us,
                grpc_recv_us,
                event_type_filter,
                has_create,
            ) {
                // 填充账户信息
                crate::core::account_filler::fill_accounts_from_transaction_data(
                    &mut log_event,
                    meta,
                    transaction,
                    &program_invokes,
                );
                // 填充其他信息
                crate::core::common_filler::fill_data(
                    &mut log_event,
                    meta,
                    transaction,
                    &program_invokes,
                );
                let _ = queue.push(log_event);
            }
        }
    }

    fn parse_transaction_events(
        meta: &TransactionStatusMeta,
        transaction: &Option<yellowstone_grpc_proto::prelude::Transaction>,
        signature: solana_sdk::signature::Signature,
        slot: u64,
        tx_index: u64,
        block_time_us: Option<i64>,
        grpc_recv_us: i64,
        queue: &Arc<ArrayQueue<DexEvent>>,
        event_type_filter: Option<&EventTypeFilter>,
    ) {
        if let Some(_transaction) = transaction {
            if let Some(message) = &_transaction.message {
                // 索引器
                let get_key = |index: usize| -> Option<&Vec<u8>> {
                    let account_keys_len = message.account_keys.len();
                    let writable_len = meta.loaded_writable_addresses.len();

                    if index < account_keys_len {
                        message.account_keys.get(index)
                    } else if index < account_keys_len + writable_len {
                        meta.loaded_writable_addresses.get(index - account_keys_len)
                    } else {
                        meta.loaded_readonly_addresses.get(index - account_keys_len - writable_len)
                    }
                };
                // 静态空切片，避免重复分配
                static EMPTY_ACCOUNTS: &[Pubkey] = &[];

                // 记录每个程序的调用栈位置 - 只是为了查找【填充账户信息】的指令的位置（如果有更好的其他办法，后续可优化）
                let mut program_invokes: HashMap<Pubkey, Vec<(i32, i32)>> = HashMap::new();
                let mut outer_index = -1;
                message.instructions.iter().for_each(|ix| {
                    outer_index += 1;
                    let program_id = get_key(ix.program_id_index as usize)
                        .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                    program_invokes.entry(program_id).or_default().push((outer_index, -1));
                });
                meta.inner_instructions.iter().for_each(|inner| {
                    let mut inner_index = -1;
                    inner.instructions.iter().for_each(|ix| {
                        inner_index += 1;
                        let program_id = get_key(ix.program_id_index as usize)
                            .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                        // 解析内部指令 (cpi log)
                        if let Some(mut instr_event) = crate::instr::parse_instruction_unified(
                            &ix.data,
                            EMPTY_ACCOUNTS,
                            signature,
                            slot,
                            tx_index,
                            block_time_us,
                            grpc_recv_us,
                            event_type_filter,
                            &program_id,
                        ) {
                            crate::core::account_filler::fill_accounts_with_owned_keys(
                                &mut instr_event,
                                meta,
                                transaction,
                                &program_invokes,
                            );
                            let _ = queue.push(instr_event);
                        } else {
                            program_invokes
                                .entry(program_id)
                                .or_default()
                                .push((inner.index as i32, inner_index));
                        }
                    });
                });
            }
        }
    }
}
