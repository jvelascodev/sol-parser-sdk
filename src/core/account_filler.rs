//! 账户填充模块
//!
//! 负责从指令账户数据填充DEX事件中缺失的账户字段
//! 每个平台的每个事件类型都有专门的填充函数
//! 只填充那些会变化的账户，排除系统程序等常量账户

use crate::core::events::*;
use crate::instr::utils::get_instruction_account_getter;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{Transaction, TransactionStatusMeta};

/// 账户获取辅助函数类型
type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

/// 主要的账户填充调度函数
pub fn fill_accounts_from_transaction_data(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<&str, Vec<(i32, i32)>>,
) {
    // 获取账户的辅助函数
    let account_keys =
        transaction.as_ref().and_then(|tx| tx.message.as_ref()).map(|msg| &msg.account_keys);
    let loaded_writable_addresses = &meta.loaded_writable_addresses;
    let loaded_readonly_addresses = &meta.loaded_readonly_addresses;
    match event {
        // PumpFun 事件填充 (所有交易类型共用相同的账户填充逻辑)
        DexEvent::PumpFunTrade(ref mut trade_event)
        | DexEvent::PumpFunBuy(ref mut trade_event)
        | DexEvent::PumpFunSell(ref mut trade_event)
        | DexEvent::PumpFunBuyExactSolIn(ref mut trade_event) => {
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
        DexEvent::PumpSwapBuy(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::PUMPSWAP_PROGRAM_ID)
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
                    pumpswap::fill_buy_accounts(event, &get_account);
                }
            }
        }
        DexEvent::PumpSwapSell(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::PUMPSWAP_PROGRAM_ID)
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
                    pumpswap::fill_sell_accounts(event, &get_account);
                }
            }
        }
        DexEvent::MeteoraDammV2Swap(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::METEORA_DAMM_V2_PROGRAM_ID)
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
                    meteora::fill_damm_v2_swap_accounts(event, &get_account);
                }
            }
        }
        _ => {} // 其他事件类型TODO
    }
}

