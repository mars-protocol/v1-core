use cosmwasm_std::{to_binary, Uint128, StdResult, QueryRequest, WasmQuery, Querier, Api, Storage, Extern, HumanAddr};
use cw20::{Cw20QueryMsg, BalanceResponse, TokenInfoResponse};

// CW20
pub fn cw20_get_balance<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    token_address: HumanAddr,
    balance_address: HumanAddr
) -> StdResult<Uint128> {
    let query: BalanceResponse = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: token_address,
        msg: to_binary(&Cw20QueryMsg::Balance {
            address: balance_address,
        })?,
    }))?;

    Ok(query.balance)
}

pub fn cw20_get_total_supply<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    token_address: HumanAddr
) -> StdResult<Uint128> {
    let query: TokenInfoResponse = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: token_address,
        msg: to_binary(&Cw20QueryMsg::TokenInfo {})?,
    }))?;

    Ok(query.total_supply)
}
