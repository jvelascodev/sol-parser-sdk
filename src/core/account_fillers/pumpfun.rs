//! PumpFun 账户填充模块

use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;

/// 账户获取辅助函数类型
pub type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

/// 填充 PumpFun Trade 事件账户
///
/// PumpFun Buy/Sell instruction account mapping (based on IDL):
/// 0: global
/// 1: feeRecipient
/// 2: mint
/// 3: bondingCurve
/// 4: associatedBondingCurve
/// 5: associatedUser
/// 6: user
/// 7: systemProgram
/// 8: tokenProgram (buy) / associatedTokenProgram (sell)
/// 9: rent (buy) / eventAuthority (sell)
/// 10: eventAuthority (buy) / program (sell)
/// 11: program
pub fn fill_trade_accounts(e: &mut PumpFunTradeEvent, get: &AccountGetter<'_>) {
    if e.user == Pubkey::default() {
        e.user = get(6);
    }
    if e.bonding_curve == Pubkey::default() {
        e.bonding_curve = get(3);
    }
    if e.associated_bonding_curve == Pubkey::default() {
        e.associated_bonding_curve = get(4);
    }
    if e.creator_vault == Pubkey::default() {
        e.creator_vault = if e.is_buy { get(9) } else { get(8) };
    }
    if e.token_program == Pubkey::default() {
        e.token_program = if e.is_buy { get(8) } else { get(9) };
    }
}

/// 填充 PumpFun Create 事件账户
///
/// PumpFun Create instruction account mapping (based on IDL):
/// 0: mint
/// 1: mintAuthority
/// 2: bondingCurve
/// 3: associatedBondingCurve
/// 4: global
/// 5: mplTokenMetadata
/// 6: metadata
/// 7: user
/// 8: systemProgram
/// 9: tokenProgram
/// 10: associatedTokenProgram
/// 11: rent
/// 12: eventAuthority
/// 13: program
pub fn fill_create_accounts(e: &mut PumpFunCreateTokenEvent, get: &AccountGetter<'_>) {
    if e.mint == Pubkey::default() {
        e.mint = get(0);
    }
    if e.bonding_curve == Pubkey::default() {
        e.bonding_curve = get(2);
    }
    if e.user == Pubkey::default() {
        e.user = get(7);
    }
}

/// 填充 PumpFun Migrate 事件账户
pub fn fill_migrate_accounts(_e: &mut PumpFunMigrateEvent, _get: &AccountGetter<'_>) {
    // 暂未实现 - 需要 IDL
}
