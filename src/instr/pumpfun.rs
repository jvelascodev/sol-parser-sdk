//! PumpFun 指令解析器
//!
//! 使用 match discriminator 模式解析 PumpFun 指令

use super::program_ids;
use super::utils::*;
use crate::core::events::*;
use solana_sdk::{pubkey::Pubkey, signature::Signature};

/// PumpFun discriminator 常量
pub mod discriminators {
    // pub const CREATE: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];
    // pub const BUY: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
    // pub const SELL: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
    pub const MIGRATE_EVENT_LOG: [u8; 8] = [189, 233, 93, 185, 92, 148, 234, 148];
}

/// PumpFun 程序 ID
pub const PROGRAM_ID_PUBKEY: Pubkey = program_ids::PUMPFUN_PROGRAM_ID;

/// 主要的 PumpFun 指令解析函数
pub fn parse_instruction(
    instruction_data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
    grpc_recv_us: i64,
) -> Option<DexEvent> {
    if instruction_data.len() < 8 {
        return None;
    }

    let discriminator: [u8; 8] = instruction_data[0..8].try_into().ok()?;
    let data = &instruction_data[8..];

    // match discriminator {
    //     discriminators::CREATE => {
    //         parse_create_instruction(data, accounts, signature, slot, tx_index, block_time_us)
    //     }
    //     discriminators::BUY => {
    //         parse_buy_instruction(data, accounts, signature, slot, tx_index, block_time_us)
    //     }
    //     discriminators::SELL => {
    //         parse_sell_instruction(data, accounts, signature, slot, tx_index, block_time_us)
    //     }
    //     _ => None,
    // }

    if instruction_data.len() < 16 {
        return None;
    }

    let cpi_discriminator: [u8; 8] = instruction_data[8..16].try_into().ok()?;
    let cpi_data = &instruction_data[16..];

    match cpi_discriminator {
        discriminators::MIGRATE_EVENT_LOG => {
            return parse_migrate_log_instruction(
                cpi_data,
                accounts,
                signature,
                slot,
                tx_index,
                block_time_us,
                grpc_recv_us,
            )
        }
        _ => None,
    }
}

/// 解析 Migrate 指令
fn parse_migrate_log_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
    rpc_recv_us: i64,
) -> Option<DexEvent> {
    let mut offset = 0;

    // user (Pubkey - 32 bytes)
    let user = read_pubkey(data, offset)?;
    offset += 32;

    // mint (Pubkey - 32 bytes)
    let mint = read_pubkey(data, offset)?;
    offset += 32;

    // mintAmount (u64 - 8 bytes)
    let mint_amount = read_u64_le(data, offset)?;
    offset += 8;

    // solAmount (u64 - 8 bytes)
    let sol_amount = read_u64_le(data, offset)?;
    offset += 8;

    // poolMigrationFee (u64 - 8 bytes)
    let pool_migration_fee = read_u64_le(data, offset)?;
    offset += 8;

    // bondingCurve (Pubkey - 32 bytes)
    let bonding_curve = read_pubkey(data, offset)?;
    offset += 32;

    // timestamp (i64 - 8 bytes)
    let timestamp = read_u64_le(data, offset)? as i64;
    offset += 8;

    // pool (Pubkey - 32 bytes)
    let pool = read_pubkey(data, offset)?;

    let metadata =
        create_metadata(signature, slot, tx_index, block_time_us.unwrap_or_default(), rpc_recv_us);

    Some(DexEvent::PumpFunMigrate(PumpFunMigrateEvent {
        metadata,
        user,
        mint,
        mint_amount,
        sol_amount,
        pool_migration_fee,
        bonding_curve,
        timestamp,
        pool,
    }))
}

// 解析创建指令
// fn parse_create_instruction(
//     data: &[u8],
//     accounts: &[Pubkey],
//     signature: Signature,
//     slot: u64,
//     tx_index: u64,
//     block_time_us: Option<i64>,
// ) -> Option<DexEvent> {
//     let mint = get_account(accounts, 0)?;
//     let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, mint);

//     Some(DexEvent::PumpFunCreate(PumpFunCreateTokenEvent {
//         metadata,
//         name: "Unknown".to_string(),
//         symbol: "UNK".to_string(),
//         uri: String::new(),
//         mint,
//         bonding_curve: get_account(accounts, 1).unwrap_or_default(),
//         user: get_account(accounts, 2).unwrap_or_default(),
//         creator: Pubkey::default(), // 将从日志填充
//         timestamp: 0,
//         virtual_token_reserves: 1_073_000_000_000_000,
//         virtual_sol_reserves: 30_000_000_000,
//         real_token_reserves: 0, // 将从日志填充
//         token_total_supply: 0,  // 将从日志填充
//     }))
// }

// /// 解析买入指令
// fn parse_buy_instruction(
//     data: &[u8],
//     accounts: &[Pubkey],
//     signature: Signature,
//     slot: u64,
//     tx_index: u64,
//     block_time_us: Option<i64>,
// ) -> Option<DexEvent> {
//     let mut offset = 0;
//     let amount = read_u64_le(data, offset)?;
//     offset += 8;
//     let max_sol_cost = read_u64_le(data, offset)?;
//     let mint = get_account(accounts, 2)?; // mint is at index 2
//     let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, mint);
//     Some(DexEvent::PumpFunTrade(PumpFunTradeEvent {
//         metadata,
//         mint,
//         is_buy: true,
//         ..Default::default()
//     }))
// }

// /// 解析卖出指令
// fn parse_sell_instruction(
//     data: &[u8],
//     accounts: &[Pubkey],
//     signature: Signature,
//     slot: u64,
//     tx_index: u64,
//     block_time_us: Option<i64>,
// ) -> Option<DexEvent> {
//     let mut offset = 0;
//     let amount = read_u64_le(data, offset)?;
//     offset += 8;
//     let min_sol_output = read_u64_le(data, offset)?;
//     let mint = get_account(accounts, 2)?; // mint is at index 2
//     let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, mint);
//     Some(DexEvent::PumpFunTrade(PumpFunTradeEvent {
//         metadata,
//         mint,
//         is_buy: false,
//         ..Default::default()
//     }))
// }
