use cosmwasm_std::{
    to_binary, Api, CanonicalAddr, HumanAddr, Querier, QueryRequest, StdError, StdResult, Uint128,
    WasmQuery,
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
    if input.len() != num_of_bytes {
        return Err(StdError::generic_err(format!(
            "Expected slice length to be {}, received length of {}",
            num_of_bytes,
            input.len()
        )));
    };
    let slice_to_array_result = input[0..num_of_bytes].try_into();

    match slice_to_array_result {
        Ok(array) => Ok(u64::from_be_bytes(array)),
        Err(err) => Err(StdError::generic_err(format!(
            "Error converting slice to array: {}",
            err
        ))),
    }
}

/// Converts human addr into canonical addr if present, otherwise use default
pub fn human_addr_into_canonical<A: Api>(
    api: A,
    human_addr: Option<HumanAddr>,
    default: CanonicalAddr,
) -> StdResult<CanonicalAddr> {
    match human_addr {
        Some(human_addr) => api.canonical_address(&human_addr),
        None => Ok(default),
    }
}

/// Verify if all conditions are met. If not return list of invalid params.
pub fn all_conditions_valid(conditions_and_names: Vec<(bool, &str)>) -> StdResult<()> {
    // All params which should meet criteria
    let param_names: Vec<_> = conditions_and_names.iter().map(|elem| elem.1).collect();
    // Filter params which don't meet criteria
    let invalid_params: Vec<_> = conditions_and_names
        .into_iter()
        .filter(|elem| !elem.0)
        .map(|elem| elem.1)
        .collect();
    if !invalid_params.is_empty() {
        return Err(StdError::generic_err(format!(
            "[{}] should be less or equal 1. Invalid params: [{}]",
            param_names.join(", "),
            invalid_params.join(", ")
        )));
    }

    Ok(())
}
