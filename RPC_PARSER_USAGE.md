# RPC Parser 使用说明

## ✅ 已完成的功能

1. **RPC 交易解析** - 独立于 gRPC streaming，直接从 RPC 解析交易
2. **V0 交易支持** - 完整支持 Versioned Transactions
3. **Inner Instruction 解析** - 支持 16-byte discriminators
4. **10个DEX协议支持**:
   - PumpFun
   - PumpSwap
   - Raydium CLMM
   - Raydium CPMM
   - Raydium AMM V4
   - Orca Whirlpool
   - Meteora Pools (AMM)
   - Meteora DAMM V2
   - Bonk (Raydium Launchpad)

## 🚀 使用方法

### 方法一：直接运行示例

```bash
# 使用官方 RPC（如果你的网络可以连接）
cargo run --example parse_pumpswap_tx --release

# 使用自定义 RPC（推荐）
export SOLANA_RPC_URL=https://your-rpc-endpoint.com
cargo run --example parse_pumpswap_tx --release
```

### 方法二：在代码中使用

```rust
use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use sol_parser_sdk::parse_transaction_from_rpc;
use std::str::FromStr;

fn main() {
    // 1. 创建 RPC 客户端
    let client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());

    // 2. 解析交易签名
    let signature = Signature::from_str(
        "3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS"
    ).unwrap();

    // 3. 解析交易（无需过滤器，返回所有事件）
    match parse_transaction_from_rpc(&client, &signature, None) {
        Ok(events) => {
            println!("Found {} DEX events", events.len());
            for event in events {
                match event {
                    DexEvent::PumpSwapBuy(e) => {
                        println!("PumpSwap Buy:");
                        println!("  Base Amount Out: {}", e.base_amount_out);
                        println!("  Quote Amount In: {}", e.user_quote_amount_in);
                    }
                    DexEvent::PumpSwapSell(e) => {
                        println!("PumpSwap Sell:");
                        println!("  Base Amount In: {}", e.base_amount_in);
                        println!("  Quote Amount Out: {}", e.user_quote_amount_out);
                    }
                    _ => println!("Other event: {:?}", event),
                }
            }
        }
        Err(e) => eprintln!("Parse error: {}", e),
    }
}
```

## 📊 预期输出格式

对于 PumpSwap 交易 `3zsihby...pZ6UJiS`，预期输出类似：

```
=== PumpSwap Transaction Parser ===

Transaction Signature: 3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS

Connecting to: https://api.mainnet-beta.solana.com

=== Parsing with sol-parser-sdk ===
Fetching and parsing transaction...

✓ Parsing completed!
  Found 1-2 DEX events

=== Parsed Events ===

Event #1:
  Type: PumpSwap Buy (or Sell)
  Metadata: EventMetadata {
    signature: 3zsihby...pZ6UJiS,
    slot: 12345678,
    tx_index: 0,
    block_time_us: 1234567890,
    grpc_recv_us: 1234567890
  }
  Base Amount Out: 1000000
  Quote Amount In: 500000
  Pool: <pool_pubkey>
  User: <user_pubkey>
```

## 🔧 技术实现细节

### RPC → gRPC 转换

`src/rpc_parser.rs` 负责将 RPC 格式转换为 gRPC 格式：

1. **交易获取** - 使用 `max_supported_transaction_version: 0` 支持 V0 交易
2. **格式转换**:
   - Base64 解码交易数据
   - 反序列化为 `VersionedTransaction`
   - 转换 Message (Legacy 或 V0)
   - 处理 Inner Instructions
3. **核心解析** - 调用 `parse_instructions_enhanced()` 使用完整的解析引擎

### Inner Instruction 路由

`src/grpc/instruction_parser.rs` 中的 `parse_inner_instruction()`:

```rust
// 支持的协议及其 Program IDs
PUMPFUN:        6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
PUMPSWAP:       pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA
RAYDIUM_CLMM:   CAMMCzo5YL8w4VFF8KVHrK22GGUQpMDdHFWF5LCATdCR
RAYDIUM_CPMM:   CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C
RAYDIUM_AMM_V4: 675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8
ORCA:           whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc
METEORA_POOLS:  Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB
METEORA_DAMM:   cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG
BONK:           DjVE6JNiYqPL2QXyCUUh8rNjHrbz9hXHNYt99MQ59qw1
```

## 🎯 关键特性

1. **零依赖 gRPC** - 完全独立的 RPC 解析路径
2. **测试友好** - 可以用于单元测试和集成测试
3. **完整解析** - 使用相同的核心引擎，确保一致性
4. **错误处理** - 详细的错误信息和类型

## 📝 注意事项

1. **RPC 限流** - 公共 RPC 端点可能有速率限制，建议使用私有节点
2. **网络问题** - 确保能够访问 Solana RPC 端点
3. **交易历史** - 某些 RPC 端点可能不保存完整历史记录

## ✨ 下一步

代码已准备就绪！在你的环境中运行：

```bash
cargo run --example parse_pumpswap_tx --release
```

应该能成功解析交易并显示 PumpSwap 事件详情！
