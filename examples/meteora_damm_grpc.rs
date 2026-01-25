//! Meteora DAMM gRPC Streaming Example
//!
//! Demonstrates how to:
//! - Subscribe to Meteora DAMM protocol events via gRPC
//! - Filter specific event types: Swap, Swap2, AddLiquidity, RemoveLiquidity
//! - Display event details with latency metrics
//!
//! Usage:
//! ```bash
//! cargo run --example meteora_damm_grpc --release
//! ```

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::core::now_micros;
use sol_parser_sdk::DexEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("🚀 Meteora DAMM gRPC Streaming Example");
    println!("========================================\n");

    run_example().await?;
    Ok(())
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Create ultra-low latency configuration
    // NOTE: Use Unordered mode for lowest latency (10-20μs)
    //       MicroBatch mode has no periodic flush, events may be delayed until next batch
    let config = ClientConfig {
        enable_metrics: true,
        connection_timeout_ms: 10000,
        request_timeout_ms: 30000,
        enable_tls: true,
        order_mode: OrderMode::Unordered, // Ultra-low latency mode
        ..Default::default()
    };

    println!("📋 Configuration:");
    println!("   Order Mode: {:?} (ultra-low latency)", config.order_mode);
    println!();

    // Get gRPC endpoint from environment or use default
    let grpc_endpoint = std::env::var("GRPC_ENDPOINT")
        .unwrap_or_else(|_| "https://solana-yellowstone-grpc.publicnode.com:443".to_string());

    let grpc = YellowstoneGrpc::new_with_config(
        grpc_endpoint.clone(),
        None,
        config,
    )?;

    println!("✅ gRPC client created (parser pre-warmed)");
    println!("📡 Endpoint: {}", grpc_endpoint);

    // Monitor only Meteora DAMM protocol
    let protocols = vec![Protocol::MeteoraDamm];
    println!("📊 Protocols: {:?}", protocols);

    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);

    // ========== Event Type Filter Examples ==========
    //
    // Example 1: Subscribe to Swap events only (V1)
    // let event_filter = EventTypeFilter::include_only(vec![EventType::MeteoraDammSwap]);
    //
    // Example 2: Subscribe to Swap2 events only (V2)
    // let event_filter = EventTypeFilter::include_only(vec![EventType::MeteoraDammSwap2]);
    //
    // Example 3: Subscribe to all Swap events (V1 + V2)
    // let event_filter = EventTypeFilter::include_only(vec![
    //     EventType::MeteoraDammSwap,
    //     EventType::MeteoraDammSwap2,
    // ]);
    //
    // Example 4: Subscribe to liquidity events only
    // let event_filter = EventTypeFilter::include_only(vec![
    //     EventType::MeteoraDammAddLiquidity,
    //     EventType::MeteoraDammRemoveLiquidity,
    // ]);

    // Default: Subscribe to all Meteora DAMM event types
    let event_filter = EventTypeFilter::include_only(vec![
        EventType::MeteoraDammSwap,
        EventType::MeteoraDammSwap2,
        EventType::MeteoraDammAddLiquidity,
        EventType::MeteoraDammRemoveLiquidity,
    ]);

    println!("🎯 Event Filter: Swap, Swap2, AddLiquidity, RemoveLiquidity");
    println!("🎧 Starting subscription...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    // Statistics
    let mut event_count = 0u64;
    let mut swap_count = 0u64;
    let mut swap2_count = 0u64;
    let mut add_liquidity_count = 0u64;
    let mut remove_liquidity_count = 0u64;
    let mut total_latency_us = 0i64;

    // High-performance event consumer
    tokio::spawn(async move {
        let mut spin_count = 0u32;

        loop {
            if let Some(event) = queue.pop() {
                spin_count = 0;
                event_count += 1;

                // Get current time (microseconds) - use same clock source as events
                let now_us = now_micros();

                match &event {
                    DexEvent::MeteoraDammV1Swap(e) => {
                        swap_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🔄 Meteora DAMM SWAP (V1) #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Pool       : {}", e.pool);
                        println!("│ Direction  : {}", if e.trade_direction == 0 { "A→B" } else { "B→A" });
                        println!("│ Amount In  : {}", e.amount_in);
                        println!("│ Amount Out : {}", e.output_amount);
                        println!("│ LP Fee     : {}", e.lp_fee);
                        println!("│ Protocol   : {}", e.protocol_fee);
                        println!("│ Partner    : {}", e.partner_fee);
                        println!("│ Referral   : {} (has_referral: {})", e.referral_fee, e.has_referral);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Swap={} Swap2={} AddLiq={} RemLiq={}",
                                 swap_count, swap2_count, add_liquidity_count, remove_liquidity_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::MeteoraDammV2Swap(e) => {
                        swap2_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ 🔄 Meteora DAMM SWAP2 (V2) #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Pool       : {}", e.pool);
                        println!("│ Direction  : {}", if e.trade_direction == 0 { "A→B" } else { "B→A" });
                        println!("│ Amount In  : {}", e.amount_in);
                        println!("│ Min Out    : {}", e.minimum_amount_out);
                        println!("│ Actual Out : {}", e.output_amount);
                        println!("│ Actual In  : {}", e.actual_amount_in);
                        println!("│ LP Fee     : {}", e.lp_fee);
                        println!("│ Protocol   : {}", e.protocol_fee);
                        println!("│ Referral   : {} (has_referral: {})", e.referral_fee, e.has_referral);
                        println!("│ Sqrt Price : {}", e.next_sqrt_price);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Swap={} Swap2={} AddLiq={} RemLiq={}",
                                 swap_count, swap2_count, add_liquidity_count, remove_liquidity_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::MeteoraDammAddLiquidity(e) => {
                        add_liquidity_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ ➕ Meteora DAMM ADD LIQUIDITY #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Pool       : {}", e.pool);
                        println!("│ Token A In : {}", e.token_a_amount);
                        println!("│ Token B In : {}", e.token_b_amount);
                        println!("│ LP Minted  : {}", e.lp_mint_amount);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Swap={} Swap2={} AddLiq={} RemLiq={}",
                                 swap_count, swap2_count, add_liquidity_count, remove_liquidity_count);
                        println!("└─────────────────────────────────────────────────────────────\n");
                    }

                    DexEvent::MeteoraDammRemoveLiquidity(e) => {
                        remove_liquidity_count += 1;
                        let latency_us = now_us - e.metadata.grpc_recv_us;
                        total_latency_us += latency_us;

                        println!("┌─────────────────────────────────────────────────────────────");
                        println!("│ ➖ Meteora DAMM REMOVE LIQUIDITY #{}", event_count);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Signature  : {}", e.metadata.signature);
                        println!("│ Slot       : {} | TxIndex: {}", e.metadata.slot, e.metadata.tx_index);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ Pool       : {}", e.pool);
                        println!("│ Token A Out: {}", e.token_a_amount);
                        println!("│ Token B Out: {}", e.token_b_amount);
                        println!("│ LP Burned  : {}", e.lp_unmint_amount);
                        println!("├─────────────────────────────────────────────────────────────");
                        println!("│ 📊 Latency : {} μs", latency_us);
                        println!("│ 📊 Stats   : Swap={} Swap2={} AddLiq={} RemLiq={}",
                                 swap_count, swap2_count, add_liquidity_count, remove_liquidity_count);
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
