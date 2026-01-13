//! PumpSwap 事件订阅示例
//!
//! 演示如何：
//! - 订阅 PumpSwap 协议事件
//! - 使用微批次模式（超低延迟 + 顺序保证）
//! - 打印事件详情和解析延迟

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::core::now_micros;
use sol_parser_sdk::DexEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("🚀 PumpSwap MicroBatch Ordered Event Subscription Example");
    println!("============================================================\n");

    run_example().await?;
    Ok(())
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // 创建微批次模式配置
    let config = ClientConfig {
        enable_metrics: true,
        connection_timeout_ms: 10000,
        request_timeout_ms: 30000,
        enable_tls: true,
        // 微批次模式：极短时间窗口内收集事件，窗口结束后排序释放
        order_mode: OrderMode::MicroBatch,
        // 微批次窗口大小（微秒）
        micro_batch_us: 100, // 100μs 窗口
        order_timeout_ms: 50,
        ..Default::default()
    };

    println!("📋 Configuration:");
    println!("   Order Mode: {:?} (超低延迟 + 顺序保证)", config.order_mode);
    println!("   MicroBatch Window: {}μs", config.micro_batch_us);
    println!("   算法: 极短时间窗口内收集事件，窗口结束后排序释放");
    println!();

    let grpc = YellowstoneGrpc::new_with_config(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
        config,
    )?;

    println!("✅ gRPC client created (parser pre-warmed)");

    // 只监控 PumpSwap 协议
    let protocols = vec![Protocol::PumpSwap];
    println!("📊 Protocols: {:?}", protocols);

    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);

    // 只订阅 PumpSwap 交易事件
    let event_filter = EventTypeFilter::include_only(vec![
        EventType::PumpSwapBuy,
        EventType::PumpSwapSell,
        EventType::PumpSwapCreatePool,
    ]);

    println!("🎧 Starting ordered subscription...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    // 统计信息
    let mut event_count = 0u64;
    let mut total_latency_us = 0i64;
    let mut last_slot = 0u64;
    let mut last_tx_index = 0u64;

    // 高性能消费事件
    tokio::spawn(async move {
        let mut spin_count = 0u32;

        loop {
            if let Some(event) = queue.pop() {
                spin_count = 0;
                event_count += 1;

                // 获取当前时间（微秒）- 使用与事件相同的时钟源
                let now_us = now_micros();

                match &event {
                    DexEvent::PumpSwapBuy(e) => {
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        // 检查顺序性
                        let order_ok = if e.metadata.slot > last_slot {
                            true
                        } else if e.metadata.slot == last_slot {
                            e.metadata.tx_index >= last_tx_index
                        } else {
                            false
                        };

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🟢 PumpSwap BUY #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("│ Order Check: {} (prev: slot={}, tx={})", 
                            if order_ok { "✓ OK" } else { "✗ OUT OF ORDER" },
                            last_slot, last_tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Base Token : {:?}", e.base_mint);
                        println!("│ Quote Token: {:?}", e.quote_mint);
                        println!("│ Base Out   : {}", e.base_amount_out);
                        println!("│ Quote In   : {}", e.quote_amount_in);
                        println!("│ User       : {:?}", e.user);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Avg     : {} μs", total_latency_us / event_count as i64);
                        println!("└─────────────────────────────────────────────────────────────\n");

                        last_slot = e.metadata.slot;
                        last_tx_index = e.metadata.tx_index;
                    }

                    DexEvent::PumpSwapSell(e) => {
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        let order_ok = if e.metadata.slot > last_slot {
                            true
                        } else if e.metadata.slot == last_slot {
                            e.metadata.tx_index >= last_tx_index
                        } else {
                            false
                        };

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🔴 PumpSwap SELL #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("│ Order Check: {} (prev: slot={}, tx={})", 
                            if order_ok { "✓ OK" } else { "✗ OUT OF ORDER" },
                            last_slot, last_tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Base Token : {:?}", e.base_mint);
                        println!("│ Quote Token: {:?}", e.quote_mint);
                        println!("│ Base In    : {}", e.base_amount_in);
                        println!("│ Quote Out  : {}", e.quote_amount_out);
                        println!("│ User       : {:?}", e.user);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Avg     : {} μs", total_latency_us / event_count as i64);
                        println!("└─────────────────────────────────────────────────────────────\n");

                        last_slot = e.metadata.slot;
                        last_tx_index = e.metadata.tx_index;
                    }

                    DexEvent::PumpSwapCreatePool(e) => {
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🆕 PumpSwap CREATE POOL #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Pool       : {:?}", e.pool);
                        println!("│ Base Mint  : {:?}", e.base_mint);
                        println!("│ Quote Mint : {:?}", e.quote_mint);
                        println!("│ Creator    : {:?}", e.creator);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("└─────────────────────────────────────────────────────────────\n");

                        last_slot = e.metadata.slot;
                        last_tx_index = e.metadata.tx_index;
                    }

                    _ => {}
                }
            } else {
                spin_count += 1;
                if spin_count < 1000 {
                    std::hint::spin_loop();
                } else {
                    tokio::task::yield_now().await;
                    spin_count = 0;
                }
            }
        }
    });

    // 自动停止（用于测试）
    let grpc_clone = grpc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        println!("⏰ Auto-stopping after 10 minutes...");
        grpc_clone.stop().await;
    });

    println!("🛑 Press Ctrl+C to stop...\n");
    tokio::signal::ctrl_c().await?;
    println!("\n👋 Shutting down gracefully...");

    Ok(())
}
