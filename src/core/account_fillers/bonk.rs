//! Bonk 账户填充模块

use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;

pub type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

/// 填充 Bonk Trade 事件账户
///
/// Trade instruction account mapping (based on IDL):
/// 0: user
/// 1: poolState
/// 2: userTokenAccount
/// 3: poolTokenAccount
pub fn fill_trade_accounts(e: &mut BonkTradeEvent, get: &AccountGetter<'_>) {
    if e.user == Pubkey::default() {
        e.user = get(0);
    }
    if e.pool_state == Pubkey::default() {
        e.pool_state = get(1);
    }
}

/// Bonk Pool Create 账户填充
///
/// createPool instruction account mapping (based on IDL):
/// 0: state
/// 1: pool
/// 2: tokenX
/// 3: tokenY
/// 4: poolXAccount
/// 5: poolYAccount
/// 6: adminXAccount
/// 7: adminYAccount
/// 8: admin
/// 9: projectOwner
/// 10: programAuthority
/// 11: systemProgram
/// 12: tokenProgram
/// 13: rent
pub fn fill_pool_create_accounts(e: &mut BonkPoolCreateEvent, get: &AccountGetter<'_>) {
    if e.pool_state == Pubkey::default() {
        e.pool_state = get(1); // pool
    }
    if e.creator == Pubkey::default() {
        e.creator = get(8); // admin
    }
    // base_mint_param 已从事件数据或其他来源解析
}
