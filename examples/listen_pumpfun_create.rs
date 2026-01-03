//! PumpFun Creation Event Listener Example
//!
//! Demonstrates how to:
//! - Subscribe to PumpFun protocol creation events
//! - Print event details to the console
//! - Save the first captured event to a JSON file without stripping any data

#![allow(warnings)]
use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;
use std::fs::File;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    println!("🚀 PumpFun Creation Event Listener");
    println!("==================================\n");

    run_example().await?;
    Ok(())
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Create ultra-low latency configuration
    let config = ClientConfig {
        enable_metrics: true,
        connection_timeout_ms: 10000,
        request_timeout_ms: 30000,
        enable_tls: true,
        order_mode: OrderMode::Unordered,
        ..Default::default()
    };

    let grpc = YellowstoneGrpc::new_with_config(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
        config,
    )?;

    println!("✅ gRPC client created");

    // Monitor only PumpFun protocol
    let protocols = vec![Protocol::PumpFun];
    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);

    // Subscribe ONLY to Create events
    let event_filter = EventTypeFilter::include_only(vec![EventType::PumpFunCreate]);

    println!("🎯 Event Filter: PumpFunCreate");
    println!("🎧 Starting subscription...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    let mut saved = false;

    loop {
        if let Some(event) = queue.pop() {
            if let DexEvent::PumpFunCreate(e) = event {
                println!("┌─────────────────────────────────────────────────────────────");
                println!("│ 🆕 PumpFun CREATE Captured");
                println!("├─────────────────────────────────────────────────────────────");
                println!("│ Signature  : {}", e.metadata.signature);
                println!("│ Mint       : {}", e.mint);
                println!("│ Name       : {}", e.name);
                println!("│ Symbol     : {}", e.symbol);
                println!("│ Creator    : {}", e.creator);
                println!("└─────────────────────────────────────────────────────────────\n");

                if !saved {
                    let mut value = serde_json::to_value(&e)?;

                    // Convert specific fields to Base58 strings for better readability
                    if let Some(meta) = value.get_mut("metadata") {
                        if let Some(sig) = meta.get_mut("signature") {
                            *sig = serde_json::Value::String(e.metadata.signature.to_string());
                        }
                    }

                    let fields = ["mint", "bonding_curve", "user", "creator", "token_program"];
                    for field in fields {
                        if let Some(val) = value.get_mut(field) {
                            let bs58_str = match field {
                                "mint" => e.mint.to_string(),
                                "bonding_curve" => e.bonding_curve.to_string(),
                                "user" => e.user.to_string(),
                                "creator" => e.creator.to_string(),
                                "token_program" => e.token_program.to_string(),
                                _ => unreachable!(),
                            };
                            *val = serde_json::Value::String(bs58_str);
                        }
                    }

                    let json_data = serde_json::to_string_pretty(&value)?;
                    let mut file = File::create("examples/events/pumpfun_create_event.json")?;
                    file.write_all(json_data.as_bytes())?;
                    println!("💾 Saved the first event to 'examples/events/pumpfun_create_event.json' (Base58 encoded specific fields)");
                    saved = true;

                    println!("🛑 Task complete. Exiting...");
                    break;
                }
            }
        } else {
            // Spin or yield if no events
            std::hint::spin_loop();
            tokio::task::yield_now().await;
        }
    }

    grpc.stop().await;
    Ok(())
}
