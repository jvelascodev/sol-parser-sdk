use super::types::*;
use crate::instr::read_pubkey_fast;
use crate::logs::timestamp_to_microseconds;
use crate::DexEvent;
use crossbeam_queue::ArrayQueue;
use futures::StreamExt;
use log::error;
use memchr::memmem;
use once_cell::sync::Lazy;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
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
}

impl YellowstoneGrpc {
    pub fn new(
        endpoint: String,
        token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self { endpoint, token, config: ClientConfig::default() })
    }

    pub fn new_with_config(
        endpoint: String,
        token: Option<String>,
        config: ClientConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self { endpoint, token, config })
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
            let _ = self_clone
                .stream_to_queue(
                    transaction_filters,
                    account_filters,
                    event_type_filter,
                    queue_clone,
                )
                .await;
        });

        Ok(queue)
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("🚀 Starting Zero-Copy DEX event subscription...");

        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut builder = GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .x_token(self.token.clone())?
            .max_decoding_message_size(1024 * 1024 * 1024);

        if self.config.connection_timeout_ms > 0 {
            builder = builder.connect_timeout(std::time::Duration::from_millis(
                self.config.connection_timeout_ms,
            ));
        }

        // 添加 TLS 配置
        if self.config.enable_tls {
            let tls_config = ClientTlsConfig::new().with_native_roots();
            builder = builder.tls_config(tls_config)?;
        }

        println!("🔗 Connecting to gRPC endpoint: {}", self.endpoint);
        println!("⏱️  Connection timeout: {}ms", self.config.connection_timeout_ms);

        let mut client = match builder.connect().await {
            Ok(c) => {
                println!("✅ Connection established");
                c
            }
            Err(e) => {
                println!("❌ Connection failed: {:?}", e);
                return Err(e.into());
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
                    filters: vec![],
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
        let (_subscribe_tx, mut stream) = client.subscribe_with_request(Some(request)).await?;
        println!("✅ Subscribed successfully - Zero Copy Mode");
        println!("👂 Listening for events...");

        let mut msg_count = 0u64;
        while let Some(message) = stream.next().await {
            match message {
                Ok(update_msg) => {
                    let block_time = update_msg.created_at.unwrap_or_default();
                    let block_time_us = timestamp_to_microseconds(&block_time);
                    msg_count += 1;
                    // if msg_count % 100 == 0 {
                    //     println!("📨 Received {} messages", msg_count);
                    // }

                    if let Some(update) = update_msg.update_oneof {
                        if let subscribe_update::UpdateOneof::Transaction(transaction_update) =
                            update
                        {
                            let grpc_recv_us = unsafe {
                                let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                                libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
                                (ts.tv_sec as i64) * 1_000_000 + (ts.tv_nsec as i64) / 1_000
                            };
                            Self::parse_transaction(
                                &transaction_update,
                                grpc_recv_us,
                                Some(block_time_us as i64),
                                &queue,
                                event_type_filter.as_ref(),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    println!("❌ Stream error: {:?}", e);
                }
            }
        }

        println!("⚠️  Stream ended");

        Ok(())
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
        let has_create = event_type_filter.map(|f| f.includes_pumpfun()).unwrap_or(true)
            && crate::logs::optimized_matcher::detect_pumpfun_create(logs);

        // 外层指令索引
        let mut outer_index = -1;
        // 内层指令索引
        let mut inner_index = -1;
        // 记录每个程序的调用栈位置 - 只是为了查找【填充账户信息】的指令的位置（如果有更好的其他办法，后续可优化）
        let mut program_invokes: HashMap<String, Vec<(i32, i32)>> = HashMap::new();

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
                // 记录每个程序的调用栈位置 - 只是为了查找【填充账户信息】的指令的位置（如果有更好的其他办法，后续可优化）
                let mut program_invokes: HashMap<String, Vec<(i32, i32)>> = HashMap::new();
                let mut outer_index = -1;
                message.instructions.iter().for_each(|ix| {
                    outer_index += 1;
                    let program_id = get_key(ix.program_id_index as usize)
                        .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                    program_invokes
                        .entry(program_id.to_string())
                        .or_default()
                        .push((outer_index, -1));
                });
                meta.inner_instructions.iter().for_each(|inner| {
                    // 内部指令索引
                    let mut inner_index = -1;
                    inner.instructions.iter().for_each(|ix| {
                        inner_index += 1;
                        let program_id = get_key(ix.program_id_index as usize)
                            .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                        // 解析内部指令 (cpi log)
                        if let Some(mut instr_event) = crate::instr::parse_instruction_unified(
                            &ix.data,
                            &vec![],
                            signature,
                            slot,
                            tx_index,
                            block_time_us,
                            grpc_recv_us,
                            event_type_filter,
                            &program_id,
                        ) {
                            // 填充账户信息
                            crate::core::account_filler::fill_accounts_from_transaction_data(
                                &mut instr_event,
                                meta,
                                transaction,
                                &program_invokes,
                            );
                            let _ = queue.push(instr_event);
                        } else {
                            program_invokes
                                .entry(program_id.to_string())
                                .or_default()
                                .push((inner.index as i32, inner_index));
                        }
                    });
                });
            }
        }
    }
}
