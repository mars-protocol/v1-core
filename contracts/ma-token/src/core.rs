use crate::state::balances;
use cosmwasm_std::{
    to_binary, Api, CanonicalAddr, CosmosMsg, Extern, HumanAddr, Querier, StdResult, Storage,
    Uint128, WasmMsg,
};

/// Deduct amount form sender balance and deducts it from recipient
/// Returns previous balances
pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    from_raw: &CanonicalAddr,
    to_raw: &CanonicalAddr,
    amount: Uint128,
) -> StdResult<(Uint128, Uint128)> {
    let mut accounts = balances(&mut deps.storage);

    let from_previous_balance = accounts.load(from_raw.as_slice()).unwrap_or_default();
    let from_new_balance = (from_previous_balance - amount)?;
    accounts.save(from_raw.as_slice(), &from_new_balance)?;

    let to_previous_balance = accounts.load(to_raw.as_slice()).unwrap_or_default();
    let to_new_balance = to_previous_balance + amount;
    accounts.save(to_raw.as_slice(), &to_new_balance)?;

    Ok((from_previous_balance, to_previous_balance))
}

pub fn finalize_transfer_msg<A: Api>(
    api: &A,
    money_market_address: &CanonicalAddr,
    from_address: HumanAddr,
    to_address: HumanAddr,
    from_previous_balance: Uint128,
    to_previous_balance: Uint128,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: api.human_address(money_market_address)?,
        msg: to_binary(
            &mars::liquidity_pool::msg::HandleMsg::FinalizeLiquidityTokenTransfer {
                from_address,
                to_address,
                from_previous_balance,
                to_previous_balance,
                amount,
            },
        )?,
        send: vec![],
    }))
}
