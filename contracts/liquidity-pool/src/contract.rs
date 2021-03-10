use cosmwasm_std::{
    to_binary, Api, Binary, CosmosMsg, CanonicalAddr, Env, Extern, HandleResponse, HumanAddr,
    InitResponse, log, Querier, StdError, StdResult, Storage, WasmMsg
};

use cw20::MinterResponse;
use mars::ma_token;

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, QueryMsg};
use crate::state::{
    config_state, config_state_read, Config,
    reserves_state, reserves_state_read, Reserve
};

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        ma_token_contract_id: msg.ma_token_contract_id,
    };

    config_state(&mut deps.storage).save(&config)?;

    Ok(InitResponse::default())
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::InitAsset { symbol } => try_init_asset(deps, env, symbol),
        HandleMsg::InitAssetTokenCallback{ id } => try_init_asset_token_callback(deps, env, id),
    }
}

/// Initialize asset so it can be deposited and borrowed.
/// A new maToken should be created which callbacks this contract in order to be registered
pub fn try_init_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    symbol: String
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
            reserves.save(symbol.as_bytes(), &Reserve {
               ma_token_address: CanonicalAddr::default(),
            })?;
        }
        Ok(Some(_)) => return Err(StdError::generic_err("Asset already initialized")),
        Err(err) => return Err(err),
    }
    

    // Prepare response, should instantiate an maToken
    // and use the Register hook
    Ok(HandleResponse{
        log: vec![],
        data: None,
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 1u64,
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
                        msg: to_binary(&HandleMsg::InitAssetTokenCallback { id: String::from("luna")})?,
                        contract_addr: env.contract.address,
                    }),
                })?,
                send: vec![],
                label: None,
            }),
        ]
    })
}

pub fn try_init_asset_token_callback<S: Storage, A: Api, Q: Querier>(
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
                log("ma_token_address", &env.message.sender)
            ],
            data: None,
        })
    } else {
        // Can do this only once
        Err(StdError::unauthorized())
    }

}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetConfig {} => to_binary(&query_config(deps)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<ConfigResponse> {
    let state = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse { ma_token_contract_id: state.ma_token_contract_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{from_binary};

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg { ma_token_contract_id: 1u64 };
        let env = mock_env("owner", &[]);

        // we can just call .unwrap() to assert this was a success
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(&deps, QueryMsg::GetConfig {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(1, value.ma_token_contract_id);
    }

    #[test]
    fn init_native_asset() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg { ma_token_contract_id: 1u64 };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // non owner is not authorized
        let env = mock_env("somebody", &[]);
        let msg = HandleMsg::InitAsset { symbol: String::from("luna") };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // owner is authorized
        let env = mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset { symbol: String::from("luna") };
        let res = handle(&mut deps, env, msg).unwrap();

        // should have asset reserve with Canonical default address
        let reserve = reserves_state_read(&deps.storage).load(b"luna").unwrap();
        assert_eq!(CanonicalAddr::default(), reserve.ma_token_address);

        // should instantiate a debt token
        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Instantiate {
                    code_id: 1u64,
                    msg: to_binary(&ma_token::msg::InitMsg {
                        name: String::from("mars luna debt token"),
                        symbol: String::from("maluna"),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                            cap: None,
                        }),
                        init_hook: Some(ma_token::msg::InitHook {
                            msg: to_binary(
                                     &HandleMsg::InitAssetTokenCallback {
                                         id: String::from("luna")
                                     }
                                ).unwrap(),
                            contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        }),
                    })
                    .unwrap(),
                    send: vec![],
                    label: None,
                }),
            ]
        );

        // callback comes back with created token
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback { id: String::from("luna") };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "init_asset"),
                log("asset", "luna"),
                log("ma_token_address", "mtokencontract"),
            ]
        );

        // should have asset reserve with contract address
        let reserve = reserves_state_read(&deps.storage).load(b"luna").unwrap();
        assert_eq!(
            deps.api.canonical_address(&HumanAddr::from("mtokencontract")).unwrap(),
            reserve.ma_token_address
        );

        // calling this again should not be allowed
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback { id: String::from("luna") };
        let _res = handle(&mut deps, env, msg).unwrap_err();

    }

    #[test]
    fn init_asset_callback_cannot_be_called_on_its_own() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg { ma_token_contract_id: 1u64 };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback { id: String::from("luna") };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

}
