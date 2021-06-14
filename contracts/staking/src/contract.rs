use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use crate::msg::{
    ConfigResponse, CooldownResponse, CreateOrUpdateConfig, HandleMsg, InitMsg, MigrateMsg,
    QueryMsg, ReceiveMsg,
};
use crate::state::{
    config_state, config_state_read, cooldowns_state, cooldowns_state_read, Config, Cooldown,
};
use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::cw20_token;
use mars::helpers::{cw20_get_balance, cw20_get_total_supply, unwrap_or};
use terraswap::asset::{Asset as TerraswapAsset, AssetInfo, PairInfo};
use terraswap::pair::HandleMsg as TerraswapPairHandleMsg;
use terraswap::querier::query_pair_info;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        mars_token_address,
        terraswap_factory_address,
        terraswap_max_spread,
        cooldown_duration,
        unstake_window,
    } = msg.config;

    // All fields should be available
    let available = mars_token_address.is_some()
        && terraswap_factory_address.is_some()
        && terraswap_max_spread.is_some()
        && cooldown_duration.is_some()
        && unstake_window.is_some();

    if !available {
        return Err(StdError::generic_err(
            "All params should be available during initialization",
        ));
    };

    // Initialize config
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        mars_token_address: deps.api.canonical_address(&mars_token_address.unwrap())?,
        xmars_token_address: CanonicalAddr::default(),
        terraswap_factory_address: deps
            .api
            .canonical_address(&terraswap_factory_address.unwrap())?,
        terraswap_max_spread: terraswap_max_spread.unwrap(),
        cooldown_duration: cooldown_duration.unwrap(),
        unstake_window: unstake_window.unwrap(),
    };

    config_state(&mut deps.storage).save(&config)?;

    // Prepare response, should instantiate xMars
    // and use the Register hook
    Ok(InitResponse {
        log: vec![],
        // TODO: Tokens are initialized here. Evaluate doing this outside of
        // the contract
        messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
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
                    msg: to_binary(&HandleMsg::InitTokenCallback {})?,
                    contract_addr: env.contract.address,
                }),
            })?,
            send: vec![],
            label: None,
        })],
    })
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::UpdateConfig {
            owner,
            xmars_token_address,
            config,
        } => handle_update_config(deps, env, owner, xmars_token_address, config),
        HandleMsg::Receive(cw20_msg) => handle_receive_cw20(deps, env, cw20_msg),
        HandleMsg::InitTokenCallback {} => handle_init_xmars_token_callback(deps, env),
        HandleMsg::Cooldown {} => handle_cooldown(deps, env),
        HandleMsg::ExecuteCosmosMsg(cosmos_msg) => handle_execute_cosmos_msg(deps, env, cosmos_msg),
        HandleMsg::SwapAssetToUusd {
            offer_asset_info,
            amount,
        } => handle_swap_asset_to_uusd(deps, env, offer_asset_info, amount),
        HandleMsg::SwapAssetToMars {
            offer_asset_info,
            amount,
        } => handle_swap_asset_to_mars(deps, env, offer_asset_info, amount),
    }
}

