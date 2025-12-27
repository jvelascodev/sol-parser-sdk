//! PumpFun Trade Event Filter Example (Ordered Mode)
//!
//! Demonstrates how to:
//! - Subscribe to PumpFun protocol events with StreamingOrdered mode
//! - Filter specific trade types: Buy, Sell, BuyExactSolIn
//! - Events are ordered by (slot, tx_index) for consistency

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("🚀 PumpFun Trade Event Filter Example (Ordered Mode)");
    println!("=====================================================\n");

    run_example().await?;
    Ok(())
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Create ordered configuration with MicroBatch mode
    // Events are batched in time windows and sorted by (slot, tx_index)
    let config = ClientConfig {
        enable_metrics: true,
        connection_timeout_ms: 10000,
        request_timeout_ms: 30000,
        enable_tls: true,
        order_mode: OrderMode::MicroBatch,  // Micro-batch ordering
        micro_batch_us: 1000,               // 1ms batch window
        order_timeout_ms: 50,               // Periodic flush check interval
        ..Default::default()
    };

    println!("📋 Configuration:");
    println!("   Order Mode: {:?}", config.order_mode);
    println!("   Batch Window: {}μs", config.micro_batch_us);
    println!("   Events sorted by (slot, tx_index) within batch");
    println!();

    let grpc = YellowstoneGrpc::new_with_config(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
        config,
    )?;

    println!("✅ gRPC client created (parser pre-warmed)");

    // Monitor only PumpFun protocol
    let protocols = vec![Protocol::PumpFun];
    println!("📊 Protocols: {:?}", protocols);

    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);

    // Subscribe to all PumpFun trade types
    let event_filter = EventTypeFilter::include_only(vec![
        EventType::PumpFunBuy,
        EventType::PumpFunSell,
        EventType::PumpFunBuyExactSolIn,
        EventType::PumpFunCreate,
    ]);

    println!("🎯 Event Filter: Buy, Sell, BuyExactSolIn, Create");
    println!("🎧 Starting subscription (ordered mode)...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    // Statistics
    let mut event_count = 0u64;
    let mut buy_count = 0u64;
    let mut sell_count = 0u64;
    let mut buy_exact_count = 0u64;
    let mut create_count = 0u64;
    let mut last_slot = 0u64;
    let mut last_tx_index = 0u64;

    // High-performance event consumer
    tokio::spawn(async move {
        let mut spin_count = 0u32;

        loop {
            if let Some(event) = queue.pop() {
                spin_count = 0;
                event_count += 1;

                // Get current time (microseconds)
                let now_us = unsafe {
                    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                    libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
                    (ts.tv_sec as i64) * 1_000_000 + (ts.tv_nsec as i64) / 1_000
                };

                match &event {
                    DexEvent::PumpFunBuy(e) => {
                        buy_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        let order_ok = check_order(e.metadata.slot, e.metadata.tx_index, &mut last_slot, &mut last_tx_index);

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🟢 PumpFun BUY #{} {}", event_count, if order_ok { "✓" } else { "⚠️ OUT OF ORDER" });
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Mint       : {}", e.mint);
                        println!("│ SOL Amount : {} lamports", e.sol_amount);
                        println!("│ Token Amt  : {}", e.token_amount);
                        println!("│ User       : {}", e.user);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Buy={} Sell={} BuyExact={} Create={}", buy_count, sell_count, buy_exact_count, create_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::PumpFunSell(e) => {
                        sell_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        let order_ok = check_order(e.metadata.slot, e.metadata.tx_index, &mut last_slot, &mut last_tx_index);

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🔴 PumpFun SELL #{} {}", event_count, if order_ok { "✓" } else { "⚠️ OUT OF ORDER" });
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Mint       : {}", e.mint);
                        println!("│ SOL Amount : {} lamports", e.sol_amount);
                        println!("│ Token Amt  : {}", e.token_amount);
                        println!("│ User       : {}", e.user);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Buy={} Sell={} BuyExact={} Create={}", buy_count, sell_count, buy_exact_count, create_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::PumpFunBuyExactSolIn(e) => {
                        buy_exact_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        let order_ok = check_order(e.metadata.slot, e.metadata.tx_index, &mut last_slot, &mut last_tx_index);

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🟡 PumpFun BUY_EXACT_SOL_IN #{} {}", event_count, if order_ok { "✓" } else { "⚠️ OUT OF ORDER" });
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Mint       : {}", e.mint);
                        println!("│ SOL Amount : {} lamports", e.sol_amount);
                        println!("│ Token Amt  : {}", e.token_amount);
                        println!("│ User       : {}", e.user);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Buy={} Sell={} BuyExact={} Create={}", buy_count, sell_count, buy_exact_count, create_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::PumpFunTrade(e) => {
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        let order_ok = check_order(e.metadata.slot, e.metadata.tx_index, &mut last_slot, &mut last_tx_index);

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ ⚪ PumpFun TRADE #{} {}", event_count, if order_ok { "✓" } else { "⚠️ OUT OF ORDER" });
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ ix_name    : {} (is_buy={})", e.ix_name, e.is_buy);
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::PumpFunCreate(e) => {
                        create_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        let order_ok = check_order(e.metadata.slot, e.metadata.tx_index, &mut last_slot, &mut last_tx_index);

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🆕 PumpFun CREATE #{} {}", event_count, if order_ok { "✓" } else { "⚠️ OUT OF ORDER" });
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Name       : {}", e.name);
                        println!("│ Symbol     : {}", e.symbol);
                        println!("│ Mint       : {}", e.mint);
                        println!("│ Creator    : {}", e.creator);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Creates : {}", create_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
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

    // Auto-stop timer
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

/// Check if events are in order (slot, tx_index)
#[inline]
fn check_order(slot: u64, tx_index: u64, last_slot: &mut u64, last_tx_index: &mut u64) -> bool {
    let in_order = if slot > *last_slot {
        true
    } else if slot == *last_slot {
        tx_index >= *last_tx_index
    } else {
        false
    };

    *last_slot = slot;
    *last_tx_index = tx_index;
    in_order
}
