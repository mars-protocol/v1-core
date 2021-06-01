use cosmwasm_std::{to_binary, HumanAddr, Querier, QueryRequest, StdResult, Uint128, WasmQuery};
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};

// CW20
pub fn cw20_get_balance<Q: Querier>(
    querier: &Q,
    token_address: HumanAddr,
    balance_address: HumanAddr,
) -> StdResult<Uint128> {
    let query: BalanceResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: token_address,
        msg: to_binary(&Cw20QueryMsg::Balance {
            address: balance_address,
        })?,
    }))?;

    Ok(query.balance)
}

pub fn cw20_get_total_supply<Q: Querier>(
    querier: &Q,
    token_address: HumanAddr,
) -> StdResult<Uint128> {
    let query: TokenInfoResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: token_address,
        msg: to_binary(&Cw20QueryMsg::TokenInfo {})?,
    }))?;

    Ok(query.total_supply)
}

pub fn cw20_get_symbol<Q: Querier>(querier: &Q, token_address: HumanAddr) -> StdResult<String> {
    let query: TokenInfoResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: token_address,
        msg: to_binary(&Cw20QueryMsg::TokenInfo {})?,
    }))?;

    Ok(query.symbol)
}
