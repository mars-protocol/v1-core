use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::cw20_token;
use mars::helpers::{cw20_get_balance, cw20_get_total_supply};

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg, ReceiveMsg};
use crate::state::{
    basecamp_state, config_state, config_state_read, cooldowns_state, Basecamp, Config, Cooldown,
};

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // initialize Config
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        mars_token_address: CanonicalAddr::default(),
        xmars_token_address: CanonicalAddr::default(),
        cooldown_duration: msg.cooldown_duration,
        unstake_window: msg.unstake_window,
        voting_period: msg.voting_period,
        effective_delay: msg.effective_delay,
        expiration_period: msg.expiration_period,
        proposal_deposit: msg.proposal_deposit,
    };

    config_state(&mut deps.storage).save(&config)?;

    // initialize State
    basecamp_state(&mut deps.storage).save(&Basecamp { poll_count: 0 })?;

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
        HandleMsg::Cooldown {} => handle_cooldown(deps, env),
        HandleMsg::MintMars { recipient, amount } => handle_mint_mars(deps, env, recipient, amount),
        HandleMsg::CastVote { .. } => Ok(HandleResponse::default()), //TODO
        HandleMsg::EndPoll { .. } => Ok(HandleResponse::default()),  //TODO
        HandleMsg::ExecutePoll { .. } => Ok(HandleResponse::default()), //TODO
        HandleMsg::ExpirePoll { .. } => Ok(HandleResponse::default()), //TODO
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
            ReceiveMsg::Stake => handle_stake(deps, env, cw20_msg.sender, cw20_msg.amount),
            ReceiveMsg::Unstake => handle_unstake(deps, env, cw20_msg.sender, cw20_msg.amount),
            ReceiveMsg::SubmitPoll { .. } => Ok(HandleResponse::default()), // TODO
        }
    } else {
        Err(StdError::generic_err("Invalid Cw20ReceiveMsg"))
    }
}

/// Mint xMars tokens to staker
pub fn handle_stake<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    stake_amount: Uint128,
) -> StdResult<HandleResponse> {
    // check stake is valid
    let config = config_state_read(&deps.storage).load()?;
    // Has to send Mars tokens
    if deps.api.canonical_address(&env.message.sender)? != config.mars_token_address {
        return Err(StdError::unauthorized());
    }
    if stake_amount == Uint128(0) {
        return Err(StdError::generic_err("Stake amount must be greater than 0"));
    }

    // get total mars in contract before the stake transaction
    let total_mars_in_basecamp = (cw20_get_balance(
        deps,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )? - stake_amount)?;

    let total_xmars_supply =
        cw20_get_total_supply(deps, deps.api.human_address(&config.xmars_token_address)?)?;

    let mint_amount = if total_mars_in_basecamp == Uint128(0) || total_xmars_supply == Uint128(0) {
        stake_amount
    } else {
        stake_amount.multiply_ratio(total_xmars_supply, total_mars_in_basecamp)
    };

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.xmars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: staker.clone(),
                amount: mint_amount,
            })?,
        })],
        log: vec![
            log("action", "stake"),
            log("user", staker),
            log("mars_staked", stake_amount),
            log("xmars_minted", mint_amount),
        ],
        data: None,
    })
}

