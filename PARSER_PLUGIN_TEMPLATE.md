# Parser Plugin Implementation Template
# 解析器插件实现模板

## 📋 概述

本文档提供了为 DEX 协议实现可插拔解析器的标准模板和步骤指南。

## 🎯 实现步骤

### 步骤 1: 为事件结构添加 Borsh 支持

在 `src/core/events.rs` 中，为目标事件添加 `BorshDeserialize` trait：

```rust
use borsh::BorshDeserialize;

/// 示例：交易事件
#[derive(Debug, Clone, Serialize, Deserialize, Default, BorshDeserialize)]
pub struct YourDexTradeEvent {
    #[borsh(skip)]  // metadata 不参与反序列化
    pub metadata: EventMetadata,

    // Borsh 序列化字段（按顺序）
    pub timestamp: i64,
    pub amount_in: u64,
    pub amount_out: u64,
    pub user: Pubkey,
    pub pool: Pubkey,

    // 额外字段（不在 Borsh 数据中，从指令账户填充）
    #[borsh(skip)]
    pub token_mint_a: Pubkey,
    #[borsh(skip)]
    pub token_mint_b: Pubkey,
}
```

### 步骤 2: 实现两种解析器

在 `src/instr/your_dex_inner.rs` 中：

```rust
//! YourDex Inner Instruction 解析器
//!
//! ## 解析器插件系统
//!
//! 本模块提供两种可插拔的解析器实现：
//!
//! ### 1. Borsh 反序列化解析器（默认，推荐）
//! - **启用**: `cargo build --features parse-borsh` （默认）
//! - **优点**: 类型安全、代码简洁、易维护、自动验证
//! - **适用**: 一般场景、需要稳定性和可维护性的项目
//!
//! ### 2. 零拷贝解析器（高性能）
//! - **启用**: `cargo build --features parse-zero-copy --no-default-features`
//! - **优点**: 最快、零拷贝、无验证开销、适合超高频场景
//! - **适用**: 性能关键路径、每秒数万次解析的场景

use crate::core::events::*;
use crate::instr::inner_common::*;

#[cfg(feature = "parse-borsh")]
use borsh::BorshDeserialize;

/// Discriminators
pub mod discriminators {
    pub const TRADE_EVENT: [u8; 16] = [...]; // 16 字节 discriminator
}

// ============================================================================
// Trade 事件解析器
// ============================================================================

/// 解析 Trade 事件（统一入口）
///
/// 根据编译时的 feature flag 自动选择解析器实现
#[inline(always)]
fn parse_trade_inner(data: &[u8], metadata: EventMetadata) -> Option<DexEvent> {
    #[cfg(feature = "parse-borsh")]
    {
        parse_trade_inner_borsh(data, metadata)
    }

    #[cfg(feature = "parse-zero-copy")]
    {
        parse_trade_inner_zero_copy(data, metadata)
    }
}

/// Borsh 反序列化解析器 - Trade 事件
///
/// **优点**: 类型安全、代码简洁、自动验证
#[cfg(feature = "parse-borsh")]
#[inline(always)]
fn parse_trade_inner_borsh(data: &[u8], metadata: EventMetadata) -> Option<DexEvent> {
    const EVENT_SIZE: usize = 123; // 实际事件大小

    if data.len() < EVENT_SIZE {
        return None;
    }

    // 一行代码解析所有字段
    let event = borsh::from_slice::<YourDexTradeEvent>(&data[..EVENT_SIZE]).ok()?;

    Some(DexEvent::YourDexTrade(YourDexTradeEvent {
        metadata,
        ..event
    }))
}

/// 零拷贝解析器 - Trade 事件
///
/// **优点**: 最快、零拷贝、无验证开销
#[cfg(feature = "parse-zero-copy")]
#[inline(always)]
fn parse_trade_inner_zero_copy(data: &[u8], metadata: EventMetadata) -> Option<DexEvent> {
    // 数据结构说明（方便维护）:
    // timestamp: i64 (8 bytes)
    // amount_in: u64 (8 bytes)
    // amount_out: u64 (8 bytes)
    // user: Pubkey (32 bytes)
    // pool: Pubkey (32 bytes)
    // Total: 88 bytes

    unsafe {
        const MIN_SIZE: usize = 8 + 8 + 8 + 32 + 32;
        if !check_length(data, MIN_SIZE) {
            return None;
        }

        let mut offset = 0;

        let timestamp = read_i64_unchecked(data, offset);
        offset += 8;
        let amount_in = read_u64_unchecked(data, offset);
        offset += 8;
        let amount_out = read_u64_unchecked(data, offset);
        offset += 8;
        let user = read_pubkey_unchecked(data, offset);
        offset += 32;
        let pool = read_pubkey_unchecked(data, offset);

        Some(DexEvent::YourDexTrade(YourDexTradeEvent {
            metadata,
            timestamp,
            amount_in,
            amount_out,
            user,
            pool,
            ..Default::default()
        }))
    }
}

/// 主入口：解析 inner instruction
#[inline]
pub fn parse_yourdex_inner_instruction(
    discriminator: &[u8; 16],
    data: &[u8],
    metadata: EventMetadata,
) -> Option<DexEvent> {
    match discriminator {
        &discriminators::TRADE_EVENT => parse_trade_inner(data, metadata),
        // 添加其他事件类型...
        _ => None,
    }
}
```

### 步骤 3: 添加到主解析器

在 `src/grpc/instruction_parser.rs` 中集成：

