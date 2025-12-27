//! Parse a specific PumpSwap transaction from RPC
//!
//! This example fetches a Jupiter Aggregator v6 PumpSwap transaction from RPC
//! and parses it using sol-parser-sdk's RPC parsing support.
//!
//! Transaction: 3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS
//!
//! Usage:
//! ```bash
//! cargo run --example parse_pumpswap_tx --release
//! ```

use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use sol_parser_sdk::parse_transaction_from_rpc;
use std::str::FromStr;

fn main() {
    // 交易签名
    let tx_sig = "3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS";

    println!("=== PumpSwap Transaction Parser ===\n");
    println!("Transaction Signature: {}\n", tx_sig);

    // 连接到 Solana RPC
    let rpc_url = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "http://64.130.37.195:8899".to_string());

    println!("Connecting to: {}", rpc_url);
    let client = RpcClient::new(rpc_url);

    // 解析签名
    let signature = Signature::from_str(tx_sig)
        .expect("Failed to parse signature");

    // 使用 sol-parser-sdk 直接解析交易
    println!("\n=== Parsing with sol-parser-sdk ===");
    println!("Fetching and parsing transaction...\n");

    let events = match parse_transaction_from_rpc(&client, &signature, None) {
        Ok(events) => events,
        Err(e) => {
            eprintln!("✗ Failed to parse transaction: {}", e);
            eprintln!("\nNote: You might need to use a different RPC endpoint.");
            eprintln!("Set SOLANA_RPC_URL environment variable to use a custom endpoint.");
            eprintln!("Example: export SOLANA_RPC_URL=https://your-rpc-endpoint.com");
            std::process::exit(1);
        }
    };

    println!("✓ Parsing completed!");
    println!("  Found {} DEX events\n", events.len());

    // 显示解析结果
    if events.is_empty() {
        println!("⚠ No DEX events found in this transaction.");
        println!("  This could mean:");
        println!("  - The transaction doesn't contain DEX operations");
        println!("  - The DEX protocol is not yet supported");
        println!("  - The transaction only contains logs (no inner instructions)");
        println!("\nNote: This transaction may still have been parsed from logs.");
        println!("The inner instruction support is specifically for:");
        println!("  - Transactions without logs");
        println!("  - Failed transactions (no logs emitted)");
        println!("  - Additional data beyond what logs provide");
    } else {
        println!("=== Parsed Events ===\n");
        for (i, event) in events.iter().enumerate() {
            println!("Event #{}:", i + 1);
            match event {
                sol_parser_sdk::DexEvent::PumpSwapBuy(e) => {
                    println!("  Type: PumpSwap Buy");
                    println!("  Metadata: {:?}", e.metadata);
                    println!("  Base Amount Out: {}", e.base_amount_out);
                    println!("  Quote Amount In: {}", e.user_quote_amount_in);
                }
                sol_parser_sdk::DexEvent::PumpSwapSell(e) => {
                    println!("  Type: PumpSwap Sell");
                    println!("  Metadata: {:?}", e.metadata);
                    println!("  Base Amount In: {}", e.base_amount_in);
                    println!("  Quote Amount Out: {}", e.user_quote_amount_out);
                }
                sol_parser_sdk::DexEvent::PumpSwapCreatePool(e) => {
                    println!("  Type: PumpSwap Create Pool");
                    println!("  Metadata: {:?}", e.metadata);
                    println!("  Pool: {}", e.pool);
                    println!("  Creator: {}", e.creator);
                }
                sol_parser_sdk::DexEvent::PumpSwapLiquidityAdded(e) => {
                    println!("  Type: PumpSwap Liquidity Added");
                    println!("  Metadata: {:?}", e.metadata);
                    println!("  Base Amount In: {}", e.base_amount_in);
                    println!("  Quote Amount In: {}", e.quote_amount_in);
                }
                sol_parser_sdk::DexEvent::PumpSwapLiquidityRemoved(e) => {
                    println!("  Type: PumpSwap Liquidity Removed");
                    println!("  Metadata: {:?}", e.metadata);
                    println!("  Base Amount Out: {}", e.base_amount_out);
                    println!("  Quote Amount Out: {}", e.quote_amount_out);
                }
                other => {
                    println!("  Type: {:?}", other);
                }
            }
            println!();
        }
    }

    println!("\n=== Summary ===");
    println!("✓ sol-parser-sdk successfully parsed the transaction!");
    println!("  The new RPC parsing API supports:");
    println!("  - Direct parsing from RPC (no gRPC streaming needed)");
    println!("  - Inner instruction parsing (16-byte discriminators)");
    println!("  - All 10 DEX protocols");
    println!("  - Perfect for testing and validation");

    println!("\n✓ Example completed!");
}
