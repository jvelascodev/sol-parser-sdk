//! 账户填充模块
//!
//! 负责从指令账户数据填充DEX事件中缺失的账户字段
//! 每个平台的每个事件类型都有专门的填充函数
//! 只填充那些会变化的账户，排除系统程序等常量账户

use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{Transaction, TransactionStatusMeta};

/// 账户获取辅助函数类型
type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

#[inline(always)]
fn read_pubkey_fast(bytes: &[u8]) -> Pubkey {
    crate::logs::utils::read_pubkey(bytes, 0).unwrap_or_default()
}

/// 获取指令账户访问器
/// 返回一个可以通过索引获取 Pubkey 的闭包
fn get_instruction_account_getter<'a>(
    meta: &'a TransactionStatusMeta,
    transaction: &'a Option<Transaction>,
    account_keys: Option<&'a Vec<Vec<u8>>>,
    // 地址表
    loaded_writable_addresses: &'a Vec<Vec<u8>>,
    loaded_readonly_addresses: &'a Vec<Vec<u8>>,
    index: &(i32, i32), // (outer_index, inner_index)
) -> Option<impl Fn(usize) -> Pubkey + 'a> {
    // 1. 获取指令的账户索引数组
    let accounts = if index.1 >= 0 {
        // 内层指令
        meta.inner_instructions
            .iter()
            .find(|i| i.index == index.0 as u32)?
            .instructions
            .get(index.1 as usize)?
            .accounts
            .as_slice()
    } else {
        // 外层指令
        transaction
            .as_ref()?
            .message
            .as_ref()?
            .instructions
            .get(index.0 as usize)?
            .accounts
            .as_slice()
    };

    // 2. 创建高性能的账户查找闭包
    Some(move |acc_index: usize| -> Pubkey {
        // 获取账户在交易中的索引
        let account_index = match accounts.get(acc_index) {
            Some(&idx) => idx as usize,
            None => return Pubkey::default(),
        };
        // 早期返回优化
        let Some(keys) = account_keys else {
            return Pubkey::default();
        };
        // 主账户列表
        if let Some(key_bytes) = keys.get(account_index) {
            return read_pubkey_fast(key_bytes);
        }
        // 可写地址
        let writable_offset = account_index.saturating_sub(keys.len());
        if let Some(key_bytes) = loaded_writable_addresses.get(writable_offset) {
            return read_pubkey_fast(key_bytes);
        }
        // 只读地址
        let readonly_offset = writable_offset.saturating_sub(loaded_writable_addresses.len());
        if let Some(key_bytes) = loaded_readonly_addresses.get(readonly_offset) {
            return read_pubkey_fast(key_bytes);
        }
        Pubkey::default()
    })
}

/// 主要的账户填充调度函数
pub fn fill_accounts_from_transaction_data(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<String, Vec<(i32, i32)>>,
) {
    // 获取账户的辅助函数
    let account_keys =
        transaction.as_ref().and_then(|tx| tx.message.as_ref()).map(|msg| &msg.account_keys);
    let loaded_writable_addresses = &meta.loaded_writable_addresses;
    let loaded_readonly_addresses = &meta.loaded_readonly_addresses;
    match event {
        // PumpFun 事件填充
        DexEvent::PumpFunTrade(ref mut trade_event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::PUMPFUN_PROGRAM_ID)
                .as_ref()
                .and_then(|v| v.last())
            {
                if let Some(get_account) = get_instruction_account_getter(
                    meta,
                    transaction,
                    account_keys,
                    loaded_writable_addresses,
                    loaded_readonly_addresses,
                    invoke,
                ) {
                    pumpfun::fill_trade_accounts(trade_event, &get_account);
                }
            }
        }
        _ => {} // 其他事件类型TODO
    }
}

