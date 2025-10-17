use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, Protocol, TransactionFilter,
    YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("========================================");
    println!("🚀 Dynamic Subscription Example");
    println!("========================================\n");

    run_dynamic_subscription_example().await?;
    Ok(())
}

async fn run_dynamic_subscription_example() -> Result<(), Box<dyn std::error::Error>> {
    // 创建配置
    let mut config: ClientConfig = ClientConfig::default();
    config.enable_metrics = true;
    config.connection_timeout_ms = 10000;
    config.request_timeout_ms = 30000;
    config.enable_tls = true;

    let grpc = YellowstoneGrpc::new_with_config(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
        config,
    )?;

    println!("✅ gRPC client created successfully\n");

    // ==========================================
    // 阶段 1: 初始订阅 - 仅监控 PumpFun
    // ==========================================
    println!("📊 Phase 1: Initial subscription - PumpFun only");
    let initial_protocols = vec![Protocol::PumpFun];

    let transaction_filter = TransactionFilter::for_protocols(&initial_protocols);
    let account_filter = AccountFilter::for_protocols(&initial_protocols);

    let event_filter = EventTypeFilter::include_only(vec![
        EventType::PumpFunTrade,
        EventType::PumpFunCreate,
        EventType::PumpSwapBuy,
        EventType::PumpSwapSell,
    ]);

    println!("🎧 Starting initial subscription...");
    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    println!("✅ Initial subscription active (PumpFun only)\n");

    // 启动事件消费任务
    let queue_clone = queue.clone();
    tokio::spawn(async move {
        let mut event_count = 0u64;
        let mut last_protocol = String::new();

        loop {
            if let Some(event) = queue_clone.pop() {
                event_count += 1;
                let current_protocol = match &event {
                    DexEvent::PumpFunTrade(_) => "PumpFun (Trade)",
                    DexEvent::PumpFunCreate(_) => "PumpFun (Create)",
                    DexEvent::PumpSwapBuy(_) => "PumpSwap Trade",
                    DexEvent::PumpSwapSell(_) => "PumpSwap Trade",
                    _ => "",
                };

                if current_protocol != "" && current_protocol != last_protocol {
                    println!("📦 [Event #{}] Received: {}", event_count, current_protocol);
                    last_protocol = current_protocol.to_string();
                }

                // 每 50 个事件打印一次统计
                if event_count % 50 == 0 {
                    println!("📈 Total events received: {}", event_count);
                }
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        }
    });

    // 等待 15 秒，观察初始订阅
    println!("⏳ Monitoring PumpFun events for 15 seconds...\n");
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

    // ==========================================
    // 阶段 2: 动态更新 - 切换到 Pumpswap
    // ==========================================
    println!("\n========================================");
    println!("📊 Phase 2: Switching to Pumpswap");
    println!("========================================\n");

    let updated_protocols = vec![Protocol::PumpSwap];
    let updated_tx_filter = TransactionFilter::for_protocols(&updated_protocols);
    let updated_acc_filter = AccountFilter::for_protocols(&updated_protocols);

    println!("🔄 Updating subscription dynamically (no reconnection)...");
    grpc.update_subscription(vec![updated_tx_filter], vec![updated_acc_filter]).await?;

    println!("✅ Subscription updated (RaydiumCpmm only)\n");
    println!("⏳ Monitoring RaydiumCpmm for 15 seconds...\n");
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

    // ==========================================
    // 优雅停止
    // ==========================================
    println!("\n========================================");
    println!("🛑 Stopping subscription gracefully...");
    println!("========================================");

    grpc.stop().await;

    println!("✅ Dynamic subscription example completed successfully!");
    println!("\n🎉 Summary:");
    println!("  • Phase 1: PumpFun only (15s)");
    println!("  • Phase 2: RaydiumCpmm only (15s)");
    println!("\n✨ Protocol switched without reconnection!");

    Ok(())
}
