use cosmwasm_std::{
    to_binary, Api, CanonicalAddr, Coin, CosmosMsg, Decimal, Empty, Extern, HumanAddr, Querier,
    QueryRequest, StdError, StdResult, Storage, Uint128, WasmMsg, WasmQuery,
};
use cw20::{BalanceResponse, Cw20HandleMsg, Cw20QueryMsg, TokenInfoResponse};
use std::convert::TryInto;
use terraswap::asset::{Asset, AssetInfo};
use terraswap::pair::HandleMsg as TerraswapPairHandleMsg;

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

/// Construct terraswap message to swap assets
pub fn asset_into_swap_msg<S: Storage, A: Api, Q: Querier>(
    _deps: &Extern<S, A, Q>,
    pair_contract: HumanAddr,
    offer_asset: Asset,
    max_spread: Option<Decimal>,
) -> StdResult<CosmosMsg<Empty>> {
    let message = match offer_asset.info.clone() {
        AssetInfo::NativeToken { denom } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_contract,
            msg: to_binary(&TerraswapPairHandleMsg::Swap {
                offer_asset: offer_asset.clone(),
                belief_price: None,
                max_spread,
                to: None,
            })?,
            send: vec![Coin {
                denom,
                amount: offer_asset.amount,
            }],
        }),
        AssetInfo::Token { contract_addr } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            msg: to_binary(&Cw20HandleMsg::Send {
                contract: pair_contract,
                amount: offer_asset.amount,
                msg: Some(to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset,
                    belief_price: None,
                    max_spread,
                    to: None,
                })?),
            })?,
            send: vec![],
        }),
    };
    Ok(message)
}
