use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::cw20_token;

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg, ReceiveMsg};
use crate::state::{config_state, config_state_read, Config};

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        mars_token_address: CanonicalAddr::default(),
        xmars_token_address: CanonicalAddr::default(),
    };

    config_state(&mut deps.storage).save(&config)?;

    // Prepare response, should instantiate Mars and xMars
    // and use the Register hook
    Ok(InitResponse {
        log: vec![],
        // TODO: Tokens are initialized here. Evaluate doing this outside of
        // the contract
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: msg.cw20_code_id,
                msg: to_binary(&cw20_token::msg::InitMsg {
                    name: "Mars token".to_string(),
                    symbol: "Mars".to_string(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(env.contract.address.as_str()),
                        cap: None,
                    }),
                    init_hook: Some(cw20_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 0 })?,
                        contract_addr: env.contract.address.clone(),
                    }),
                })?,
                send: vec![],
                label: None,
            }),
            CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: msg.cw20_code_id,
                msg: to_binary(&cw20_token::msg::InitMsg {
                    name: "xMars token".to_string(),
                    symbol: "xMars".to_string(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(env.contract.address.as_str()),
                        cap: None,
                    }),
                    init_hook: Some(cw20_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 1 })?,
                        contract_addr: env.contract.address,
                    }),
                })?,
                send: vec![],
                label: None,
            }),
        ],
    })
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Receive(cw20_msg) => handle_receive_cw20(deps, env, cw20_msg),
        HandleMsg::InitTokenCallback { token_id } => {
            handle_init_token_callback(deps, env, token_id)
        }
        HandleMsg::MintMars { recipient, amount } => handle_mint_mars(deps, env, recipient, amount),
    }
}

/// cw20 receive implementation
pub fn handle_receive_cw20<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<HandleResponse> {
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            ReceiveMsg::Bond => handle_bond(deps, env, cw20_msg.sender, cw20_msg.amount),
            ReceiveMsg::Unbond => handle_unbond(deps, env, cw20_msg.sender, cw20_msg.amount),
        }
    } else {
        Err(StdError::generic_err("Invalid Cw20RecieveMsg"))
    }
}

/// Mint xMars tokens to bonder
pub fn handle_bond<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;
    if deps.api.canonical_address(&env.message.sender)? != config.mars_token_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128(0) {
        return Err(StdError::generic_err("Bond amount must be greater than 0"));
    }

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.xmars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: staker.clone(),
                amount: amount,
            })?,
        })],
        log: vec![
            log("action", "bond"),
            log("user", staker),
            log("bond_amount", amount),
        ],
        data: None,
    })
}

/// Burn xMars tokens and send corresponding Mars
pub fn handle_unbond<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;
    if deps.api.canonical_address(&env.message.sender)? != config.xmars_token_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128(0) {
        return Err(StdError::generic_err(
            "Unbond amount must be greater than 0",
        ));
    }

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.xmars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn { amount: amount })?,
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.mars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: staker.clone(),
                    amount: amount,
                })?,
            }),
        ],
        log: vec![
            log("action", "unbond"),
            log("user", staker),
            log("bond_amount", amount),
        ],
        data: None,
    })
}

/// Handles token post initialization storing the addresses
/// in config
/// token is a byte: 0 = Mars, 1 = xMars, others are not authorized
pub fn handle_init_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    token_id: u8,
) -> StdResult<HandleResponse> {
    let mut config_singleton = config_state(&mut deps.storage);
    let mut config = config_singleton.load()?;

    return match token_id {
        // Mars
        0 => {
            if config.mars_token_address == CanonicalAddr::default() {
                config.mars_token_address = deps.api.canonical_address(&env.message.sender)?;
                config_singleton.save(&config)?;
                Ok(HandleResponse {
                    messages: vec![],
                    log: vec![
                        log("action", "init_mars_token"),
                        log("token_address", &env.message.sender),
                    ],
                    data: None,
                })
            } else {
                // Can do this only once
                Err(StdError::unauthorized())
            }
        }
        // xMars
        1 => {
            if config.xmars_token_address == CanonicalAddr::default() {
                config.xmars_token_address = deps.api.canonical_address(&env.message.sender)?;
                config_singleton.save(&config)?;
                Ok(HandleResponse {
                    messages: vec![],
                    log: vec![
                        log("action", "init_xmars_token"),
                        log("token_address", &env.message.sender),
                    ],
                    data: None,
                })
            } else {
                // Can do this only once
                Err(StdError::unauthorized())
            }
        }
        _ => Err(StdError::unauthorized()),
    };
}

/// Mints Mars token to receiver (Temp action for testing)
pub fn handle_mint_mars<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    // Only owner can trigger a mint
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.mars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: recipient,
                amount: amount,
            })
            .unwrap(),
        })],
        log: vec![],
        data: None,
    })
}