/// 主要的账户填充调度函数
pub fn fill_accounts_from_instruction_data(event: &mut DexEvent, instruction_accounts: &[Pubkey]) {
    // 获取账户的辅助函数
    let get_account =
        |index: usize| -> Pubkey { instruction_accounts.get(index).cloned().unwrap_or_default() };

    match event {
        // PumpFun 事件填充
        DexEvent::PumpFunTrade(ref mut trade_event) => {
            pumpfun::fill_trade_accounts(trade_event, &get_account);
        }
        DexEvent::PumpFunCreate(ref mut create_event) => {
            pumpfun::fill_create_accounts(create_event, &get_account);
        }
        DexEvent::PumpFunMigrate(ref mut migrate_event) => {
            pumpfun::fill_migrate_accounts(migrate_event, &get_account);
        }

        // Raydium 事件填充
        DexEvent::RaydiumClmmSwap(ref mut swap_event) => {
            raydium::fill_clmm_swap_accounts(swap_event, &get_account);
        }
        DexEvent::RaydiumCpmmSwap(ref mut swap_event) => {
            raydium::fill_cpmm_swap_accounts(swap_event, &get_account);
        }
        DexEvent::RaydiumAmmV4Swap(ref mut swap_event) => {
            raydium::fill_amm_v4_swap_accounts(swap_event, &get_account);
        }

        // Orca 事件填充
        DexEvent::OrcaWhirlpoolSwap(ref mut swap_event) => {
            orca::fill_whirlpool_swap_accounts(swap_event, &get_account);
        }

        // Meteora 事件填充
        DexEvent::MeteoraPoolsSwap(ref mut swap_event) => {
            meteora::fill_pools_swap_accounts(swap_event, &get_account);
        }
        DexEvent::MeteoraDammV2Swap(ref mut swap_event) => {
            meteora::fill_damm_v2_swap_accounts(swap_event, &get_account);
        }
        DexEvent::MeteoraDlmmSwap(ref mut swap_event) => {
            meteora::fill_dlmm_swap_accounts(swap_event, &get_account);
        }
        DexEvent::MeteoraDlmmAddLiquidity(ref mut event) => {
            meteora::fill_dlmm_add_liquidity_accounts(event, &get_account);
        }
        DexEvent::MeteoraDlmmRemoveLiquidity(ref mut event) => {
            meteora::fill_dlmm_remove_liquidity_accounts(event, &get_account);
        }

        // Bonk 事件填充
        DexEvent::BonkTrade(ref mut trade_event) => {
            bonk::fill_trade_accounts(trade_event, &get_account);
        }

        // 其他事件类型暂时不处理
        _ => {}
    }
}

/// PumpFun 账户填充模块
pub mod pumpfun {
    use super::*;

    /// 填充 PumpFun Trade 事件账户
    /// 基于PumpFun IDL的buy/sell指令账户映射
    pub fn fill_trade_accounts(
        trade_event: &mut PumpFunTradeEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if trade_event.user == Pubkey::default() {
            trade_event.user = get_account(6);
        }
        if trade_event.bonding_curve == Pubkey::default() {
            trade_event.bonding_curve = get_account(3);
        }
        if trade_event.associated_bonding_curve == Pubkey::default() {
            trade_event.associated_bonding_curve = get_account(4);
        }
        if trade_event.creator_vault == Pubkey::default() {
            trade_event.creator_vault =
                if trade_event.is_buy { get_account(9) } else { get_account(8) };
        }
    }

    /// 填充 PumpFun Create 事件账户
    /// 基于PumpFun IDL create指令账户映射:
    /// 0: mint
    /// 1: mint_authority
    /// 2: bonding_curve
    /// 3: associated_bonding_curve
    /// 4: global
    /// 5: mpl_token_metadata
    /// 6: metadata
    /// 7: user
    pub fn fill_create_accounts(
        create_event: &mut PumpFunCreateTokenEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if create_event.mint == Pubkey::default() {
            create_event.mint = get_account(0);
        }
        if create_event.bonding_curve == Pubkey::default() {
            create_event.bonding_curve = get_account(2);
        }
        if create_event.user == Pubkey::default() {
            create_event.user = get_account(7);
        }
    }