/// Update config
pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    xmars_token_address: Option<HumanAddr>,
    new_config: CreateOrUpdateConfig,
) -> StdResult<HandleResponse> {
    let mut config = config_state_read(&deps.storage).load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        mars_token_address,
        terraswap_factory_address,
        terraswap_max_spread,
        cooldown_duration,
        unstake_window,
    } = new_config;

    // Update config
    config.owner = unwrap_or(deps.api, owner, config.owner)?;
    config.xmars_token_address =
        unwrap_or(deps.api, xmars_token_address, config.xmars_token_address)?;
    config.mars_token_address = unwrap_or(deps.api, mars_token_address, config.mars_token_address)?;
    config.terraswap_factory_address = unwrap_or(
        deps.api,
        terraswap_factory_address,
        config.terraswap_factory_address,
    )?;
    config.terraswap_max_spread = terraswap_max_spread.unwrap_or(config.terraswap_max_spread);
    config.cooldown_duration = cooldown_duration.unwrap_or(config.cooldown_duration);
    config.unstake_window = unstake_window.unwrap_or(config.unstake_window);

    config_state(&mut deps.storage).save(&config)?;

    Ok(HandleResponse::default())
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

    let total_mars_in_staking_contract = cw20_get_balance(
        &deps.querier,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )?;
    // Mars amount needs to be before the stake transaction (which is already in the staking contract's
    // balance so it needs to be deducted)
    let net_total_mars_in_staking_contract = (total_mars_in_staking_contract - stake_amount)?;

    let total_xmars_supply = cw20_get_total_supply(
        &deps.querier,
        deps.api.human_address(&config.xmars_token_address)?,
    )?;

    let mint_amount =
        if net_total_mars_in_staking_contract == Uint128(0) || total_xmars_supply == Uint128(0) {
            stake_amount
        } else {
            stake_amount.multiply_ratio(total_xmars_supply, net_total_mars_in_staking_contract)
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
    // check if unstake is valid
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

    let total_mars_in_staking_contract = cw20_get_balance(
        &deps.querier,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )?;

    let total_xmars_supply = cw20_get_total_supply(
        &deps.querier,
        deps.api.human_address(&config.xmars_token_address)?,
    )?;

    let unstake_amount =
        burn_amount.multiply_ratio(total_mars_in_staking_contract, total_xmars_supply);

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

/// Handles xMars post-initialization storing the address in config
pub fn handle_init_xmars_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config_singleton = config_state(&mut deps.storage);
    let mut config = config_singleton.load()?;

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

/// Handles cooldown. if staking non zero amount, activates a cooldown for that amount.
/// If a cooldown exists and amount has changed it computes the weighted average
/// for the cooldown
pub fn handle_cooldown<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    // get total xMars in contract before the stake transaction
    let xmars_balance = cw20_get_balance(
        &deps.querier,
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

/// Execute Cosmos message
pub fn handle_execute_cosmos_msg<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: CosmosMsg,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    Ok(HandleResponse {
        messages: vec![msg],
        log: vec![log("action", "execute_cosmos_msg")],
        data: None,
    })
}

/// Swap any asset on the contract to uusd
pub fn handle_swap_asset_to_uusd<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    offer_asset_info: AssetInfo,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    let ask_asset_info = AssetInfo::NativeToken {
        denom: "uusd".to_string(),
    };

    handle_swap(deps, env, config, offer_asset_info, ask_asset_info, amount)
}

/// Swap any asset on the contract to Mars
pub fn handle_swap_asset_to_mars<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    offer_asset_info: AssetInfo,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    let mars_token_human_addr = deps.api.human_address(&config.mars_token_address)?;
    let ask_asset_info = AssetInfo::Token {
        contract_addr: mars_token_human_addr,
    };

    handle_swap(deps, env, config, offer_asset_info, ask_asset_info, amount)
}

fn handle_swap<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    config: Config,
    offer_asset_info: AssetInfo,
    ask_asset_info: AssetInfo,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {
    // swapping the same assets doesn't make any sense
    if offer_asset_info == ask_asset_info {
        return Err(StdError::generic_err(format!(
            "Cannot swap the same assets {}",
            offer_asset_info
        )));
    }

    let mars_token_human_addr = deps.api.human_address(&config.mars_token_address)?;
    let (contract_asset_balance, asset_label) = match offer_asset_info.clone() {
        AssetInfo::NativeToken { denom } => (
            deps.querier
                .query_balance(env.contract.address, denom.as_str())?
                .amount,
            denom,
        ),
        AssetInfo::Token { contract_addr } if contract_addr == mars_token_human_addr => {
            // throw error if the user tries to swap Mars
            return Err(StdError::generic_err("Cannot swap Mars"));
        }
        AssetInfo::Token { contract_addr } => {
            let asset_label = String::from(contract_addr.as_str());
            (
                cw20_get_balance(&deps.querier, contract_addr, env.contract.address)?,
                asset_label,
            )
        }
    };

    if contract_asset_balance.is_zero() {
        return Err(StdError::generic_err(format!(
            "Contract has no balance for the asset {}",
            asset_label
        )));
    }

    let contract_asset_balance_for_swap = match amount {
        Some(amount) if amount > contract_asset_balance => {
            return Err(StdError::generic_err(format!(
                "The amount requested for swap exceeds contract balance for the asset {}",
                asset_label
            )));
        }
        Some(amount) => amount,
        None => contract_asset_balance,
    };

    let terraswap_factory_human_addr = deps.api.human_address(&config.terraswap_factory_address)?;
    let pair_info: PairInfo = query_pair_info(
        &deps,
        &terraswap_factory_human_addr,
        &[offer_asset_info.clone(), ask_asset_info],
    )?;

    let send_msg = match offer_asset_info.clone() {
        AssetInfo::NativeToken { denom } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_info.contract_addr,
            msg: to_binary(&TerraswapPairHandleMsg::Swap {
                offer_asset: TerraswapAsset {
                    info: offer_asset_info,
                    amount: contract_asset_balance_for_swap,
                },
                belief_price: None,
                max_spread: Some(config.terraswap_max_spread),
                to: None,
            })?,
            send: vec![Coin {
                denom,
                amount: contract_asset_balance_for_swap,
            }],
        }),
        AssetInfo::Token { contract_addr } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            msg: to_binary(&Cw20HandleMsg::Send {
                contract: pair_info.contract_addr,
                amount: contract_asset_balance_for_swap,
                msg: Some(to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset: TerraswapAsset {
                        info: offer_asset_info,
                        amount: contract_asset_balance_for_swap,
                    },
                    belief_price: None,
                    max_spread: Some(config.terraswap_max_spread),
                    to: None,
                })?),
            })?,
            send: vec![],
        }),
    };

    Ok(HandleResponse {
        messages: vec![send_msg],
        log: vec![log("action", "swap"), log("asset", asset_label)],
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
        QueryMsg::Cooldown { sender_address } => to_binary(&query_cooldown(deps, sender_address)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        mars_token_address: deps.api.human_address(&config.mars_token_address)?,
        xmars_token_address: deps.api.human_address(&config.xmars_token_address)?,
    })
}

