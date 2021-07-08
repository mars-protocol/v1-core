use cosmwasm_std::{
    to_binary, Api, CanonicalAddr, CosmosMsg, Extern, HumanAddr, Querier, StdResult, Storage,
    Uint128, WasmMsg,
};

use crate::state;
use crate::state::{balances, Config};

/// Deduct amount form sender balance and deducts it from recipient
/// Returns messages to be sent on the final response
pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    config: &Config,
    sender_address: &HumanAddr,
    recipient_address: &HumanAddr,
    amount: Uint128,
    finalize_on_red_bank: bool,
) -> StdResult<Vec<CosmosMsg>> {
    let mut accounts = balances(&mut deps.storage);

    let sender_raw = deps.api.canonical_address(sender_address)?;
    let recipient_raw = deps.api.canonical_address(recipient_address)?;

    let sender_previous_balance = accounts.load(sender_raw.as_slice()).unwrap_or_default();
    let sender_new_balance = (sender_previous_balance - amount)?;
    accounts.save(sender_raw.as_slice(), &sender_new_balance)?;

    let recipient_previous_balance = accounts.load(recipient_raw.as_slice()).unwrap_or_default();
    let recipient_new_balance = recipient_previous_balance + amount;
    accounts.save(recipient_raw.as_slice(), &recipient_new_balance)?;

    let total_supply = state::token_info_read(&deps.storage).load()?.total_supply;

    let mut messages = vec![];

    // If the transfer results from a method called on the money market,
    // it is finalized there. Else it needs to update state and perform some validations
    // to ensure the transfer can be executed
    if finalize_on_red_bank {
        messages.push(finalize_transfer_msg(
            &deps.api,
            &config.red_bank_address,
            sender_address.clone(),
            recipient_address.clone(),
            sender_previous_balance,
            recipient_previous_balance,
            amount,
        )?);
    }

    // Build incentive messages
    messages.push(balance_change_msg(
        &deps.api,
        &config.incentives_address,
        sender_address.clone(),
        sender_previous_balance,
        total_supply,
    )?);
    messages.push(balance_change_msg(
        &deps.api,
        &config.incentives_address,
        recipient_address.clone(),
        recipient_previous_balance,
        total_supply,
    )?);

    Ok(messages)
}

pub fn finalize_transfer_msg<A: Api>(
    api: &A,
    red_bank_canonical_address: &CanonicalAddr,
    sender_address: HumanAddr,
    recipient_address: HumanAddr,
    sender_previous_balance: Uint128,
    recipient_previous_balance: Uint128,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: api.human_address(red_bank_canonical_address)?,
        msg: to_binary(
            &mars::red_bank::msg::HandleMsg::FinalizeLiquidityTokenTransfer {
                sender_address,
                recipient_address,
                sender_previous_balance,
                recipient_previous_balance,
                amount,
            },
        )?,
        send: vec![],
    }))
}

pub fn balance_change_msg<A: Api>(
    api: &A,
    incentives_canonical_address: &CanonicalAddr,
    user_address: HumanAddr,
    user_balance_before: Uint128,
    total_supply_before: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: api.human_address(incentives_canonical_address)?,
        msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
            user_address,
            user_balance_before,
            total_supply_before,
        })?,
        send: vec![],
    }))
}