    /// 填充 PumpFun Migrate 事件账户
    /// 基于PumpFun IDL migrate指令账户映射:
    /// 0: global
    /// 1: withdraw_authority
    /// 2: mint
    /// 3: bonding_curve
    /// 4: associated_bonding_curve
    /// 5: user
    /// 8: pump_amm
    /// 9: pool
    /// 10: pool_authority
    /// 11: pool_authority_mint_account
    /// 12: pool_authority_wsol_account
    /// 13: amm_global_config
    /// 14: wsol_mint
    /// 15: lp_mint
    /// 16: user_pool_token_account
    /// 17: pool_base_token_account
    /// 18: pool_quote_token_account
    pub fn fill_migrate_accounts(
        migrate_event: &mut PumpFunMigrateEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // 暂时注释，以后会用，AI禁止改动
        // if migrate_event.global == Pubkey::default() {
        //     migrate_event.global = get_account(0);
        // }
        // if migrate_event.withdraw_authority == Pubkey::default() {
        //     migrate_event.withdraw_authority = get_account(1);
        // }
        // if migrate_event.mint == Pubkey::default() {
        //     migrate_event.mint = get_account(2);
        // }
        // if migrate_event.bonding_curve == Pubkey::default() {
        //     migrate_event.bonding_curve = get_account(3);
        // }
        // if migrate_event.associated_bonding_curve == Pubkey::default() {
        //     migrate_event.associated_bonding_curve = get_account(4);
        // }
        // if migrate_event.user == Pubkey::default() {
        //     migrate_event.user = get_account(5);
        // }
        // if migrate_event.pump_amm == Pubkey::default() {
        //     migrate_event.pump_amm = get_account(8);
        // }
        // if migrate_event.pool == Pubkey::default() {
        //     migrate_event.pool = get_account(9);
        // }
        // if migrate_event.pool_authority == Pubkey::default() {
        //     migrate_event.pool_authority = get_account(10);
        // }
        // if migrate_event.pool_authority_mint_account == Pubkey::default() {
        //     migrate_event.pool_authority_mint_account = get_account(11);
        // }
        // if migrate_event.pool_authority_wsol_account == Pubkey::default() {
        //     migrate_event.pool_authority_wsol_account = get_account(12);
        // }
        // if migrate_event.amm_global_config == Pubkey::default() {
        //     migrate_event.amm_global_config = get_account(13);
        // }
        // if migrate_event.wsol_mint == Pubkey::default() {
        //     migrate_event.wsol_mint = get_account(14);
        // }
        // if migrate_event.lp_mint == Pubkey::default() {
        //     migrate_event.lp_mint = get_account(15);
        // }
        // if migrate_event.user_pool_token_account == Pubkey::default() {
        //     migrate_event.user_pool_token_account = get_account(16);
        // }
        // if migrate_event.pool_base_token_account == Pubkey::default() {
        //     migrate_event.pool_base_token_account = get_account(17);
        // }
        // if migrate_event.pool_quote_token_account == Pubkey::default() {
        //     migrate_event.pool_quote_token_account = get_account(18);
        // }
    }
}

/// Raydium 账户填充模块
pub mod raydium {
    use super::*;

    /// 填充 Raydium CLMM Swap 事件账户
    pub fn fill_clmm_swap_accounts(
        swap_event: &mut RaydiumClmmSwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // 基于Raydium CLMM IDL swap指令账户映射
        if swap_event.pool_state == Pubkey::default() {
            swap_event.pool_state = get_account(1);
        }
        if swap_event.sender == Pubkey::default() {
            swap_event.sender = get_account(0);
        }
    }

