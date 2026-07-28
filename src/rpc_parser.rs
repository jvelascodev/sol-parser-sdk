//! RPC Transaction Parser
//!
//! 提供独立的 RPC 交易解析功能，不依赖 gRPC streaming
//! 可以用于测试验证和离线分析

use crate::core::events::DexEvent;
use crate::grpc::instruction_parser::parse_instructions_enhanced;
use crate::grpc::types::EventTypeFilter;
use crate::instr::read_pubkey_fast;
use base64::{engine::general_purpose, Engine as _};
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiTransactionEncoding,
};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, InnerInstruction, InnerInstructions, Message, MessageAddressTableLookup,
    MessageHeader, SubscribeUpdateTransaction, SubscribeUpdateTransactionInfo, Transaction,
    TransactionStatusMeta,
};

/// Parse a transaction from RPC by signature
///
/// # Arguments
/// * `rpc_client` - RPC client to fetch the transaction
/// * `signature` - Transaction signature
/// * `filter` - Optional event type filter
///
/// # Returns
/// Vector of parsed DEX events
///
/// # Example
/// ```no_run
/// use solana_client::rpc_client::RpcClient;
/// use solana_sdk::signature::Signature;
/// use sol_parser_sdk::parse_transaction_from_rpc;
/// use std::str::FromStr;
///
/// let client = RpcClient::new("https://api.mainnet-beta.solana.com".to_string());
/// let sig = Signature::from_str("your-signature-here").unwrap();
/// let events = parse_transaction_from_rpc(&client, &sig, None).unwrap();
/// ```
pub fn parse_transaction_from_rpc(
    rpc_client: &RpcClient,
    signature: &Signature,
    filter: Option<&EventTypeFilter>,
) -> Result<Vec<DexEvent>, ParseError> {
    // Fetch transaction from RPC with V0 transaction support
    let config = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::Base64),
        commitment: None,
        max_supported_transaction_version: Some(0),
    };

    let rpc_tx = rpc_client.get_transaction_with_config(signature, config).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("429") || msg.contains("Too Many Requests") {
            ParseError::RateLimited(msg)
        } else {
            ParseError::RpcError(msg)
        }
    })?;

    parse_rpc_transaction(&rpc_tx, filter)
}

