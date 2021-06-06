use cosmwasm_std::{
    to_binary, HumanAddr, Querier, QueryRequest, StdError, StdResult, Uint128, WasmQuery,
};
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};
use std::convert::TryInto;

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

pub fn read_be_u64(input: &[u8]) -> StdResult<u64> {
    let num_of_bytes = std::mem::size_of::<u64>();
    if input.len() != 8 {
        return Err(StdError::generic_err(format!(
            "Expected slice length to be {}, received length of {}",
            num_of_bytes,
            input.len()
        )));
    };
    let slice_to_array_result = input[0..num_of_bytes].try_into();

    match slice_to_array_result {
        Ok(array) => Ok(u64::from_be_bytes(array)),
        Err(_) => Err(StdError::generic_err("Error converting slice to array")),
    }
}