    /// 填充 Raydium CPMM Swap 事件账户
    /// 基于Raydium CPMM swapBaseInput/swapBaseOutput指令IDL定义账户映射
    pub fn fill_cpmm_swap_accounts(
        swap_event: &mut RaydiumCpmmSwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // Raydium CPMM swap指令账户映射 (基于IDL):
        // 0: payer - 用户执行交换
        // 1: authority - 权限账户
        // 2: ammConfig - AMM配置
        // 3: poolState - 池状态
        // 4: inputTokenAccount - 输入代币账户
        // 5: outputTokenAccount - 输出代币账户
        // 6: inputVault - 输入库存
        // 7: outputVault - 输出库存
        // 8: inputTokenProgram - 输入代币程序
        // 9: outputTokenProgram - 输出代币程序
        // 10: inputTokenMint - 输入代币mint
        // 11: outputTokenMint - 输出代币mint
        // 12: observationState - 观察状态

        // 基于最新IDL swapBaseInput指令账户映射:
        // 0: payer - 用户执行交换
        // 1: authority - 权限账户
        // 2: ammConfig - AMM配置
        // 3: poolState - 池状态
        // 4: inputTokenAccount - 用户输入代币账户
        // 5: outputTokenAccount - 用户输出代币账户
        // 6: inputVault - 输入库存
        // 7: outputVault - 输出库存
        // 10: inputTokenMint - 输入代币mint
        // 11: outputTokenMint - 输出代币mint

        // 暂时注释，以后会用，AI禁止改动
        // if swap_event.payer == Pubkey::default() {
        //     swap_event.payer = get_account(0);
        // }
        // if swap_event.authority == Pubkey::default() {
        //     swap_event.authority = get_account(1);
        // }
        // if swap_event.amm_config == Pubkey::default() {
        //     swap_event.amm_config = get_account(2);
        // }
        // if swap_event.pool_state == Pubkey::default() {
        //     swap_event.pool_state = get_account(3);
        // }
        // if swap_event.input_token_account == Pubkey::default() {
        //     swap_event.input_token_account = get_account(4);
        // }
        // if swap_event.output_token_account == Pubkey::default() {
        //     swap_event.output_token_account = get_account(5);
        // }
        // if swap_event.input_vault == Pubkey::default() {
        //     swap_event.input_vault = get_account(6);
        // }
        // if swap_event.output_vault == Pubkey::default() {
        //     swap_event.output_vault = get_account(7);
        // }
        // if swap_event.input_token_mint == Pubkey::default() {
        //     swap_event.input_token_mint = get_account(10);
        // }
        // if swap_event.output_token_mint == Pubkey::default() {
        //     swap_event.output_token_mint = get_account(11);
        // }
    }

    /// 填充 Raydium AMM V4 Swap 事件账户
    pub fn fill_amm_v4_swap_accounts(
        swap_event: &mut RaydiumAmmV4SwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // TODO: 基于Raydium AMM V4 IDL定义账户映射
        if swap_event.amm == Pubkey::default() {
            swap_event.amm = get_account(1);
        }
        // RaydiumAmmV4SwapEvent 没有user字段，需要后续添加
    }
}

/// Orca 账户填充模块
pub mod orca {
    use super::*;

    /// 填充 Orca Whirlpool Swap 事件账户
    pub fn fill_whirlpool_swap_accounts(
        swap_event: &mut OrcaWhirlpoolSwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // 基于Orca Whirlpool swap指令IDL账户映射:
        // 0: tokenProgram - SPL代币程序 (常量)
        // 1: tokenAuthority - 代币权限
        // 2: whirlpool - 池状态
        // 3: tokenOwnerAccountA - 用户代币A账户
        // 4: tokenVaultA - 池代币A库存
        // 5: tokenOwnerAccountB - 用户代币B账户
        // 6: tokenVaultB - 池代币B库存
        // 7: tickArray0 - tick数组0
        // 8: tickArray1 - tick数组1
        // 9: tickArray2 - tick数组2

        // 暂时注释，以后会用，AI禁止改动
        // if swap_event.token_authority == Pubkey::default() {
        //     swap_event.token_authority = get_account(1);
        // }
        // if swap_event.whirlpool == Pubkey::default() {
        //     swap_event.whirlpool = get_account(2);
        // }
        // if swap_event.token_owner_account_a == Pubkey::default() {
        //     swap_event.token_owner_account_a = get_account(3);
        // }
        // if swap_event.token_vault_a == Pubkey::default() {
        //     swap_event.token_vault_a = get_account(4);
        // }
        // if swap_event.token_owner_account_b == Pubkey::default() {
        //     swap_event.token_owner_account_b = get_account(5);
        // }
        // if swap_event.token_vault_b == Pubkey::default() {
        //     swap_event.token_vault_b = get_account(6);
        // }
        // if swap_event.tick_array_0 == Pubkey::default() {
        //     swap_event.tick_array_0 = get_account(7);
        // }
        // if swap_event.tick_array_1 == Pubkey::default() {
        //     swap_event.tick_array_1 = get_account(8);
        // }
        // if swap_event.tick_array_2 == Pubkey::default() {
        //     swap_event.tick_array_2 = get_account(9);
        // }
    }
}