/// Parse a RPC transaction structure
///
/// # Arguments
/// * `rpc_tx` - RPC transaction to parse
/// * `filter` - Optional event type filter
///
/// # Returns
/// Vector of parsed DEX events
///
/// # Example
/// ```no_run
/// use sol_parser_sdk::parse_rpc_transaction;
///
/// // Assuming you have an rpc_tx from RPC
/// // let events = parse_rpc_transaction(&rpc_tx, None).unwrap();
/// ```
pub fn parse_rpc_transaction(
    rpc_tx: &EncodedConfirmedTransactionWithStatusMeta,
    filter: Option<&EventTypeFilter>,
) -> Result<Vec<DexEvent>, ParseError> {
    // Convert RPC format to gRPC format
    let (grpc_meta, grpc_tx) = convert_rpc_to_grpc(rpc_tx)?;

    // Extract metadata
    let signature = extract_signature(rpc_tx)?;
    let slot = rpc_tx.slot;
    let block_time_us = rpc_tx.block_time.map(|t| t * 1_000_000);
    let grpc_recv_us =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros()
            as i64;

    // Wrap grpc_tx in Option for reuse
    let grpc_tx_opt = Some(grpc_tx);

    // Build program_invokes HashMap for account filling
    // Use string keys to match gRPC parsing logic
    let mut program_invokes: HashMap<&str, Vec<(i32, i32)>> = HashMap::new();

    if let Some(ref tx) = grpc_tx_opt {
        if let Some(ref msg) = tx.message {
            // Build account key lookup
            let keys_len = msg.account_keys.len();
            let writable_len = grpc_meta.loaded_writable_addresses.len();
            let get_key = |i: usize| -> Option<&Vec<u8>> {
                if i < keys_len {
                    msg.account_keys.get(i)
                } else if i < keys_len + writable_len {
                    grpc_meta.loaded_writable_addresses.get(i - keys_len)
                } else {
                    grpc_meta.loaded_readonly_addresses.get(i - keys_len - writable_len)
                }
            };

            // Record outer instructions
            for (i, ix) in msg.instructions.iter().enumerate() {
                let pid = get_key(ix.program_id_index as usize)
                    .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                let pid_str = pid.to_string();
                let pid_static: &'static str = pid_str.leak();
                program_invokes.entry(pid_static).or_default().push((i as i32, -1));
            }

            // Record inner instructions
            for inner in &grpc_meta.inner_instructions {
                let outer_idx = inner.index as usize;
                for (j, inner_ix) in inner.instructions.iter().enumerate() {
                    let pid = get_key(inner_ix.program_id_index as usize)
                        .map_or(Pubkey::default(), |k| read_pubkey_fast(k));
                    let pid_str = pid.to_string();
                    let pid_static: &'static str = pid_str.leak();
                    program_invokes
                        .entry(pid_static)
                        .or_default()
                        .push((outer_idx as i32, j as i32));
                }
            }
        }
    }

    // Parse instructions
    let mut events = parse_instructions_enhanced(
        &grpc_meta,
        &grpc_tx_opt,
        signature,
        slot,
        0, // tx_idx
        block_time_us,
        grpc_recv_us,
        filter,
    );

    // Parse logs (for protocols like PumpFun that emit events in logs)
    let mut is_created_buy = false;

    for log in &grpc_meta.log_messages {
        if let Some(mut event) = crate::logs::parse_log(
            log,
            signature,
            slot,
            0, // tx_index
            block_time_us,
            grpc_recv_us,
            filter,
            is_created_buy,
        ) {
            // Check if this is a PumpFun create event to set is_created_buy flag
            if matches!(event, DexEvent::PumpFunCreate(_)) {
                is_created_buy = true;
            }

            // Fill account fields - use same function as gRPC parsing
            crate::core::account_dispatcher::fill_accounts_from_transaction_data(
                &mut event,
                &grpc_meta,
                &grpc_tx_opt,
                &program_invokes,
            );

            // Fill additional data fields (e.g., PumpSwap is_pump_pool)
            crate::core::common_filler::fill_data(
                &mut event,
                &grpc_meta,
                &grpc_tx_opt,
                &program_invokes,
            );

            events.push(event);
        }
    }

    Ok(events)
}

/// Parse a decoded transaction with caller-supplied historical metadata.
pub fn parse_native_transaction(
    transaction: &VersionedTransaction,
    meta: &solana_transaction_status::TransactionStatusMeta,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
    received_at_us: i64,
    stream_epoch: u64,
    filter: Option<&EventTypeFilter>,
) -> Result<Vec<DexEvent>, ParseError> {
    if meta.status.is_err() {
        return Ok(Vec::new());
    }
    let update = native_transaction_update(transaction, meta, slot, tx_index)?;
    let mut events =
        crate::grpc::client::parse_transaction_core(&update, received_at_us, block_time_us, filter);
    for event in &mut events {
        event.set_stream_epoch(stream_epoch);
    }
    Ok(events)
}