```rust
use crate::instr::your_dex_inner;

// 在 parse_all_inner_instructions 函数中添加：
if program_id == &your_dex_inner::PROGRAM_ID {
    if let Some(event) = your_dex_inner::parse_yourdex_inner_instruction(
        &discriminator_array,
        remaining_data,
        metadata,
    ) {
        return Some(event);
    }
}
```

### 步骤 4: 添加测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_borsh_parser() {
        // 测试 Borsh 解析器
    }

    #[cfg(feature = "parse-zero-copy")]
    #[test]
    fn test_zero_copy_parser() {
        // 测试零拷贝解析器
    }

    #[test]
    fn test_parser_consistency() {
        // 确保两种解析器产生相同结果
    }
}
```

## 📊 数据结构对齐要求

### Borsh 序列化字段顺序

**重要**：Borsh 序列化严格按照字段定义顺序。确保：

```rust
// ✅ 正确：字段顺序与 Borsh 数据一致
#[derive(BorshDeserialize)]
pub struct Event {
    pub field_a: u64,  // offset 0
    pub field_b: u64,  // offset 8
    pub field_c: Pubkey, // offset 16
}

// ❌ 错误：字段顺序与数据不匹配
#[derive(BorshDeserialize)]
pub struct Event {
    pub field_c: Pubkey,  // 错误的顺序！
    pub field_a: u64,
    pub field_b: u64,
}
```

### 跳过字段

使用 `#[borsh(skip)]` 标记不在序列化数据中的字段：

```rust
#[derive(BorshDeserialize)]
pub struct Event {
    pub data_field: u64,        // 在 Borsh 数据中

    #[borsh(skip)]
    pub metadata: EventMetadata, // 不在数据中，手动设置

    #[borsh(skip)]
    pub extra_info: Pubkey,      // 从指令账户填充
}
```

## 🧪 测试清单

为每个协议实现解析器插件时，确保：

- [ ] 添加了 `BorshDeserialize` trait
- [ ] 实现了 Borsh 解析器
- [ ] 实现了零拷贝解析器
- [ ] 两种解析器产生相同结果
- [ ] 添加了单元测试
- [ ] 更新了文档
- [ ] 测试了两种编译配置

## 📝 标准化命名规范

### 解析器函数命名

```rust
// 统一入口
fn parse_{event_type}_inner(data: &[u8], metadata: EventMetadata) -> Option<DexEvent>

// Borsh 实现
fn parse_{event_type}_inner_borsh(data: &[u8], metadata: EventMetadata) -> Option<DexEvent>

// 零拷贝实现
fn parse_{event_type}_inner_zero_copy(data: &[u8], metadata: EventMetadata) -> Option<DexEvent>
```

### 示例

```rust
// Trade 事件
fn parse_trade_inner(...)         // 统一入口
fn parse_trade_inner_borsh(...)   // Borsh 实现
fn parse_trade_inner_zero_copy(...) // 零拷贝实现

// Swap 事件
fn parse_swap_inner(...)
fn parse_swap_inner_borsh(...)
fn parse_swap_inner_zero_copy(...)
```

## ⚡ 性能优化建议

### 1. 使用 #[inline(always)]

所有解析函数都应标记为 `#[inline(always)]`：

```rust
#[inline(always)]
fn parse_trade_inner_borsh(...) -> Option<DexEvent> {
    // 实现
}
```

### 2. 避免不必要的分配

```rust
// ✅ 好：零分配
let event = borsh::from_slice::<Event>(&data[..SIZE]).ok()?;

// ❌ 差：不必要的 Vec 分配
let vec = data[..SIZE].to_vec();
let event = borsh::from_slice::<Event>(&vec).ok()?;
```

### 3. 批量长度检查

```rust
// ✅ 好：一次检查
unsafe {
    const MIN_SIZE: usize = 8 + 32 + 8;
    if !check_length(data, MIN_SIZE) {
        return None;
    }
    // 然后安全读取
}

// ❌ 差：多次检查
unsafe {
    if data.len() < 8 { return None; }
    let a = read_u64(...);
    if data.len() < 40 { return None; }
    let b = read_pubkey(...);
}
```

## 📚 协议优先级

建议按此顺序实现解析器插件：

1. **PumpSwap** ✅ 已完成
2. **PumpFun** - 高优先级
3. **Raydium CLMM** - 高优先级
4. **Raydium AMM V4** - 中优先级
5. **Raydium CPMM** - 中优先级
6. **Meteora DAMM V2** - 中优先级
7. **Orca Whirlpool** - 低优先级
8. **其他协议** - 按需实现

## 🔧 工具函数

### 零拷贝读取函数（已提供）

在 `src/instr/inner_common.rs` 中：

- `read_u8_unchecked()`
- `read_u16_unchecked()`
- `read_u32_unchecked()`
- `read_u64_unchecked()`
- `read_u128_unchecked()`
- `read_i64_unchecked()`
- `read_i128_unchecked()`
- `read_bool_unchecked()`
- `read_pubkey_unchecked()`
- `read_string_unchecked()`
- `check_length()`

### 检查清单

复制此清单用于每个新协议：

```markdown
## {Protocol Name} 解析器插件实现

- [ ] Step 1: 添加 BorshDeserialize trait
- [ ] Step 2: 实现 Borsh 解析器
- [ ] Step 3: 实现零拷贝解析器
- [ ] Step 4: 添加到主解析器
- [ ] Step 5: 编写单元测试
- [ ] Step 6: 测试两种配置
- [ ] Step 7: 更新文档
- [ ] Step 8: 性能基准测试（可选）
```

---

**完成模板后**：复制 PumpSwap 的实现作为参考，它是最完整的示例。