/// Meteora 账户填充模块
pub mod meteora {
    use super::*;

    /// 填充 Meteora Pools Swap 事件账户
    /// 基于Meteora AMM IDL swap指令账户映射:
    /// 0: pool
    /// 1: userSourceToken
    /// 2: userDestinationToken
    /// 3: aVault
    /// 4: bVault
    /// 5: aTokenVault
    /// 6: bTokenVault
    /// 7: aVaultLpMint
    /// 8: bVaultLpMint
    /// 9: aVaultLp
    /// 10: bVaultLp
    /// 11: adminTokenFee
    /// 12: user
    pub fn fill_pools_swap_accounts(
        swap_event: &mut MeteoraPoolsSwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        // MeteoraPoolsSwapEvent只有IDL事件字段，无指令账户字段
    }

    /// 填充 Meteora DAMM V2 Swap 事件账户
    pub fn fill_damm_v2_swap_accounts(
        swap_event: &mut MeteoraDammV2SwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if swap_event.lb_pair == Pubkey::default() {
            swap_event.lb_pair = get_account(1);
        }
        if swap_event.from == Pubkey::default() {
            swap_event.from = get_account(0);
        }
    }

    /// 填充 Meteora DLMM Swap 事件账户
    /// 基于Meteora DLMM IDL swap指令账户映射:
    /// 0: lbPair
    /// 5: userTokenOut
    /// 10: user
    pub fn fill_dlmm_swap_accounts(
        swap_event: &mut MeteoraDlmmSwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if swap_event.pool == Pubkey::default() {
            swap_event.pool = get_account(0);
        }
        if swap_event.from == Pubkey::default() {
            swap_event.from = get_account(10);
        }
    }

    /// 填充 Meteora DLMM Add Liquidity 事件账户
    /// 基于Meteora DLMM IDL addLiquidity指令账户映射:
    /// 0: position
    /// 1: lbPair
    /// 11: sender
    pub fn fill_dlmm_add_liquidity_accounts(
        event: &mut MeteoraDlmmAddLiquidityEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if event.position == Pubkey::default() {
            event.position = get_account(0);
        }
        if event.pool == Pubkey::default() {
            event.pool = get_account(1);
        }
        if event.from == Pubkey::default() {
            event.from = get_account(11);
        }
    }

    /// 填充 Meteora DLMM Remove Liquidity 事件账户
    /// 基于Meteora DLMM IDL removeLiquidity指令账户映射:
    /// 0: position
    /// 1: lbPair
    /// 11: sender
    pub fn fill_dlmm_remove_liquidity_accounts(
        event: &mut MeteoraDlmmRemoveLiquidityEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if event.position == Pubkey::default() {
            event.position = get_account(0);
        }
        if event.pool == Pubkey::default() {
            event.pool = get_account(1);
        }
        if event.from == Pubkey::default() {
            event.from = get_account(11);
        }
    }
}

/// Bonk 账户填充模块
pub mod bonk {
    use super::*;

    /// 填充 Bonk Trade 事件账户
    pub fn fill_trade_accounts(trade_event: &mut BonkTradeEvent, get_account: &AccountGetter<'_>) {
        // 基于Bonk IDL swap指令账户映射
        if trade_event.user == Pubkey::default() {
            trade_event.user = get_account(0);
        }
        if trade_event.pool_state == Pubkey::default() {
            trade_event.pool_state = get_account(1);
        }
    }
}
