//! SPL Token 和 Token-2022 账户解析
//!
//! 提供 Token Account 和 Mint 账户的解析功能

use crate::core::events::{EventMetadata, TokenAccountEvent};
use crate::DexEvent;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::{Account, Mint};
use spl_token_2022::{
    extension::StateWithExtensions,
    state::{Account as Account2022, Mint as Mint2022},
};

/// Account data from gRPC subscription or other sources
#[derive(Clone, Debug)]
pub struct AccountData {
    pub pubkey: Pubkey,
    pub executable: bool,
    pub lamports: u64,
    pub owner: Pubkey,
    pub rent_epoch: u64,
    pub data: Vec<u8>,
}

/// Parse token account or mint from account data
///
/// This function attempts to parse the account as:
/// 1. SPL Token Mint
/// 2. SPL Token-2022 Mint
/// 3. SPL Token Account
/// 4. SPL Token-2022 Account
///
/// # Arguments
/// * `account` - Account data from gRPC
/// * `metadata` - Event metadata (slot, signature, etc.)
///
/// # Returns
/// Returns `Some(DexEvent)` if parsing succeeds, `None` otherwise
pub fn parse_token_account(account: &AccountData, metadata: EventMetadata) -> Option<DexEvent> {
    let pubkey = account.pubkey;
    let executable = account.executable;
    let lamports = account.lamports;
    let owner = account.owner;
    let rent_epoch = account.rent_epoch;

    // Try parsing as SPL Token Mint
    if account.data.len() >= Mint::LEN {
        if let Ok(mint) = Mint::unpack_from_slice(&account.data) {
            let event = TokenAccountEvent {
                metadata,
                pubkey,
                executable,
                lamports,
                owner,
                rent_epoch,
                amount: None,
                token_owner: owner, 
                supply: Some(mint.supply),
                decimals: Some(mint.decimals),
            };
            return Some(DexEvent::TokenAccount(event));
        }
    }

    // Try parsing as SPL Token-2022 Mint
    if account.data.len() >= Account2022::LEN {
        if let Ok(mint) = StateWithExtensions::<Mint2022>::unpack(&account.data) {
            let event = TokenAccountEvent {
                metadata,
                pubkey,
                executable,
                lamports,
                owner,
                rent_epoch,
                amount: None,
                token_owner: owner,
                supply: Some(mint.base.supply),
                decimals: Some(mint.base.decimals),
            };
            return Some(DexEvent::TokenAccount(event));
        }
    }

    // Parse as Token Account (SPL Token or Token-2022)
    let amount = if account.owner.to_bytes() == spl_token_2022::ID.to_bytes() {
        StateWithExtensions::<Account2022>::unpack(&account.data)
            .ok()
            .map(|info| info.base.amount)
    } else {
        Account::unpack(&account.data).ok().map(|info| info.amount)
    };

    let event = TokenAccountEvent {
        metadata,
        pubkey,
        executable,
        lamports,
        owner,
        rent_epoch,
        amount,
        token_owner: account.owner,
        supply: None,
        decimals: None,
    };

    Some(DexEvent::TokenAccount(event))
}

/// Helper function to detect if account is owned by token program
pub fn is_token_program_account(owner: &Pubkey) -> bool {
    owner.to_bytes() == spl_token::ID.to_bytes()
        || owner.to_bytes() == spl_token_2022::ID.to_bytes()
}
