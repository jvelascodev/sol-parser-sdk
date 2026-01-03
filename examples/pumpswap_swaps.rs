//! PumpSwap Swaps Event Listener Example
//!
//! Demonstrates how to:
//! - Subscribe to PumpSwap protocol buy and sell events
//! - Print event details to the console
//! - Save the first captured buy and sell events to JSON files

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

    println!("🚀 PumpSwap Swaps Event Listener");
    println!("===============================\n");

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

    // Subscribe to Buy and Sell events
    let event_filter =
        EventTypeFilter::include_only(vec![EventType::PumpSwapBuy, EventType::PumpSwapSell]);

    println!("🎯 Event Filter: PumpSwapBuy, PumpSwapSell");
    println!("🎧 Starting subscription...\n");

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    let mut buy_saved = false;
    let mut sell_saved = false;

    loop {
        if let Some(event) = queue.pop() {
            match event {
                DexEvent::PumpSwapBuy(e) => {
                    println!("┌─────────────────────────────────────────────────────────────");
                    println!("│ 🟢 PumpSwap BUY Captured");
                    println!("├─────────────────────────────────────────────────────────────");
                    println!("│ Signature  : {}", e.metadata.signature);
                    println!("│ Pool       : {}", e.pool);
                    println!("│ Base Mint  : {}", e.base_mint);
                    println!("│ Quote Mint : {}", e.quote_mint);
                    println!("│ Base Out   : {}", e.base_amount_out);
                    println!("│ Quote In   : {}", e.quote_amount_in);
                    println!("│ User       : {}", e.user);
                    println!("└─────────────────────────────────────────────────────────────\n");

                    if !buy_saved {
                        let mut value = serde_json::to_value(&e)?;

                        // Convert specific fields to Base58 strings for better readability
                        if let Some(meta) = value.get_mut("metadata") {
                            if let Some(sig) = meta.get_mut("signature") {
                                *sig = serde_json::Value::String(e.metadata.signature.to_string());
                            }
                        }

                        let pubkey_fields = [
                            "pool",
                            "user",
                            "user_base_token_account",
                            "user_quote_token_account",
                            "protocol_fee_recipient",
                            "protocol_fee_recipient_token_account",
                            "coin_creator",
                            "base_mint",
                            "quote_mint",
                            "pool_base_token_account",
                            "pool_quote_token_account",
                            "coin_creator_vault_ata",
                            "coin_creator_vault_authority",
                            "base_token_program",
                            "quote_token_program",
                        ];

                        for field in pubkey_fields {
                            if let Some(val) = value.get_mut(field) {
                                let bs58_str = match field {
                                    "pool" => e.pool.to_string(),
                                    "user" => e.user.to_string(),
                                    "user_base_token_account" => {
                                        e.user_base_token_account.to_string()
                                    }
                                    "user_quote_token_account" => {
                                        e.user_quote_token_account.to_string()
                                    }
                                    "protocol_fee_recipient" => {
                                        e.protocol_fee_recipient.to_string()
                                    }
                                    "protocol_fee_recipient_token_account" => {
                                        e.protocol_fee_recipient_token_account.to_string()
                                    }
                                    "coin_creator" => e.coin_creator.to_string(),
                                    "base_mint" => e.base_mint.to_string(),
                                    "quote_mint" => e.quote_mint.to_string(),
                                    "pool_base_token_account" => {
                                        e.pool_base_token_account.to_string()
                                    }
                                    "pool_quote_token_account" => {
                                        e.pool_quote_token_account.to_string()
                                    }
                                    "coin_creator_vault_ata" => {
                                        e.coin_creator_vault_ata.to_string()
                                    }
                                    "coin_creator_vault_authority" => {
                                        e.coin_creator_vault_authority.to_string()
                                    }
                                    "base_token_program" => e.base_token_program.to_string(),
                                    "quote_token_program" => e.quote_token_program.to_string(),
                                    _ => unreachable!(),
                                };
                                *val = serde_json::Value::String(bs58_str);
                            }
                        }

                        let json_data = serde_json::to_string_pretty(&value)?;
                        let mut file = File::create("examples/events/pumpswap_buy_event.json")?;
                        file.write_all(json_data.as_bytes())?;
                        println!("💾 Saved the first buy event to 'examples/events/pumpswap_buy_event.json'");
                        buy_saved = true;
                    }
                }
                DexEvent::PumpSwapSell(e) => {
                    println!("┌─────────────────────────────────────────────────────────────");
                    println!("│ 🔴 PumpSwap SELL Captured");
                    println!("├─────────────────────────────────────────────────────────────");
                    println!("│ Signature  : {}", e.metadata.signature);
                    println!("│ Pool       : {}", e.pool);
                    println!("│ Base Mint  : {}", e.base_mint);
                    println!("│ Quote Mint : {}", e.quote_mint);
                    println!("│ Base In    : {}", e.base_amount_in);
                    println!("│ Quote Out  : {}", e.quote_amount_out);
                    println!("│ User       : {}", e.user);
                    println!("└─────────────────────────────────────────────────────────────\n");

                    if !sell_saved {
                        let mut value = serde_json::to_value(&e)?;

                        // Convert specific fields to Base58 strings for better readability
                        if let Some(meta) = value.get_mut("metadata") {
                            if let Some(sig) = meta.get_mut("signature") {
                                *sig = serde_json::Value::String(e.metadata.signature.to_string());
                            }
                        }

                        let pubkey_fields = [
                            "pool",
                            "user",
                            "user_base_token_account",
                            "user_quote_token_account",
                            "protocol_fee_recipient",
                            "protocol_fee_recipient_token_account",
                            "coin_creator",
                            "base_mint",
                            "quote_mint",
                            "pool_base_token_account",
                            "pool_quote_token_account",
                            "coin_creator_vault_ata",
                            "coin_creator_vault_authority",
                            "base_token_program",
                            "quote_token_program",
                        ];

                        for field in pubkey_fields {
                            if let Some(val) = value.get_mut(field) {
                                let bs58_str = match field {
                                    "pool" => e.pool.to_string(),
                                    "user" => e.user.to_string(),
                                    "user_base_token_account" => {
                                        e.user_base_token_account.to_string()
                                    }
                                    "user_quote_token_account" => {
                                        e.user_quote_token_account.to_string()
                                    }
                                    "protocol_fee_recipient" => {
                                        e.protocol_fee_recipient.to_string()
                                    }
                                    "protocol_fee_recipient_token_account" => {
                                        e.protocol_fee_recipient_token_account.to_string()
                                    }
                                    "coin_creator" => e.coin_creator.to_string(),
                                    "base_mint" => e.base_mint.to_string(),
                                    "quote_mint" => e.quote_mint.to_string(),
                                    "pool_base_token_account" => {
                                        e.pool_base_token_account.to_string()
                                    }
                                    "pool_quote_token_account" => {
                                        e.pool_quote_token_account.to_string()
                                    }
                                    "coin_creator_vault_ata" => {
                                        e.coin_creator_vault_ata.to_string()
                                    }
                                    "coin_creator_vault_authority" => {
                                        e.coin_creator_vault_authority.to_string()
                                    }
                                    "base_token_program" => e.base_token_program.to_string(),
                                    "quote_token_program" => e.quote_token_program.to_string(),
                                    _ => unreachable!(),
                                };
                                *val = serde_json::Value::String(bs58_str);
                            }
                        }

                        let json_data = serde_json::to_string_pretty(&value)?;
                        let mut file = File::create("examples/events/pumpswap_sell_event.json")?;
                        file.write_all(json_data.as_bytes())?;
                        println!("💾 Saved the first sell event to 'examples/events/pumpswap_sell_event.json'");
                        sell_saved = true;
                    }
                }
                _ => {}
            }

            if buy_saved && sell_saved {
                println!("🛑 Both events captured and saved. Exiting...");
                break;
            }
        } else {
            std::hint::spin_loop();
            tokio::task::yield_now().await;
        }
    }

    grpc.stop().await;
    Ok(())
}
