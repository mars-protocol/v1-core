use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Env, Extern, HandleResponse, HumanAddr,
    InitResponse, Querier, StdError, StdResult, Storage, WasmMsg,
};

use cw20::{Cw20HandleMsg, MinterResponse};
use mars::ma_token;

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, QueryMsg, ReserveResponse};
use crate::state::{
    config_state, config_state_read, reserves_state, reserves_state_read, Config, Reserve,
};

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        ma_token_code_id: msg.ma_token_code_id,
    };

    config_state(&mut deps.storage).save(&config)?;

    Ok(InitResponse::default())
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::InitAsset { symbol } => init_asset(deps, env, symbol),
        HandleMsg::InitAssetTokenCallback { id } => init_asset_token_callback(deps, env, id),
        HandleMsg::DepositNative { symbol } => deposit_native(deps, env, symbol),
    }
}

/// Initialize asset so it can be deposited and borrowed.
/// A new maToken should be created which callbacks this contract in order to be registered
pub fn init_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    symbol: String,
) -> StdResult<HandleResponse> {
    // Get config
    let config = config_state_read(&deps.storage).load()?;

    // Only owner can do this
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    // create only if it doesn't exist
    let mut reserves = reserves_state(&mut deps.storage);
    match reserves.may_load(symbol.as_bytes()) {
        Ok(None) => {
            // create asset reserve
            reserves.save(
                symbol.as_bytes(),
                &Reserve {
                    ma_token_address: CanonicalAddr::default(),
                    liquidity_index: Decimal256::one(),
                },
            )?;
        }
        Ok(Some(_)) => return Err(StdError::generic_err("Asset already initialized")),
        Err(err) => return Err(err),
    }

    // Prepare response, should instantiate an maToken
    // and use the Register hook
    Ok(HandleResponse {
        log: vec![],
        data: None,
        messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id: config.ma_token_code_id,
            msg: to_binary(&ma_token::msg::InitMsg {
                name: format!("mars {} debt token", symbol),
                symbol: format!("ma{}", symbol),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: HumanAddr::from(env.contract.address.as_str()),
                    cap: None,
                }),
                init_hook: Some(ma_token::msg::InitHook {
                    msg: to_binary(&HandleMsg::InitAssetTokenCallback { id: symbol })?,
                    contract_addr: env.contract.address,
                }),
            })?,
            send: vec![],
            label: None,
        })],
    })
}

pub fn init_asset_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    id: String,
) -> StdResult<HandleResponse> {
    let mut state = reserves_state(&mut deps.storage);
    let mut reserve = state.load(&id.as_bytes())?;

    if reserve.ma_token_address == CanonicalAddr::default() {
        reserve.ma_token_address = deps.api.canonical_address(&env.message.sender)?;
        state.save(&id.as_bytes(), &reserve)?;
        Ok(HandleResponse {
            messages: vec![],
            log: vec![
                log("action", "init_asset"),
                log("asset", &id),
                log("ma_token_address", &env.message.sender),
            ],
            data: None,
        })
    } else {
        // Can do this only once
        Err(StdError::unauthorized())
    }
}

/// Handle the deposit of native tokens and mint corresponding debt tokens
pub fn deposit_native<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    symbol: String,
) -> StdResult<HandleResponse> {
    let reserve = reserves_state_read(&deps.storage).load(symbol.as_bytes())?;

    // Get deposit amount
    // TODO: asumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    let deposit_amount = env
        .message
        .sent_funds
        .iter()
        .find(|c| &c.denom[1..] == symbol)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // Cannot deposit zero amount
    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            symbol,
        )));
    }

    // TODO: Interest rate update and computing goes here

    let mint_amount = deposit_amount / reserve.liquidity_index;

    Ok(HandleResponse {
        data: None,
        log: vec![
            log("action", "deposit"),
            log("reserve", symbol),
            log("user", env.message.sender.clone()),
            log("amount", deposit_amount),
        ],
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: env.message.sender,
                amount: mint_amount.into(),
            })?,
        })],
    })
}

// QUERIES

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetConfig {} => to_binary(&query_config(deps)?),
        QueryMsg::QueryReserve { symbol } => to_binary(&query_reserve(deps, symbol)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let state = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        ma_token_code_id: state.ma_token_code_id,
    })
}

fn query_reserve<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    symbol: String,
) -> StdResult<ReserveResponse> {
    let reserve = reserves_state_read(&deps.storage).load(symbol.as_bytes())?;
    let ma_token_address = deps.api.human_address(&reserve.ma_token_address)?;
    Ok(ReserveResponse { ma_token_address })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{coin, from_binary, Uint128};

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 10u64,
        };
        let env = mock_env("owner", &[]);

        // we can just call .unwrap() to assert this was a success
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(&deps, QueryMsg::GetConfig {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(10, value.ma_token_code_id);
    }

    #[test]
    fn init_native_asset() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 5u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // non owner is not authorized
        let env = mock_env("somebody", &[]);
        let msg = HandleMsg::InitAsset {
            symbol: String::from("someasset"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // owner is authorized
        let env = mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            symbol: String::from("someasset"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // should have asset reserve with Canonical default address
        let reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(CanonicalAddr::default(), reserve.ma_token_address);

        // should instantiate a debt token
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 5u64,
                msg: to_binary(&ma_token::msg::InitMsg {
                    name: String::from("mars someasset debt token"),
                    symbol: String::from("masomeasset"),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        cap: None,
                    }),
                    init_hook: Some(ma_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitAssetTokenCallback {
                            id: String::from("someasset")
                        })
                        .unwrap(),
                        contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    }),
                })
                .unwrap(),
                send: vec![],
                label: None,
            }),]
        );

        // callback comes back with created token
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("someasset"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "init_asset"),
                log("asset", "someasset"),
                log("ma_token_address", "mtokencontract"),
            ]
        );

        // should have asset reserve with contract address
        let reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mtokencontract"))
                .unwrap(),
            reserve.ma_token_address
        );
        assert_eq!(Decimal256::one(), reserve.liquidity_index);

        // calling this again should not be allowed
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("someasset"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn init_asset_callback_cannot_be_called_on_its_own() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 1u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("luna"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn deposit_native_asset() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 1u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        let mut reserves = reserves_state(&mut deps.storage);
        reserves
            .save(
                b"somecoin",
                &Reserve {
                    ma_token_address: deps
                        .api
                        .canonical_address(&HumanAddr::from("matoken"))
                        .unwrap(),
                    liquidity_index: Decimal256::from_ratio(11, 10),
                },
            )
            .unwrap();

        let env = mock_env("depositer", &[coin(110000, "usomecoin")]);
        let msg = HandleMsg::DepositNative {
            symbol: String::from("somecoin"),
        };
        let res = handle(&mut deps, env, msg).unwrap();
        // mints coin_amount/liquidity_index
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("matoken"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("depositer"),
                    amount: Uint128(100000),
                })
                .unwrap(),
            }),]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "deposit"),
                log("reserve", "somecoin"),
                log("user", "depositer"),
                log("amount", "110000"),
            ]
        );

        // empty deposit fails
        let env = mock_env("depositer", &[]);
        let msg = HandleMsg::DepositNative {
            symbol: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn cannot_deposit_if_no_reserve() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 1u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        let env = mock_env("depositer", &[coin(110000, "usomecoin")]);
        let msg = HandleMsg::DepositNative {
            symbol: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }
}