/// Burn xMars tokens and send corresponding Mars
pub fn handle_unstake<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    burn_amount: Uint128,
) -> StdResult<HandleResponse> {
    // check unstake is valid
    let config = config_state_read(&deps.storage).load()?;
    if deps.api.canonical_address(&env.message.sender)? != config.xmars_token_address {
        return Err(StdError::unauthorized());
    }
    if burn_amount == Uint128(0) {
        return Err(StdError::generic_err(
            "Unstake amount must be greater than 0",
        ));
    }

    // check valid cooldown
    //
    let mut cooldowns_bucket = cooldowns_state(&mut deps.storage);
    let staker_canonical_addr = deps.api.canonical_address(&staker)?;
    match cooldowns_bucket.may_load(staker_canonical_addr.as_slice())? {
        Some(mut cooldown) => {
            if burn_amount > cooldown.amount {
                return Err(StdError::generic_err(
                    "Unstake amount must not be greater than cooldown amount",
                ));
            }
            if env.block.time < cooldown.timestamp + config.cooldown_duration {
                return Err(StdError::generic_err("Cooldown has not finished"));
            }
            if env.block.time
                > cooldown.timestamp + config.cooldown_duration + config.unstake_window
            {
                return Err(StdError::generic_err("Cooldown has expired"));
            }

            if burn_amount == cooldown.amount {
                cooldowns_bucket.remove(staker_canonical_addr.as_slice());
            } else {
                cooldown.amount = (cooldown.amount - burn_amount)?;
                cooldowns_bucket.save(staker_canonical_addr.as_slice(), &cooldown)?;
            }
        }

        None => {
            return Err(StdError::generic_err(
                "Address must have a valid cooldown to unstake",
            ))
        }
    };

    let total_mars_in_basecamp = cw20_get_balance(
        deps,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )?;

    let total_xmars_supply =
        cw20_get_total_supply(deps, deps.api.human_address(&config.xmars_token_address)?)?;

    let unstake_amount = burn_amount.multiply_ratio(total_mars_in_basecamp, total_xmars_supply);

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.xmars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: burn_amount,
                })?,
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.mars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: staker.clone(),
                    amount: unstake_amount,
                })?,
            }),
        ],
        log: vec![
            log("action", "unstake"),
            log("user", staker),
            log("mars_unstaked", unstake_amount),
            log("xmars_burned", burn_amount),
        ],
        data: None,
    })
}