fn query_cooldown<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    sender_address: HumanAddr,
) -> StdResult<CooldownResponse> {
    let cooldown = cooldowns_state_read(&deps.storage)
        .may_load(deps.api.canonical_address(&sender_address)?.as_slice())?;

    match cooldown {
        Some(result) => Ok(CooldownResponse {
            timestamp: result.timestamp,
            amount: result.amount,
        }),
        None => Result::Err(StdError::not_found("No cooldown found")),
    }
}

// MIGRATION

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// HELPERS

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{BankMsg, Coin, CosmosMsg, Decimal, HumanAddr};
    use mars::testing::{mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams};

    use crate::msg::HandleMsg::UpdateConfig;
    use crate::state::{config_state_read, cooldowns_state_read};
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};

    const TEST_COOLDOWN_DURATION: u64 = 1000;
    const TEST_UNSTAKE_WINDOW: u64 = 100;

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with empty params
        // *
        let empty_config = CreateOrUpdateConfig {
            mars_token_address: None,
            terraswap_factory_address: None,
            terraswap_max_spread: None,
            cooldown_duration: None,
            unstake_window: None,
        };
        let msg = InitMsg {
            cw20_code_id: 11,
            config: empty_config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let res_error = init(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "All params should be available during initialization")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        let config = CreateOrUpdateConfig {
            mars_token_address: Some(HumanAddr::from("mars_token")),
            terraswap_factory_address: Some(HumanAddr::from("terraswap_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(20),
            unstake_window: Some(10),
        };
        let msg = InitMsg {
            cw20_code_id: 11,
            config,
        };
        let env = mock_env("owner", MockEnvParams::default());

        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
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
                        msg: to_binary(&HandleMsg::InitTokenCallback {}).unwrap(),
                        contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    }),
                })
                .unwrap(),
                send: vec![],
                label: None,
            })],
            res.messages
        );

        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("owner"))
                .unwrap(),
            config.owner
        );
        let mars_token_canonical_address = deps
            .api
            .canonical_address(&HumanAddr::from("mars_token"))
            .unwrap();
        assert_eq!(config.mars_token_address, mars_token_canonical_address);
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // xmars token init callback
        let msg = HandleMsg::InitTokenCallback {};
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
                .canonical_address(&HumanAddr::from("xmars_token"))
                .unwrap(),
            config.xmars_token_address
        );

        // trying again fails
        let msg = HandleMsg::InitTokenCallback {};
        let env = mock_env("xmars_token_again", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();
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
    fn test_update_config() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with valid params
        // *
        let init_config = CreateOrUpdateConfig {
            mars_token_address: Some(HumanAddr::from("mars_token")),
            terraswap_factory_address: Some(HumanAddr::from("terraswap_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(20),
            unstake_window: Some(10),
        };
        let msg = InitMsg {
            cw20_code_id: 11,
            config: init_config.clone(),
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            owner: None,
            xmars_token_address: None,
            config: init_config,
        };
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // update config with all new params
        // *
        let config = CreateOrUpdateConfig {
            mars_token_address: Some(HumanAddr::from("new_mars_addr")),
            terraswap_factory_address: Some(HumanAddr::from("new_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(2u128, 100u128)),
            cooldown_duration: Some(200),
            unstake_window: Some(100),
        };
        let msg = UpdateConfig {
            owner: Some(HumanAddr::from("new_owner")),
            xmars_token_address: Some(HumanAddr::from("new_xmars_addr")),
            config: config.clone(),
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        // we can just call .unwrap() to assert this was a success
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = config_state_read(&deps.storage).load().unwrap();

        assert_eq!(
            new_config.owner,
            deps.api
                .canonical_address(&HumanAddr::from("new_owner"))
                .unwrap()
        );
        assert_eq!(
            new_config.xmars_token_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_xmars_addr"))
                .unwrap()
        );
        assert_eq!(
            new_config.mars_token_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_mars_addr"))
                .unwrap()
        );
        assert_eq!(
            new_config.terraswap_factory_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_factory"))
                .unwrap()
        );
        assert_eq!(
            new_config.cooldown_duration,
            config.cooldown_duration.unwrap()
        );
        assert_eq!(new_config.unstake_window, config.unstake_window.unwrap());
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
        handle(&mut deps, env, msg.clone()).unwrap_err();

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
        handle(&mut deps, env, msg.clone()).unwrap_err();

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
        handle(&mut deps, env, msg.clone()).unwrap_err();

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
        handle(&mut deps, env, msg.clone()).unwrap_err();

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

        handle(&mut deps, env, msg).unwrap_err();

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
        handle(&mut deps, env, msg).unwrap_err();

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

        // same amount does not alter cooldown
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

        // additional amount gets a weighted average timestamp with the new amount
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

        // expired cooldown with additional amount gets a new timestamp (test lower and higher)
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
        handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

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
    fn test_execute_cosmos_msg() {
        let mut deps = th_setup(&[]);

        let bank = BankMsg::Send {
            from_address: HumanAddr("source".to_string()),
            to_address: HumanAddr("destination".to_string()),
            amount: vec![Coin {
                denom: "uluna".to_string(),
                amount: Uint128(123456u128),
            }],
        };
        let cosmos_msg = CosmosMsg::Bank(bank);
        let msg = HandleMsg::ExecuteCosmosMsg(cosmos_msg.clone());

        // *
        // non owner is not authorized
        // *
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg.clone()).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // can execute Cosmos msg
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(res.messages, vec![cosmos_msg]);
        let expected_log = vec![log("action", "execute_cosmos_msg")];
        assert_eq!(res.log, expected_log);
    }

    #[test]
    fn test_swap_asset_to_uusd() {
        let contract_asset_balance = Uint128(1_000_000);
        let mut deps = th_setup(&[
            Coin {
                denom: "somecoin".to_string(),
                amount: contract_asset_balance,
            },
            Coin {
                denom: "zero".to_string(),
                amount: Uint128::zero(),
            },
        ]);

        // *
        // can't swap the same assets
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "uusd".to_string(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Cannot swap the same assets uusd")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap Mars
        // *
        let config = config_state(&mut deps.storage).load().unwrap();
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::Token {
                contract_addr: deps.api.human_address(&config.mars_token_address).unwrap(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Cannot swap Mars"),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap asset with zero balance
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "zero".to_string(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Contract has no balance for the asset zero")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap amount greater than contract balance
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "somecoin".to_string(),
            },
            amount: Some(Uint128(1_000_001)),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "The amount requested for swap exceeds contract balance for the asset somecoin"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // swap
        // *
        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [
                AssetInfo::NativeToken {
                    denom: "somecoin".to_string(),
                },
                AssetInfo::NativeToken {
                    denom: "uusd".to_string(),
                },
            ],
            contract_addr: HumanAddr::from("pair_somecoin_uusd"),
            liquidity_token: HumanAddr::from("lp_somecoin_uusd"),
        });

        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "somecoin".to_string(),
            },
            amount: None,
        };

        let env = mock_env("owner", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("pair_somecoin_uusd"),
                msg: to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset: TerraswapAsset {
                        info: AssetInfo::NativeToken {
                            denom: "somecoin".to_string(),
                        },
                        amount: contract_asset_balance,
                    },
                    belief_price: None,
                    max_spread: Some(config.terraswap_max_spread),
                    to: None,
                })
                .unwrap(),
                send: vec![Coin {
                    denom: "somecoin".to_string(),
                    amount: contract_asset_balance,
                }],
            })]
        );

        assert_eq!(
            res.log,
            vec![log("action", "swap"), log("asset", "somecoin")]
        );
    }

    #[test]
    fn test_swap_asset_to_mars() {
        let mut deps = th_setup(&[]);

        let config = config_state(&mut deps.storage).load().unwrap();
        let config_mars_token_human_addr =
            deps.api.human_address(&config.mars_token_address).unwrap();

        // *
        // can't swap the same assets
        // *
        let msg = HandleMsg::SwapAssetToMars {
            offer_asset_info: AssetInfo::Token {
                contract_addr: config_mars_token_human_addr.clone(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                format!(
                    "Cannot swap the same assets {}",
                    config_mars_token_human_addr.as_str()
                )
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap Mars
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::Token {
                contract_addr: config_mars_token_human_addr.clone(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Cannot swap Mars"),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap asset with zero balance
        // *
        let cw20_contract_address = HumanAddr::from("cw20_zero");
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), Uint128::zero())],
        );

        let msg = HandleMsg::SwapAssetToMars {
            offer_asset_info: AssetInfo::Token {
                contract_addr: cw20_contract_address,
            },
            amount: None,
        };

        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Contract has no balance for the asset cw20_zero")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        let cw20_contract_address = HumanAddr::from("cw20_token");
        let contract_asset_balance = Uint128(1_000_000);
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), contract_asset_balance)],
        );

        // *
        // can't swap amount greater than contract balance
        // *
        let msg = HandleMsg::SwapAssetToMars {
            offer_asset_info: AssetInfo::Token {
                contract_addr: cw20_contract_address.clone(),
            },
            amount: Some(Uint128(1_000_001)),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "The amount requested for swap exceeds contract balance for the asset cw20_token"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // swap
        // *
        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [
                AssetInfo::Token {
                    contract_addr: cw20_contract_address.clone(),
                },
                AssetInfo::Token {
                    contract_addr: config_mars_token_human_addr,
                },
            ],
            contract_addr: HumanAddr::from("pair_cw20_mars"),
            liquidity_token: HumanAddr::from("lp_cw20_mars"),
        });

        let msg = HandleMsg::SwapAssetToMars {
            offer_asset_info: AssetInfo::Token {
                contract_addr: cw20_contract_address.clone(),
            },
            amount: None,
        };

        let env = mock_env("owner", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_address.clone(),
                msg: to_binary(&Cw20HandleMsg::Send {
                    contract: HumanAddr::from("pair_cw20_mars"),
                    amount: contract_asset_balance,
                    msg: Some(
                        to_binary(&TerraswapPairHandleMsg::Swap {
                            offer_asset: TerraswapAsset {
                                info: AssetInfo::Token {
                                    contract_addr: cw20_contract_address.clone(),
                                },
                                amount: contract_asset_balance,
                            },
                            belief_price: None,
                            max_spread: Some(config.terraswap_max_spread),
                            to: None,
                        })
                        .unwrap()
                    ),
                })
                .unwrap(),
                send: vec![],
            })]
        );

        assert_eq!(
            res.log,
            vec![
                log("action", "swap"),
                log("asset", cw20_contract_address.as_str()),
            ]
        );
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let config = CreateOrUpdateConfig {
            mars_token_address: Some(HumanAddr::from("mars_token")),
            terraswap_factory_address: Some(HumanAddr::from("terraswap_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(TEST_COOLDOWN_DURATION),
            unstake_window: Some(TEST_UNSTAKE_WINDOW),
        };
        let msg = InitMsg {
            cw20_code_id: 1,
            config,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let _res = init(&mut deps, env, msg).unwrap();

        let mut config_singleton = config_state(&mut deps.storage);
        let mut config = config_singleton.load().unwrap();
        config.xmars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("xmars_token"))
            .unwrap();
        config_singleton.save(&config).unwrap();

        deps
    }
}