/// Parse error types
#[derive(Debug)]
pub enum ParseError {
    RpcError(String),
    RateLimited(String),
    ConversionError(String),
    MissingField(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::RpcError(msg) => write!(f, "RPC error: {}", msg),
            ParseError::RateLimited(msg) => write!(f, "Rate limited: {}", msg),
            ParseError::ConversionError(msg) => write!(f, "Conversion error: {}", msg),
            ParseError::MissingField(msg) => write!(f, "Missing field: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

// ============================================================================
// Internal conversion functions
// ============================================================================

fn extract_signature(
    rpc_tx: &EncodedConfirmedTransactionWithStatusMeta,
) -> Result<Signature, ParseError> {
    let ui_tx = &rpc_tx.transaction.transaction;

    match ui_tx {
        EncodedTransaction::Binary(data, _encoding) => {
            let bytes = general_purpose::STANDARD.decode(data).map_err(|e| {
                ParseError::ConversionError(format!("Failed to decode base64: {}", e))
            })?;

            let versioned_tx: solana_sdk::transaction::VersionedTransaction =
                bincode::deserialize(&bytes).map_err(|e| {
                    ParseError::ConversionError(format!("Failed to deserialize transaction: {}", e))
                })?;

            Ok(versioned_tx.signatures[0])
        }
        _ => Err(ParseError::ConversionError("Unsupported transaction encoding".to_string())),
    }
}

pub fn convert_rpc_to_grpc(
    rpc_tx: &EncodedConfirmedTransactionWithStatusMeta,
) -> Result<(TransactionStatusMeta, Transaction), ParseError> {
    let rpc_meta = rpc_tx
        .transaction
        .meta
        .as_ref()
        .ok_or_else(|| ParseError::MissingField("meta".to_string()))?;

    // Convert meta
    let mut grpc_meta = TransactionStatusMeta {
        err: None,
        fee: rpc_meta.fee,
        pre_balances: rpc_meta.pre_balances.clone(),
        post_balances: rpc_meta.post_balances.clone(),
        inner_instructions: Vec::new(),
        log_messages: {
            let opt: Option<Vec<String>> = rpc_meta.log_messages.clone().into();
            opt.unwrap_or_default()
        },
        pre_token_balances: Vec::new(),
        post_token_balances: Vec::new(),
        rewards: Vec::new(),
        loaded_writable_addresses: {
            let loaded_opt: Option<solana_transaction_status::UiLoadedAddresses> =
                rpc_meta.loaded_addresses.clone().into();
            loaded_opt
                .map(|addrs| {
                    addrs
                        .writable
                        .iter()
                        .map(|pk_str| {
                            use std::str::FromStr;
                            solana_sdk::pubkey::Pubkey::from_str(pk_str)
                                .unwrap()
                                .to_bytes()
                                .to_vec()
                        })
                        .collect()
                })
                .unwrap_or_default()
        },
        loaded_readonly_addresses: {
            let loaded_opt: Option<solana_transaction_status::UiLoadedAddresses> =
                rpc_meta.loaded_addresses.clone().into();
            loaded_opt
                .map(|addrs| {
                    addrs
                        .readonly
                        .iter()
                        .map(|pk_str| {
                            use std::str::FromStr;
                            solana_sdk::pubkey::Pubkey::from_str(pk_str)
                                .unwrap()
                                .to_bytes()
                                .to_vec()
                        })
                        .collect()
                })
                .unwrap_or_default()
        },
        return_data: None,
        compute_units_consumed: rpc_meta.compute_units_consumed.clone().into(),
        cost_units: None,
        inner_instructions_none: {
            let opt: Option<Vec<_>> = rpc_meta.inner_instructions.clone().into();
            opt.is_none()
        },
        log_messages_none: {
            let opt: Option<Vec<String>> = rpc_meta.log_messages.clone().into();
            opt.is_none()
        },
        return_data_none: {
            let opt: Option<solana_transaction_status::UiTransactionReturnData> =
                rpc_meta.return_data.clone().into();
            opt.is_none()
        },
    };

    // Convert inner instructions
    let inner_instructions_opt: Option<Vec<_>> = rpc_meta.inner_instructions.clone().into();
    if let Some(ref inner_instructions) = inner_instructions_opt {
        for inner in inner_instructions {
            let mut grpc_inner =
                InnerInstructions { index: inner.index as u32, instructions: Vec::new() };

            for ix in &inner.instructions {
                if let solana_transaction_status::UiInstruction::Compiled(compiled) = ix {
                    // Decode base58 data
                    let data = bs58::decode(&compiled.data).into_vec().map_err(|e| {
                        ParseError::ConversionError(format!(
                            "Failed to decode instruction data: {}",
                            e
                        ))
                    })?;

                    grpc_inner.instructions.push(InnerInstruction {
                        program_id_index: compiled.program_id_index as u32,
                        accounts: compiled.accounts.clone(),
                        data,
                        stack_height: compiled.stack_height.map(|h| h as u32),
                    });
                }
            }

            grpc_meta.inner_instructions.push(grpc_inner);
        }
    }

    // Convert transaction
    let ui_tx = &rpc_tx.transaction.transaction;

    let (message, signatures) = match ui_tx {
        EncodedTransaction::Binary(data, _encoding) => {
            // Decode base64
            let bytes = general_purpose::STANDARD.decode(data).map_err(|e| {
                ParseError::ConversionError(format!("Failed to decode base64: {}", e))
            })?;

            // Parse as versioned transaction
            let versioned_tx: solana_sdk::transaction::VersionedTransaction =
                bincode::deserialize(&bytes).map_err(|e| {
                    ParseError::ConversionError(format!("Failed to deserialize transaction: {}", e))
                })?;

            let grpc_tx = convert_native_transaction(&versioned_tx)?;
            let message =
                grpc_tx.message.ok_or_else(|| ParseError::MissingField("message".to_string()))?;
            (message, grpc_tx.signatures)
        }
        EncodedTransaction::Json(_) => {
            return Err(ParseError::ConversionError(
                "JSON encoded transactions not supported yet".to_string(),
            ));
        }
        _ => {
            return Err(ParseError::ConversionError(
                "Unsupported transaction encoding".to_string(),
            ));
        }
    };

    let grpc_tx = Transaction { signatures, message: Some(message) };

    Ok((grpc_meta, grpc_tx))
}

fn native_transaction_update(
    transaction: &VersionedTransaction,
    meta: &solana_transaction_status::TransactionStatusMeta,
    slot: u64,
    tx_index: u64,
) -> Result<SubscribeUpdateTransaction, ParseError> {
    let signature = transaction
        .signatures
        .first()
        .ok_or_else(|| ParseError::MissingField("signature".to_string()))?
        .as_ref()
        .to_vec();

    Ok(SubscribeUpdateTransaction {
        transaction: Some(SubscribeUpdateTransactionInfo {
            signature,
            is_vote: false,
            transaction: Some(convert_native_transaction(transaction)?),
            meta: Some(convert_native_meta(meta)),
            index: tx_index,
        }),
        slot,
    })
}

fn convert_native_meta(
    meta: &solana_transaction_status::TransactionStatusMeta,
) -> TransactionStatusMeta {
    let inner_instructions_none = meta.inner_instructions.is_none();
    let log_messages_none = meta.log_messages.is_none();
    let return_data_none = meta.return_data.is_none();
    TransactionStatusMeta {
        err: None,
        fee: meta.fee,
        pre_balances: meta.pre_balances.clone(),
        post_balances: meta.post_balances.clone(),
        inner_instructions: meta
            .inner_instructions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|inner| InnerInstructions {
                index: inner.index as u32,
                instructions: inner
                    .instructions
                    .iter()
                    .map(|instruction| InnerInstruction {
                        program_id_index: instruction.instruction.program_id_index as u32,
                        accounts: instruction.instruction.accounts.clone(),
                        data: instruction.instruction.data.clone(),
                        stack_height: instruction.stack_height,
                    })
                    .collect(),
            })
            .collect(),
        inner_instructions_none,
        log_messages: meta.log_messages.clone().unwrap_or_default(),
        log_messages_none,
        pre_token_balances: native_token_balances(meta.pre_token_balances.as_deref()),
        post_token_balances: native_token_balances(meta.post_token_balances.as_deref()),
        rewards: meta
            .rewards
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|reward| yellowstone_grpc_proto::prelude::Reward {
                pubkey: reward.pubkey.clone(),
                lamports: reward.lamports,
                post_balance: reward.post_balance,
                reward_type: match reward.reward_type {
                    None => yellowstone_grpc_proto::prelude::RewardType::Unspecified,
                    Some(solana_transaction_status::RewardType::Fee) => {
                        yellowstone_grpc_proto::prelude::RewardType::Fee
                    }
                    Some(solana_transaction_status::RewardType::Rent) => {
                        yellowstone_grpc_proto::prelude::RewardType::Rent
                    }
                    Some(solana_transaction_status::RewardType::Staking) => {
                        yellowstone_grpc_proto::prelude::RewardType::Staking
                    }
                    Some(solana_transaction_status::RewardType::Voting) => {
                        yellowstone_grpc_proto::prelude::RewardType::Voting
                    }
                } as i32,
                commission: reward.commission.map_or_else(String::new, |value| value.to_string()),
            })
            .collect(),
        loaded_writable_addresses: meta
            .loaded_addresses
            .writable
            .iter()
            .map(|address| address.to_bytes().to_vec())
            .collect(),
        loaded_readonly_addresses: meta
            .loaded_addresses
            .readonly
            .iter()
            .map(|address| address.to_bytes().to_vec())
            .collect(),
        return_data: meta.return_data.as_ref().map(|data| {
            yellowstone_grpc_proto::prelude::ReturnData {
                program_id: data.program_id.to_bytes().to_vec(),
                data: data.data.clone(),
            }
        }),
        return_data_none,
        compute_units_consumed: meta.compute_units_consumed,
        cost_units: meta.cost_units,
    }
}

fn native_token_balances(
    balances: Option<&[solana_transaction_status::TransactionTokenBalance]>,
) -> Vec<yellowstone_grpc_proto::prelude::TokenBalance> {
    balances
        .unwrap_or_default()
        .iter()
        .map(|balance| yellowstone_grpc_proto::prelude::TokenBalance {
            account_index: balance.account_index as u32,
            mint: balance.mint.clone(),
            ui_token_amount: Some(yellowstone_grpc_proto::prelude::UiTokenAmount {
                ui_amount: balance.ui_token_amount.ui_amount.unwrap_or_default(),
                decimals: balance.ui_token_amount.decimals as u32,
                amount: balance.ui_token_amount.amount.clone(),
                ui_amount_string: balance.ui_token_amount.ui_amount_string.clone(),
            }),
            owner: balance.owner.clone(),
            program_id: balance.program_id.clone(),
        })
        .collect()
}

fn convert_native_transaction(
    transaction: &VersionedTransaction,
) -> Result<Transaction, ParseError> {
    let message = match &transaction.message {
        solana_sdk::message::VersionedMessage::Legacy(message) => convert_legacy_message(message)?,
        solana_sdk::message::VersionedMessage::V0(message) => convert_v0_message(message)?,
    };
    Ok(Transaction {
        signatures: transaction
            .signatures
            .iter()
            .map(|signature| signature.as_ref().to_vec())
            .collect(),
        message: Some(message),
    })
}

fn convert_legacy_message(
    msg: &solana_sdk::message::legacy::Message,
) -> Result<Message, ParseError> {
    let account_keys: Vec<Vec<u8>> =
        msg.account_keys.iter().map(|k| k.to_bytes().to_vec()).collect();

    let instructions: Vec<CompiledInstruction> = msg
        .instructions
        .iter()
        .map(|ix| CompiledInstruction {
            program_id_index: ix.program_id_index as u32,
            accounts: ix.accounts.clone(),
            data: ix.data.clone(),
        })
        .collect();

    Ok(Message {
        header: Some(MessageHeader {
            num_required_signatures: msg.header.num_required_signatures as u32,
            num_readonly_signed_accounts: msg.header.num_readonly_signed_accounts as u32,
            num_readonly_unsigned_accounts: msg.header.num_readonly_unsigned_accounts as u32,
        }),
        account_keys,
        recent_blockhash: msg.recent_blockhash.to_bytes().to_vec(),
        instructions,
        versioned: false,
        address_table_lookups: Vec::new(),
    })
}

fn convert_v0_message(msg: &solana_sdk::message::v0::Message) -> Result<Message, ParseError> {
    let account_keys: Vec<Vec<u8>> =
        msg.account_keys.iter().map(|k| k.to_bytes().to_vec()).collect();

    let instructions: Vec<CompiledInstruction> = msg
        .instructions
        .iter()
        .map(|ix| CompiledInstruction {
            program_id_index: ix.program_id_index as u32,
            accounts: ix.accounts.clone(),
            data: ix.data.clone(),
        })
        .collect();

    Ok(Message {
        header: Some(MessageHeader {
            num_required_signatures: msg.header.num_required_signatures as u32,
            num_readonly_signed_accounts: msg.header.num_readonly_signed_accounts as u32,
            num_readonly_unsigned_accounts: msg.header.num_readonly_unsigned_accounts as u32,
        }),
        account_keys,
        recent_blockhash: msg.recent_blockhash.to_bytes().to_vec(),
        instructions,
        versioned: true,
        address_table_lookups: msg
            .address_table_lookups
            .iter()
            .map(|lookup| MessageAddressTableLookup {
                account_key: lookup.account_key.to_bytes().to_vec(),
                writable_indexes: lookup.writable_indexes.clone(),
                readonly_indexes: lookup.readonly_indexes.clone(),
            })
            .collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{
        hash::Hash,
        message::{
            compiled_instruction::CompiledInstruction as NativeCompiledInstruction,
            legacy::Message as LegacyMessage,
            v0::{
                LoadedAddresses, Message as V0Message,
                MessageAddressTableLookup as NativeAddressTableLookup,
            },
            MessageHeader as NativeMessageHeader, VersionedMessage,
        },
    };

    #[test]
    fn test_parse_error_display() {
        let err = ParseError::RateLimited("HTTP 429".to_string());
        assert_eq!(err.to_string(), "Rate limited: HTTP 429");

        let err = ParseError::RpcError("Network error".to_string());
        assert_eq!(err.to_string(), "RPC error: Network error");
    }

    #[test]
    fn test_error_mapping_logic() {
        let test_msgs = vec![
            ("429 Too Many Requests", true),
            ("error: 429", true),
            ("Too Many Requests from server", true),
            ("Connection refused", false),
            ("Parse error", false),
        ];

        for (msg, should_be_rate_limited) in test_msgs {
            let is_rate_limited = msg.contains("429") || msg.contains("Too Many Requests");
            assert_eq!(is_rate_limited, should_be_rate_limited, "Failed for message: {}", msg);
        }
    }

    #[test]
    fn native_transaction_matches_live_core_and_preserves_metadata() {
        let transaction = VersionedTransaction {
            signatures: vec![Signature::from([7; 64])],
            message: VersionedMessage::Legacy(LegacyMessage::default()),
        };
        let mut event_data =
            crate::logs::pump::discriminators::MIGRATE_EVENT.to_le_bytes().to_vec();
        event_data.resize(8 + 160, 0);
        let meta = solana_transaction_status::TransactionStatusMeta {
            log_messages: Some(vec![
                format!("Program {} invoke [1]", crate::grpc::program_ids::PUMPFUN_PROGRAM_ID),
                format!("Program data: {}", general_purpose::STANDARD.encode(event_data)),
            ]),
            ..Default::default()
        };
        let update = native_transaction_update(&transaction, &meta, 42, 9).unwrap();
        let mut live =
            crate::grpc::client::parse_transaction_core(&update, 123_456, Some(987_000), None);
        for event in &mut live {
            event.set_stream_epoch(3);
        }

        let native =
            parse_native_transaction(&transaction, &meta, 42, 9, Some(987_000), 123_456, 3, None)
                .unwrap();

        assert_eq!(serde_json::to_value(&native).unwrap(), serde_json::to_value(&live).unwrap());
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].metadata().slot, 42);
        assert_eq!(native[0].metadata().tx_index, 9);
        assert_eq!(native[0].metadata().event_ordinal, 1);
        assert_eq!(native[0].metadata().stream_epoch, 3);
        assert_eq!(native[0].metadata().block_time_us, 987_000);
        assert_eq!(native[0].metadata().grpc_recv_us, 123_456);

        let filter =
            EventTypeFilter::include_only(vec![crate::grpc::types::EventType::PumpSwapBuy]);
        assert!(parse_native_transaction(
            &transaction,
            &meta,
            42,
            9,
            Some(987_000),
            123_456,
            3,
            Some(&filter),
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn native_transaction_requires_a_signature() {
        let transaction = VersionedTransaction {
            signatures: Vec::new(),
            message: VersionedMessage::Legacy(LegacyMessage::default()),
        };

        let error = parse_native_transaction(
            &transaction,
            &Default::default(),
            42,
            9,
            None,
            123_456,
            3,
            None,
        )
        .unwrap_err();

        assert!(matches!(error, ParseError::MissingField(field) if field == "signature"));
    }

    #[test]
    fn native_transaction_ignores_failed_transactions() {
        let transaction = VersionedTransaction {
            signatures: vec![Signature::from([7; 64])],
            message: VersionedMessage::Legacy(LegacyMessage::default()),
        };
        let meta = solana_transaction_status::TransactionStatusMeta {
            status: Err(solana_sdk::transaction::TransactionError::AccountNotFound),
            log_messages: Some(vec![format!(
                "Program {} invoke [1]",
                crate::grpc::program_ids::PUMPFUN_PROGRAM_ID
            )]),
            ..Default::default()
        };

        assert!(parse_native_transaction(
            &transaction,
            &meta,
            42,
            9,
            Some(987_000),
            123_456,
            3,
            None,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn native_v0_conversion_preserves_loaded_accounts_and_instruction_order() {
        let lookup_key = Pubkey::new_unique();
        let writable = Pubkey::new_unique();
        let readonly = Pubkey::new_unique();
        let transaction = VersionedTransaction {
            signatures: vec![Signature::from([7; 64])],
            message: VersionedMessage::V0(V0Message {
                header: NativeMessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys: vec![Pubkey::new_unique()],
                recent_blockhash: Hash::new_unique(),
                instructions: vec![
                    NativeCompiledInstruction {
                        program_id_index: 0,
                        accounts: vec![1],
                        data: vec![10],
                    },
                    NativeCompiledInstruction {
                        program_id_index: 0,
                        accounts: vec![2],
                        data: vec![20],
                    },
                ],
                address_table_lookups: vec![NativeAddressTableLookup {
                    account_key: lookup_key,
                    writable_indexes: vec![3],
                    readonly_indexes: vec![4],
                }],
            }),
        };
        let meta = solana_transaction_status::TransactionStatusMeta {
            inner_instructions: Some(vec![solana_transaction_status::InnerInstructions {
                index: 1,
                instructions: vec![solana_transaction_status::InnerInstruction {
                    instruction: NativeCompiledInstruction {
                        program_id_index: 2,
                        accounts: vec![3, 4],
                        data: vec![30],
                    },
                    stack_height: Some(2),
                }],
            }]),
            log_messages: Some(vec!["first".to_owned(), "second".to_owned()]),
            loaded_addresses: LoadedAddresses {
                writable: vec![writable],
                readonly: vec![readonly],
            },
            ..Default::default()
        };

        let update = native_transaction_update(&transaction, &meta, 42, 9).unwrap();
        let info = update.transaction.unwrap();
        let grpc_transaction = info.transaction.unwrap();
        let grpc_message = grpc_transaction.message.unwrap();
        let grpc_meta = info.meta.unwrap();

        assert_eq!(info.index, 9);
        assert_eq!(info.signature, transaction.signatures[0].as_ref());
        assert!(grpc_message.versioned);
        assert_eq!(
            grpc_message
                .instructions
                .iter()
                .map(|instruction| instruction.data.clone())
                .collect::<Vec<_>>(),
            [vec![10], vec![20]]
        );
        assert_eq!(
            grpc_message.address_table_lookups[0].account_key,
            lookup_key.to_bytes().to_vec()
        );
        assert_eq!(grpc_message.address_table_lookups[0].writable_indexes, [3]);
        assert_eq!(grpc_message.address_table_lookups[0].readonly_indexes, [4]);
        assert_eq!(grpc_meta.loaded_writable_addresses, [writable.to_bytes().to_vec()]);
        assert_eq!(grpc_meta.loaded_readonly_addresses, [readonly.to_bytes().to_vec()]);
        assert_eq!(grpc_meta.inner_instructions[0].index, 1);
        assert_eq!(grpc_meta.inner_instructions[0].instructions[0].data, [30]);
        assert_eq!(grpc_meta.log_messages, ["first".to_owned(), "second".to_owned()]);
    }
}