/// 账户填充函数（用于 parse_transaction_events，使用 Pubkey 避免 to_string() 开销）
pub fn fill_accounts_with_owned_keys(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<Pubkey, Vec<(i32, i32)>>,
) {
    // 获取账户的辅助函数
    let account_keys =
        transaction.as_ref().and_then(|tx| tx.message.as_ref()).map(|msg| &msg.account_keys);
    let loaded_writable_addresses = &meta.loaded_writable_addresses;
    let loaded_readonly_addresses = &meta.loaded_readonly_addresses;
    match event {
        // PumpFun 事件填充 (所有交易类型共用相同的账户填充逻辑)
        DexEvent::PumpFunTrade(ref mut trade_event)
        | DexEvent::PumpFunBuy(ref mut trade_event)
        | DexEvent::PumpFunSell(ref mut trade_event)
        | DexEvent::PumpFunBuyExactSolIn(ref mut trade_event) => {
            if let Some(invoke) = program_invokes
                .get(&crate::grpc::program_ids::PUMPFUN_PROGRAM)
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
        DexEvent::PumpSwapBuy(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(&crate::grpc::program_ids::PUMPSWAP_PROGRAM)
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
                    pumpswap::fill_buy_accounts(event, &get_account);
                }
            }
        }
        DexEvent::PumpSwapSell(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(&crate::grpc::program_ids::PUMPSWAP_PROGRAM)
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
                    pumpswap::fill_sell_accounts(event, &get_account);
                }
            }
        }
        DexEvent::MeteoraDammV2Swap(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(&crate::grpc::program_ids::METEORA_DAMM_V2_PROGRAM)
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
                    meteora::fill_damm_v2_swap_accounts(event, &get_account);
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
        // PumpFun 事件填充 (所有交易类型共用相同的账户填充逻辑)
        DexEvent::PumpFunTrade(ref mut trade_event)
        | DexEvent::PumpFunBuy(ref mut trade_event)
        | DexEvent::PumpFunSell(ref mut trade_event)
        | DexEvent::PumpFunBuyExactSolIn(ref mut trade_event) => {
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
        DexEvent::MeteoraPoolsSwap(ref mut swap_event) => {}
        DexEvent::MeteoraDammV2Swap(ref mut swap_event) => {
            meteora::fill_damm_v2_swap_accounts(swap_event, &get_account);
        }
        DexEvent::MeteoraDlmmSwap(ref mut swap_event) => {}
        DexEvent::MeteoraDlmmAddLiquidity(ref mut event) => {}
        DexEvent::MeteoraDlmmRemoveLiquidity(ref mut event) => {}

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
        if trade_event.token_program == Pubkey::default() {
            trade_event.token_program = 
                if trade_event.is_buy { get_account(8)} else { get_account(9) }
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

pub mod pumpswap {
    use super::*;
    use crate::core::PumpSwapBuyEvent;

    pub fn fill_buy_accounts(event: &mut PumpSwapBuyEvent, get_account: &AccountGetter<'_>) {
        if event.base_mint == Pubkey::default() {
            event.base_mint = get_account(3);
        }
        if event.quote_mint == Pubkey::default() {
            event.quote_mint = get_account(4);
        }
        if event.pool_base_token_account == Pubkey::default() {
            event.pool_base_token_account = get_account(7);
        }
        if event.pool_quote_token_account == Pubkey::default() {
            event.pool_quote_token_account = get_account(8);
        }
        if event.coin_creator_vault_ata == Pubkey::default() {
            event.coin_creator_vault_ata = get_account(17);
        }
        if event.coin_creator_vault_authority == Pubkey::default() {
            event.coin_creator_vault_authority = get_account(18);
        }
        if event.base_token_program == Pubkey::default() {
            event.base_token_program = get_account(11);
        }
        if event.quote_token_program == Pubkey::default() {
            event.quote_token_program = get_account(12);
        }
    }

    pub fn fill_sell_accounts(event: &mut PumpSwapSellEvent, get_account: &AccountGetter<'_>) {
        if event.base_mint == Pubkey::default() {
            event.base_mint = get_account(3);
        }
        if event.quote_mint == Pubkey::default() {
            event.quote_mint = get_account(4);
        }
        if event.pool_base_token_account == Pubkey::default() {
            event.pool_base_token_account = get_account(7);
        }
        if event.pool_quote_token_account == Pubkey::default() {
            event.pool_quote_token_account = get_account(8);
        }
        if event.coin_creator_vault_ata == Pubkey::default() {
            event.coin_creator_vault_ata = get_account(17);
        }
        if event.coin_creator_vault_authority == Pubkey::default() {
            event.coin_creator_vault_authority = get_account(18);
        }
        if event.base_token_program == Pubkey::default() {
            event.base_token_program = get_account(11);
        }
        if event.quote_token_program == Pubkey::default() {
            event.quote_token_program = get_account(12);
        }
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

    /// 填充 Meteora DAMM V2 Swap 事件账户
    pub fn fill_damm_v2_swap_accounts(
        swap_event: &mut MeteoraDammV2SwapEvent,
        get_account: &AccountGetter<'_>,
    ) {
        if swap_event.token_a_vault == Pubkey::default() {
            swap_event.token_a_vault = get_account(4);
        }
        if swap_event.token_b_vault == Pubkey::default() {
            swap_event.token_b_vault = get_account(5);
        }
        if swap_event.token_a_mint == Pubkey::default() {
            swap_event.token_a_mint = get_account(6);
        }
        if swap_event.token_b_mint == Pubkey::default() {
            swap_event.token_b_mint = get_account(7);
        }
        if swap_event.token_a_program == Pubkey::default() {
            swap_event.token_a_program = get_account(9);
        }
        if swap_event.token_b_program == Pubkey::default() {
            swap_event.token_b_program = get_account(10);
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
