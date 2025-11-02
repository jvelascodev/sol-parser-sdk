use crate::{core::events::*, instr::read_bool};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{Transaction, TransactionStatusMeta};

pub fn fill_data(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<&str, Vec<(i32, i32)>>,
) {
    // 获取账户的辅助函数
    match event {
        DexEvent::PumpSwapBuy(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::PUMPSWAP_FEES_PROGRAM_ID)
                .as_ref()
                .and_then(|v| v.last())
            {
                let data = get_instruction_data(meta, transaction, invoke);
                if data.is_some() {
                    let is_pump_pool = read_bool(&data.unwrap_or_default(), 9).unwrap_or_default();
                    event.is_pump_pool = is_pump_pool;
                }
            }
        }
        DexEvent::PumpSwapSell(ref mut event) => {
            if let Some(invoke) = program_invokes
                .get(crate::grpc::program_ids::PUMPSWAP_FEES_PROGRAM_ID)
                .as_ref()
                .and_then(|v| v.last())
            {
                let data = get_instruction_data(meta, transaction, invoke);
                if data.is_some() {
                    let is_pump_pool = read_bool(&data.unwrap_or_default(), 9).unwrap_or_default();
                    event.is_pump_pool = is_pump_pool;
                }
            }
        }
        _ => {} // 其他事件类型TODO
    }
}

pub fn get_instruction_data<'a>(
    meta: &'a TransactionStatusMeta,
    transaction: &'a Option<Transaction>,
    index: &(i32, i32), // (outer_index, inner_index)
) -> Option<&'a [u8]> {
    let data = if index.1 >= 0 {
        meta.inner_instructions
            .iter()
            .find(|i| i.index == index.0 as u32)?
            .instructions
            .get(index.1 as usize)?
            .data
            .as_slice()
    } else {
        transaction.as_ref()?.message.as_ref()?.instructions.get(index.0 as usize)?.data.as_slice()
    };
    return Some(data);
}
