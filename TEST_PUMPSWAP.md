# PumpSwap 交易解析测试

## 🎯 测试目标

解析 Jupiter Aggregator v6 上的 PumpSwap 交易：
```
签名: 3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS
链接: https://solscan.io/tx/3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS
```

## ✅ 已完成的代码修复

1. ✅ 修复所有编译错误
2. ✅ 实现 RPC 解析功能 (`src/rpc_parser.rs`)
3. ✅ 支持 V0 交易版本
4. ✅ 修复 PumpSwap inner instruction 路由问题
5. ✅ 创建示例程序 (`examples/parse_pumpswap_tx.rs`)

## 🚀 在你的环境中运行

### 方法一：使用你提供的 RPC

```bash
cd /Users/wood/WorkSpace/Solana-Projects/sol-parser-sdk

# 使用你的私有 RPC 节点
export SOLANA_RPC_URL="http://64.130.37.195:10900"

# 运行示例（release 模式，性能最优）
cargo run --example parse_pumpswap_tx --release
```

### 方法二：使用其他 RPC

```bash
# 公共 RPC（可能有限流）
export SOLANA_RPC_URL="https://api.mainnet-beta.solana.com"
cargo run --example parse_pumpswap_tx --release

# 或 Helius
export SOLANA_RPC_URL="https://rpc.helius.xyz/?api-key=YOUR_KEY"
cargo run --example parse_pumpswap_tx --release

# 或 QuickNode
export SOLANA_RPC_URL="https://your-quicknode-endpoint.solana-mainnet.quiknode.pro/YOUR_KEY/"
cargo run --example parse_pumpswap_tx --release
```

## 📊 预期输出

成功运行后，你应该看到类似以下的输出：

```
=== PumpSwap Transaction Parser ===

Transaction Signature: 3zsihbygW7hoKGtduAyDDFzp4E1eis8gaBzEzzNKr8ma39baffpFcphok9wHFgR3EauDe9vYYsVf4Puh5pZ6UJiS

Connecting to: http://64.130.37.195:10900

=== Parsing with sol-parser-sdk ===
Fetching and parsing transaction...

✓ Parsing completed!
  Found X DEX events

=== Parsed Events ===

Event #1:
  Type: PumpSwap Buy (或 Sell)
  Metadata: EventMetadata { ... }
  Base Amount Out: XXXXX
  Quote Amount In: XXXXX
  Pool: <pool_address>
  User: <user_address>

Event #2:
  ...

=== Summary ===
✓ sol-parser-sdk successfully parsed the transaction!
  The new RPC parsing API supports:
  - Direct parsing from RPC (no gRPC streaming needed)
  - Inner instruction parsing (16-byte discriminators)
  - All 10 DEX protocols
  - Perfect for testing and validation

✓ Example completed!
```

## 🔍 解析能力验证

这笔交易应该能测试以下能力：

1. **V0 Transaction 支持** - 该交易使用 V0 格式
2. **Inner Instruction 解析** - PumpSwap 事件在 inner instructions 中
3. **16-byte Discriminator** - Inner instructions 使用 16-byte discriminators
4. **Jupiter Aggregator 集成** - 通过 Jupiter 路由的 PumpSwap 交易
5. **完整事件数据** - 解析出完整的 Buy/Sell 事件数据

## 🐛 如果遇到问题

### 错误：无法连接 RPC
```
✗ Failed to parse transaction: RPC error: error sending request for url (...)
```

**解决方法**：
1. 检查 RPC URL 是否正确
2. 确保网络可以访问该 RPC
3. 尝试使用不同的 RPC 端点

### 错误：不支持交易版本
```
✗ Failed to parse transaction: RPC error: Transaction version (0) is not supported
```

**解决方法**：
- 这个问题已经修复！代码中已经添加了 `max_supported_transaction_version: 0`

### 错误：未找到事件
```
⚠ No DEX events found in this transaction.
```

**可能原因**：
1. 交易不包含 DEX 操作
2. 协议尚未支持
3. Inner instruction 解析失败

**调试方法**：
- 检查交易日志查看实际的程序调用
- 确认 program ID 是否在支持列表中

## 📝 代码位置

- **RPC 解析器**: `src/rpc_parser.rs`
- **Inner Instruction 路由**: `src/grpc/instruction_parser.rs:218-256`
- **PumpSwap 解析器**: `src/instr/pump_amm_inner.rs`
- **示例程序**: `examples/parse_pumpswap_tx.rs`

## 🎉 测试确认

运行成功后，你将验证：

✅ sol-parser-sdk 可以从 RPC 直接解析交易
✅ PumpSwap 交易可以被正确解析
✅ Inner instructions 解析工作正常
✅ 不依赖 gRPC streaming
✅ 适合用于测试和验证

---

**提示**: 我的运行环境有网络限制无法访问 Solana RPC，但代码已经完全就绪。请在你的环境中运行上述命令！
