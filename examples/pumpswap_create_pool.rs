//! PumpSwap Create Pool Event Listener Example
//!
//! Demonstrates how to:
//! - Subscribe to PumpSwap protocol pool creation events
//! - Print event details to the console
//! - Save the first captured event to a JSON file

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

    println!("🚀 PumpSwap Create Pool Event Listener");
    println!("======================================\n");

    run_example().await?;
    Ok(())
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration
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

    // Monitor only PumpSwap protocol
    let protocols = vec![Protocol::PumpSwap];
    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);

    // Subscribe ONLY to CreatePool events
    let event_filter = EventTypeFilter::include_only(vec![EventType::PumpSwapCreatePool]);

    println!("🎯 Event Filter: PumpSwapCreatePool");
    println!("🎧 Starting subscription...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    let mut saved = false;

    loop {
        if let Some(event) = queue.pop() {
            if let DexEvent::PumpSwapCreatePool(e) = event {
                println!("┌─────────────────────────────────────────────────────────────");
                println!("│ 🆕 PumpSwap CREATE POOL Captured");
                println!("├─────────────────────────────────────────────────────────────");
                println!("│ Signature  : {}", e.metadata.signature);
                println!("│ Pool       : {}", e.pool);
                println!("│ Creator    : {}", e.creator);
                println!("│ Base Mint  : {}", e.base_mint);
                println!("│ Quote Mint : {}", e.quote_mint);
                println!("│ Initial LP : {}", e.initial_liquidity);
                println!("└─────────────────────────────────────────────────────────────\n");

                if !saved {
                    let mut value = serde_json::to_value(&e)?;

                    // Convert Pubkey/Signature fields to String for JSON readability
                    if let Some(meta) = value.get_mut("metadata") {
                        if let Some(sig) = meta.get_mut("signature") {
                            *sig = serde_json::Value::String(e.metadata.signature.to_string());
                        }
                    }

                    let pubkey_fields = [
                        "creator",
                        "base_mint",
                        "quote_mint",
                        "pool",
                        "lp_mint",
                        "user_base_token_account",
                        "user_quote_token_account",
                        "coin_creator",
                    ];

                    for field in pubkey_fields {
                        if let Some(val) = value.get_mut(field) {
                            let bs58_str = match field {
                                "creator" => e.creator.to_string(),
                                "base_mint" => e.base_mint.to_string(),
                                "quote_mint" => e.quote_mint.to_string(),
                                "pool" => e.pool.to_string(),
                                "lp_mint" => e.lp_mint.to_string(),
                                "user_base_token_account" => e.user_base_token_account.to_string(),
                                "user_quote_token_account" => {
                                    e.user_quote_token_account.to_string()
                                }
                                "coin_creator" => e.coin_creator.to_string(),
                                _ => unreachable!(),
                            };
                            *val = serde_json::Value::String(bs58_str);
                        }
                    }

                    let json_data = serde_json::to_string_pretty(&value)?;
                    let mut file = File::create("examples/events/pumpswap_create_pool_event.json")?;
                    file.write_all(json_data.as_bytes())?;
                    println!("💾 Saved the first event to 'examples/events/pumpswap_create_pool_event.json'");
                    saved = true;

                    println!("🛑 Task complete. Exiting...");
                    break;
                }
            }
        } else {
            std::hint::spin_loop();
            tokio::task::yield_now().await;
        }
    }

    grpc.stop().await;
    Ok(())
}
