use crate::state::balances;
use cosmwasm_std::{
    to_binary, Api, CanonicalAddr, CosmosMsg, Extern, HumanAddr, Querier, StdResult, Storage,
    Uint128, WasmMsg,
};

/// Deduct amount form sender balance and deducts it from recipient
/// Returns previous balances
pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    sender_raw: &CanonicalAddr,
    recipient_raw: &CanonicalAddr,
    amount: Uint128,
) -> StdResult<(Uint128, Uint128)> {
    let mut accounts = balances(&mut deps.storage);

    let sender_previous_balance = accounts.load(sender_raw.as_slice()).unwrap_or_default();
    let sender_new_balance = (sender_previous_balance - amount)?;
    accounts.save(sender_raw.as_slice(), &sender_new_balance)?;

    let recipient_previous_balance = accounts.load(recipient_raw.as_slice()).unwrap_or_default();
    let recipient_new_balance = recipient_previous_balance + amount;
    accounts.save(recipient_raw.as_slice(), &recipient_new_balance)?;

    Ok((sender_previous_balance, recipient_previous_balance))
}

pub fn finalize_transfer_msg<A: Api>(
    api: &A,
    money_market_address: &CanonicalAddr,
    sender_address: HumanAddr,
    recipient_address: HumanAddr,
    sender_previous_balance: Uint128,
    recipient_previous_balance: Uint128,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: api.human_address(money_market_address)?,
        msg: to_binary(
            &mars::liquidity_pool::msg::HandleMsg::FinalizeLiquidityTokenTransfer {
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