/// Handles cooldown. if staking non zero amount, activates a cooldown for that amount.
/// If a cooldown exists and amount has changed it computes the weighted average
/// for the cooldown
pub fn handle_cooldown<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    // get total mars in contract before the stake transaction
    let xmars_balance = cw20_get_balance(
        deps,
        deps.api.human_address(&config.xmars_token_address)?,
        env.message.sender.clone(),
    )?;

    if xmars_balance.is_zero() {
        return Err(StdError::unauthorized());
    }

    let mut cooldowns_bucket = cooldowns_state(&mut deps.storage);
    let sender_canonical_address = deps.api.canonical_address(&env.message.sender)?;

    // compute new cooldown timestamp
    let new_cooldown_timestamp =
        match cooldowns_bucket.may_load(sender_canonical_address.as_slice())? {
            Some(cooldown) => {
                let minimal_valid_cooldown_timestamp =
                    env.block.time - config.cooldown_duration - config.unstake_window;

                if cooldown.timestamp < minimal_valid_cooldown_timestamp {
                    env.block.time
                } else {
                    let mut extra_amount: u128 = 0;
                    if xmars_balance > cooldown.amount {
                        extra_amount = xmars_balance.u128() - cooldown.amount.u128();
                    };

                    (((cooldown.timestamp as u128) * cooldown.amount.u128()
                        + (env.block.time as u128) * extra_amount)
                        / (cooldown.amount.u128() + extra_amount)) as u64
                }
            }

            None => env.block.time,
        };

    cooldowns_bucket.save(
        &sender_canonical_address.as_slice(),
        &Cooldown {
            amount: xmars_balance,
            timestamp: new_cooldown_timestamp,
        },
    )?;

    Ok(HandleResponse {
        log: vec![
            log("action", "cooldown"),
            log("user", env.message.sender),
            log("cooldown_amount", xmars_balance),
            log("cooldown_timestamp", new_cooldown_timestamp),
        ],
        data: None,
        messages: vec![],
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
            msg: to_binary(&Cw20HandleMsg::Mint { recipient, amount }).unwrap(),
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
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{from_binary, Coin};
    use mars::testing::{mock_dependencies, mock_env, MockEnvParams, WasmMockQuerier};

    use crate::state::{basecamp_state_read, cooldowns_state_read};

    const TEST_COOLDOWN_DURATION: u64 = 1000;
    const TEST_UNSTAKE_WINDOW: u64 = 100;

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            cw20_code_id: 11,
            cooldown_duration: 20,
            unstake_window: 10,
            voting_period: 1,
            effective_delay: 1,
            expiration_period: 1,
            proposal_deposit: Uint128(1),
        };
        let env = mock_env("owner", MockEnvParams::default());

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

        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        assert_eq!(basecamp.poll_count, 0);

        // mars token init callback
        let msg = HandleMsg::InitTokenCallback { token_id: 0 };
        let env = mock_env("mars_token", MockEnvParams::default());
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
        let env = mock_env("mars_token_again", MockEnvParams::default());
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
        let env = mock_env("xmars_token", MockEnvParams::default());
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
        let env = mock_env("xmars_token_again", MockEnvParams::default());
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
        let staker_canonical_addr = deps
            .api
            .canonical_address(&HumanAddr::from("staker"))
            .unwrap();

        // no Mars in pool
        // stake X Mars -> should receive X xMars
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), Uint128(2_000_000))],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), Uint128(0));

        let env = mock_env("mars_token", MockEnvParams::default());
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
                log("action", "stake"),
                log("user", HumanAddr::from("staker")),
                log("mars_staked", 2_000_000),
                log("xmars_minted", 2_000_000),
            ],
            res.log
        );

        // Some Mars in pool and some xMars supply
        // stake Mars -> should receive less xMars
        let stake_amount = Uint128(2_000_000);
        let mars_in_basecamp = Uint128(4_000_000);
        let xmars_supply = Uint128(1_000_000);

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: stake_amount,
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), mars_in_basecamp)],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), xmars_supply);

        let env = mock_env("mars_token", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_minted_xmars =
            stake_amount.multiply_ratio(xmars_supply, (mars_in_basecamp - stake_amount).unwrap());

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("xmars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("staker"),
                    amount: expected_minted_xmars,
                })
                .unwrap(),
            })],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "stake"),
                log("user", HumanAddr::from("staker")),
                log("mars_staked", stake_amount),
                log("xmars_minted", expected_minted_xmars),
            ],
            res.log
        );

        // stake other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        let env = mock_env("other_token", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // setup variables for unstake
        let unstake_amount = Uint128(1_000_000);
        let unstake_mars_in_basecamp = Uint128(4_000_000);
        let unstake_xmars_supply = Uint128(3_000_000);
        let unstake_block_timestamp = 1_000_000_000;
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: unstake_amount,
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                unstake_mars_in_basecamp,
            )],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), unstake_xmars_supply);

        // unstake Mars no cooldown -> unauthorized
        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars expired cooldown -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp
                        - TEST_COOLDOWN_DURATION
                        - TEST_UNSTAKE_WINDOW
                        - 1,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars unfinished cooldown -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION + 1,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars cooldown with low amount -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: (unstake_amount - Uint128(1000)).unwrap(),
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // partial unstake Mars valid cooldown -> burn xMars, receive Mars back,
        // deduct cooldown amount
        let pending_cooldown_amount = Uint128(300_000);
        let pending_cooldown_timestamp = unstake_block_timestamp - TEST_COOLDOWN_DURATION;

        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount + pending_cooldown_amount,
                    timestamp: pending_cooldown_timestamp,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_returned_mars =
            unstake_amount.multiply_ratio(unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("xmars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: unstake_amount,
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("mars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: HumanAddr::from("staker"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                }),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "unstake"),
                log("user", HumanAddr::from("staker")),
                log("mars_unstaked", expected_returned_mars),
                log("xmars_burned", unstake_amount),
            ],
            res.log
        );

        let actual_cooldown = cooldowns_state_read(&deps.storage)
            .load(staker_canonical_addr.as_slice())
            .unwrap();

        assert_eq!(actual_cooldown.amount, pending_cooldown_amount);
        assert_eq!(actual_cooldown.timestamp, pending_cooldown_timestamp);

        // unstake other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: pending_cooldown_amount,
        });

        let env = mock_env(
            "other_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );

        let _res = handle(&mut deps, env, msg).unwrap_err();

        // unstake pending amount Mars -> cooldown is deleted
        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: pending_cooldown_amount,
        });
        let res = handle(&mut deps, env, msg).unwrap();

        // NOTE: In reality the mars/xmars amounts would change but since they are being
        // mocked it does not really matter here.
        let expected_returned_mars =
            pending_cooldown_amount.multiply_ratio(unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("xmars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: pending_cooldown_amount,
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("mars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: HumanAddr::from("staker"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                }),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "unstake"),
                log("user", HumanAddr::from("staker")),
                log("mars_unstaked", expected_returned_mars),
                log("xmars_burned", pending_cooldown_amount),
            ],
            res.log
        );

        let actual_cooldown = cooldowns_state_read(&deps.storage)
            .may_load(staker_canonical_addr.as_slice())
            .unwrap();

        assert_eq!(actual_cooldown, None);
    }

    #[test]
    fn test_cooldown() {
        let mut deps = th_setup(&[]);

        let initial_block_time = 1_600_000_000;
        let ongoing_cooldown_block_time = initial_block_time + TEST_COOLDOWN_DURATION / 2;

        // staker with no xmars is unauthorized
        let msg = HandleMsg::Cooldown {};

        let env = mock_env("staker", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // staker with xmars gets a cooldown equal to the xmars balance
        let initial_xmars_balance = Uint128(1_000_000);
        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(HumanAddr::from("staker"), initial_xmars_balance)],
        );

        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: initial_block_time,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);
        assert_eq!(
            vec![
                log("action", "cooldown"),
                log("user", "staker"),
                log("cooldown_amount", initial_xmars_balance),
                log("cooldown_timestamp", initial_block_time)
            ],
            res.log
        );

        // same amount does not alterate cooldown
        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: ongoing_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);

        // more amount gets a weighted average timestamp with the new amount
        let additional_xmars_balance = Uint128(500_000);

        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(
                HumanAddr::from("staker"),
                initial_xmars_balance + additional_xmars_balance,
            )],
        );
        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: ongoing_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        let expected_cooldown_timestamp =
            (((initial_block_time as u128) * initial_xmars_balance.u128()
                + (ongoing_cooldown_block_time as u128) * additional_xmars_balance.u128())
                / (initial_xmars_balance + additional_xmars_balance).u128()) as u64;
        assert_eq!(cooldown.timestamp, expected_cooldown_timestamp);
        assert_eq!(
            cooldown.amount,
            initial_xmars_balance + additional_xmars_balance
        );

        // expired cooldown with more amount gets a new timestamp (test lower and higher)
        let expired_cooldown_block_time =
            expected_cooldown_timestamp + TEST_COOLDOWN_DURATION + TEST_UNSTAKE_WINDOW + 1;
        let expired_balance = initial_xmars_balance + additional_xmars_balance + Uint128(800_000);
        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(HumanAddr::from("staker"), expired_balance)],
        );

        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: expired_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, expired_cooldown_block_time);
        assert_eq!(cooldown.amount, expired_balance);
    }

    #[test]
    fn test_mint_mars() {
        let mut deps = th_setup(&[]);

        // stake Mars -> should receive xMars
        let msg = HandleMsg::MintMars {
            recipient: HumanAddr::from("recipient"),
            amount: Uint128(3_500_000),
        };

        let env = mock_env("owner", MockEnvParams::default());
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

        let env = mock_env("someoneelse", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let msg = InitMsg {
            cw20_code_id: 1,
            cooldown_duration: TEST_COOLDOWN_DURATION,
            unstake_window: TEST_UNSTAKE_WINDOW,
            voting_period: 1,
            effective_delay: 1,
            expiration_period: 1,
            proposal_deposit: Uint128(1),
        };
        let env = mock_env("owner", MockEnvParams::default());
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
