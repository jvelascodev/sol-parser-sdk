pub mod nonce;
pub mod program_ids;
pub mod pumpswap;
pub mod token;
pub mod utils;
use crate::core::events::EventMetadata;
use crate::grpc::EventTypeFilter;
use crate::DexEvent;
pub use nonce::parse_nonce_account;
use program_ids::*;
pub use pumpswap::{
    parse_global_config as parse_pumpswap_global_config, parse_pool as parse_pumpswap_pool,
};
pub use token::parse_token_account;
pub use token::AccountData;
pub use utils::*;

pub fn parse_account_unified(
    account: &AccountData,
    metadata: EventMetadata,
    event_type_filter: Option<&EventTypeFilter>,
) -> Option<DexEvent> {
    if account.data.is_empty() {
        return None;
    }

    // Early filtering based on event type filter
    if let Some(filter) = event_type_filter {
        if let Some(ref include_only) = filter.include_only {
            // Check if any of the account event types are in the include list
            let should_parse = include_only.iter().any(|t| {
                use crate::grpc::EventType;
                matches!(
                    t,
                    EventType::TokenAccount
                        | EventType::NonceAccount
                        | EventType::AccountPumpSwapGlobalConfig
                        | EventType::AccountPumpSwapPool
                )
            });
            if !should_parse {
                return None;
            }
        }
    }

    if account.owner == PUMPSWAP_PROGRAM_ID {
        if let Some(filter) = event_type_filter {
            if filter.should_include(crate::grpc::EventType::AccountPumpSwapGlobalConfig)
                || filter.should_include(crate::grpc::EventType::AccountPumpSwapPool)
            {
                let event = parse_pumpswap_account(account, metadata.clone());
                if event.is_some() {
                    return event;
                }
            }
        }
    }
    if nonce::is_nonce_account(&account.data) {
        // Check filter for NonceAccount specifically
        if let Some(filter) = event_type_filter {
            if !filter.should_include(crate::grpc::EventType::NonceAccount) {
                return None;
            }
        }
        return parse_nonce_account(account, metadata);
    }
    // Parse token account (includes both TokenAccount and TokenInfo)
    if let Some(filter) = event_type_filter {
        let includes_token = filter.should_include(crate::grpc::EventType::TokenAccount);
        if !includes_token {
            return None;
        }
    }
    return parse_token_account(account, metadata);
}

fn parse_pumpswap_account(account: &AccountData, metadata: EventMetadata) -> Option<DexEvent> {
    // 检查 discriminator 以确定账户类型
    if pumpswap::is_global_config_account(&account.data) {
        return pumpswap::parse_global_config(account, metadata);
    }
    if pumpswap::is_pool_account(&account.data) {
        return pumpswap::parse_pool(account, metadata);
    }
    None
}