// QUERIES

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        mars_token_address: deps.api.human_address(&config.mars_token_address)?,
        xmars_token_address: deps.api.human_address(&config.xmars_token_address)?,
    })
}

// MIGRATION

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{
        mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR,
    };
    use cosmwasm_std::{from_binary, Coin};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg { cw20_code_id: 11 };
        let env = mock_env("owner", &[]);

        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Instantiate {
                    code_id: 11,
                    msg: to_binary(&cw20_token::msg::InitMsg {
                        name: "Mars token".to_string(),
                        symbol: "Mars".to_string(),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                            cap: None,
                        }),
                        init_hook: Some(cw20_token::msg::InitHook {
                            msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 0 }).unwrap(),
                            contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        }),
                    })
                    .unwrap(),
                    send: vec![],
                    label: None,
                }),
                CosmosMsg::Wasm(WasmMsg::Instantiate {
                    code_id: 11,
                    msg: to_binary(&cw20_token::msg::InitMsg {
                        name: "xMars token".to_string(),
                        symbol: "xMars".to_string(),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                            cap: None,
                        }),
                        init_hook: Some(cw20_token::msg::InitHook {
                            msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 1 }).unwrap(),
                            contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        }),
                    })
                    .unwrap(),
                    send: vec![],
                    label: None,
                })
            ],
            res.messages
        );

        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("owner"))
                .unwrap(),
            config.owner
        );
        assert_eq!(CanonicalAddr::default(), config.mars_token_address);
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // mars token init callback
        let msg = HandleMsg::InitTokenCallback { token_id: 0 };
        let env = mock_env("mars_token", &[]);
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                log("action", "init_mars_token"),
                log("token_address", HumanAddr::from("mars_token")),
            ],
            res.log
        );
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // trying again fails
        let msg = HandleMsg::InitTokenCallback { token_id: 0 };
        let env = mock_env("mars_token_again", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // xmars token init callback
        let msg = HandleMsg::InitTokenCallback { token_id: 1 };
        let env = mock_env("xmars_token", &[]);
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                log("action", "init_xmars_token"),
                log("token_address", HumanAddr::from("xmars_token")),
            ],
            res.log
        );
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("xmars_token"))
                .unwrap(),
            config.xmars_token_address
        );

        // trying again fails
        let msg = HandleMsg::InitTokenCallback { token_id: 1 };
        let env = mock_env("xmars_token_again", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();

        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("xmars_token"))
                .unwrap(),
            config.xmars_token_address
        );

        // query works now
        let res = query(&deps, QueryMsg::Config {}).unwrap();
        let config: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(HumanAddr::from("mars_token"), config.mars_token_address);
        assert_eq!(HumanAddr::from("xmars_token"), config.xmars_token_address);
    }

    #[test]
    fn test_staking() {
        let mut deps = th_setup(&[]);

        // bond Mars -> should receive xMars
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Bond).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        let env = mock_env("mars_token", &[]);
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("xmars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("staker"),
                    amount: Uint128(2_000_000),
                })
                .unwrap(),
            })],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "bond"),
                log("user", HumanAddr::from("staker")),
                log("bond_amount", 2_000_000),
            ],
            res.log
        );

        // bond other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Bond).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        let env = mock_env("other_token", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // unbond Mars -> should burn xMars and receive Mars back
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unbond).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(1_000_000),
        });

        let env = mock_env("xmars_token", &[]);
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("xmars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: Uint128(1_000_000),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("mars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: HumanAddr::from("staker"),
                        amount: Uint128(1_000_000),
                    })
                    .unwrap(),
                }),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "unbond"),
                log("user", HumanAddr::from("staker")),
                log("bond_amount", 1_000_000),
            ],
            res.log
        );

        // unbond other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unbond).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        let env = mock_env("other_token", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_mint_mars() {
        let mut deps = th_setup(&[]);

        // bond Mars -> should receive xMars
        let msg = HandleMsg::MintMars {
            recipient: HumanAddr::from("recipient"),
            amount: Uint128(3_500_000),
        };

        let env = mock_env("owner", &[]);
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("recipient"),
                    amount: Uint128(3_500_000),
                })
                .unwrap(),
            })],
            res.messages
        );

        // mint by non owner -> Unauthorized
        let msg = HandleMsg::MintMars {
            recipient: HumanAddr::from("recipient"),
            amount: Uint128(3_500_000),
        };

        let env = mock_env("someoneelse", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let msg = InitMsg { cw20_code_id: 1 };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        let mut config_singleton = config_state(&mut deps.storage);
        let mut config = config_singleton.load().unwrap();
        config.mars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("mars_token"))
            .unwrap();
        config.xmars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("xmars_token"))
            .unwrap();
        config_singleton.save(&config).unwrap();

        deps
    }
}
