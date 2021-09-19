#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    from_binary, to_binary, Addr, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};
use terraswap::asset::AssetInfo;

use crate::error::ContractError;
use crate::types::{Config, Claim, GlobalState, SlashEvent};
use crate::state::{CONFIG, COOLDOWNS, GLOBAL_STATE};

use crate::msg::{
    ConfigResponse, CooldownResponse, CreateOrUpdateConfig, ExecuteMsg, InstantiateMsg, QueryMsg,
    ReceiveMsg,
};
use mars::address_provider;
use mars::address_provider::msg::MarsContract;
use mars::error::MarsError;
use mars::helpers::{cw20_get_balance, cw20_get_total_supply, option_string_to_addr, zero_address};
use mars::swapping::execute_swap;

// INIT

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        owner,
        cooldown_duration,
        address_provider_address,
        astroport_factory_address,
        astroport_max_spread,
    } = msg.config;

    // All fields should be available
    let available = owner.is_some()
        && cooldown_duration.is_some()
        && address_provider_address.is_some()
        && astroport_factory_address.is_some()
        && astroport_max_spread.is_some()

    if !available {
        return Err(MarsError::InstantiateParamsUnavailable {}.into());
    };

    // Initialize config
    let config = Config {
        owner: option_string_to_addr(deps.api, owner, zero_address())?,
        cooldown_duration: cooldown_duration.unwrap(),
        address_provider_address: option_string_to_addr(
            deps.api,
            address_provider_address,
            zero_address(),
        )?,
        astroport_factory_address: option_string_to_addr(
            deps.api,
            astroport_factory_address,
            zero_address(),
        )?,
        astroport_max_spread: astroport_max_spread.unwrap(),
    };

    CONFIG.save(deps.storage, &config)?;

    // Initialize global state
    GLOBAL_STATE.save(
        deps.storage,
        &GlobalState { total_mars_on_open_claims: Uint128::zero() },
    )?;

    Ok(Response::default())
}

// HANDLERS

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(cw20_msg) => Ok(execute_receive_cw20(deps, env, info, cw20_msg)?),

        ExecuteMsg::UpdateConfig { config } => Ok(execute_update_config(deps, info, config)?),

        ExecuteMsg::Claim { recipient } => Ok(execute_claim(deps, env, info, recipient)?),

        ExecuteMsg::SwapAssetToUusd {
            offer_asset_info,
            amount,
        } => Ok(execute_swap_asset_to_uusd(
            deps,
            env,
            offer_asset_info,
            amount,
        )?),

        ExecuteMsg::SwapUusdToMars { amount } => Ok(execute_swap_uusd_to_mars(deps, env, amount)?),

        ExecuteMsg::ExecuteCosmosMsg(cosmos_msg) => {
            Ok(execute_execute_cosmos_msg(deps, info, cosmos_msg)?)
        }
    }
}

/// cw20 receive implementation
pub fn execute_receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    match from_binary(&cw20_msg.msg)? {
        ReceiveMsg::Stake { recipient } => {
            execute_stake(deps, env, info, cw20_msg.sender, recipient, cw20_msg.amount)
        }

        ReceiveMsg::Unstake { } => {
            execute_unstake(deps, env, info, cw20_msg.sender, cw20_msg.amount)
        }
    }
}

/// Update config
pub fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    new_config: CreateOrUpdateConfig,
) -> Result<Response, MarsError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    }

    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        owner,
        cooldown_duration,
        address_provider_address,
        astroport_factory_address,
        astroport_max_spread,
    } = new_config;

    // Update config
    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;
    config.address_provider_address = option_string_to_addr(
        deps.api,
        address_provider_address,
        config.address_provider_address,
    )?;
    config.astroport_factory_address = option_string_to_addr(
        deps.api,
        astroport_factory_address,
        config.astroport_factory_address,
    )?;
    config.astroport_max_spread = astroport_max_spread.unwrap_or(config.astroport_max_spread);
    config.cooldown_duration = cooldown_duration.unwrap_or(config.cooldown_duration);

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}


/// Mint xMars tokens to staker
pub fn execute_stake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    staker: String,
    option_recipient: Option<String>,
    stake_amount: Uint128,
) -> Result<Response, ContractError> {
    // check stake is valid
    let config = CONFIG.load(deps.storage)?;
    let (mars_token_address, xmars_token_address) = get_token_addresses(&deps, &config)?;

    // Has to send Mars tokens
    if info.sender != mars_token_address {
        return Err(MarsError::Unauthorized {}.into());
    }
    if stake_amount.is_zero() {
        return Err(ContractError::StakeAmountZero {});
    }


    let total_mars_in_staking_contract =
        cw20_get_balance(&deps.querier, mars_token_address, env.contract.address)?;

    // Mars amount needs to be before the stake transaction (which is already in the staking contract's
    // balance so it needs to be deducted)
    let net_total_mars_in_staking_contract =
        total_mars_in_staking_contract.checked_sub(stake_amount)?;

    let global_state = GLOBAL_STATE.load(deps.storage)?;
    let total_mars_for_stakers =
        net_total_mars_in_staking_contract
            .checked_sub(global_state.total_mars_for_claimers)?;

    let total_xmars_supply = cw20_get_total_supply(&deps.querier, xmars_token_address.clone())?;

    let mint_amount =
        if total_mars_for_stakers.is_zero() || total_xmars_supply.is_zero() {
            stake_amount
        } else {
            stake_amount.multiply_ratio(total_xmars_supply, total_mars_for_stakers)
        };

    let recipient = option_recipient.unwrap_or_else(|| staker.clone());

    let res = Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: xmars_token_address.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: recipient.clone(),
                amount: mint_amount,
            })?,
        }))
        .add_attribute("action", "stake")
        .add_attribute("staker", staker)
        .add_attribute("recipient", recipient)
        .add_attribute("mars_staked", stake_amount)
        .add_attribute("xmars_minted", mint_amount);

    Ok(res)
}

pub fn execute_unstake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    staker: String,
    burn_amount: Uint128,
) -> Result<Response, ContractError> {
    // check if unstake is valid
    let config = CONFIG.load(deps.storage)?;
    let (mars_token_address, xmars_token_address) = get_token_addresses(&deps, &config)?;
    if info.sender != xmars_token_address {
        return Err(MarsError::Unauthorized {}.into());
    }
    if burn_amount.is_zero() {
        return Err(ContractError::UnstakeAmountZero {});
    }

    // check valid cooldown
    let staker_addr = deps.api.addr_validate(&staker)?;

    let total_mars_in_staking_contract = cw20_get_balance(
        &deps.querier,
        mars_token_address.clone(),
        env.contract.address,
    )?;

    let mut global_state = GLOBAL_STATE.load(deps.storage)?

    let total_mars_for_stakers =
        net_total_mars_in_staking_contract
            .checked_sub(global_state.total_mars_for_claimers)?;

    let total_xmars_supply =
        cw20_get_total_supply(&deps.querier, xmars_token_address.clone())?;

    let claimable_amount =
        burn_amount.multiply_ratio(total_mars_for_stakers, total_xmars_supply);

    let timestamp_start = env.block.time.seconds();

    let claim = Claim {
        timestamp_start,
        timestamp_unlocked: timestamp_start + config.cooldown_duration,
        amount: claimable_amount,
    }

    CLAIMS.save(deps.storage, &claim);

    global_state.total_mars_for_claimers += claimable_amount;
     
    GLOBAL_STATE.save(deps.storage, &global_state)?;

    let res = Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: xmars_token_address.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Burn {
                amount: burn_amount,
            })?,
        }))
        .add_attribute("action", "unstake")
        .add_attribute("staker", staker)
        .add_attribute("mars_claimable", unstake_amount)
        .add_attribute("xmars_burned", burn_amount);
    Ok(res)
}

pub fn execute_claim(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    option_recipient: Option<String>,
) -> Result<Response, ContractError> {
    let res = Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: mars_token_address.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: recipient.clone(),
                amount: unstake_amount,
            })?,
        }))
    Ok(res)
}


/// Execute Cosmos message
pub fn execute_execute_cosmos_msg(
    deps: DepsMut,
    info: MessageInfo,
    msg: CosmosMsg,
) -> Result<Response, MarsError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    }

    let res = Response::new()
        .add_message(msg)
        .add_attribute("action", "execute_cosmos_msg");
    Ok(res)
}

/// Swap any asset on the contract to uusd
pub fn execute_swap_asset_to_uusd(
    deps: DepsMut,
    env: Env,
    offer_asset_info: AssetInfo,
    amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    // throw error if the user tries to swap Mars
    let mars_token_address = address_provider::helpers::query_address(
        &deps.querier,
        config.address_provider_address,
        MarsContract::MarsToken,
    )?;

    if let AssetInfo::Token { contract_addr } = offer_asset_info.clone() {
        if contract_addr == mars_token_address {
            return Err(ContractError::MarsCannotSwap {});
        }
    }

    let ask_asset_info = AssetInfo::NativeToken {
        denom: "uusd".to_string(),
    };

    let astroport_max_spread = Some(config.astroport_max_spread);

    Ok(execute_swap(
        deps,
        env,
        offer_asset_info,
        ask_asset_info,
        amount,
        config.astroport_factory_address,
        astroport_max_spread,
    )?)
}

/// Swap uusd on the contract to Mars
pub fn execute_swap_uusd_to_mars(
    deps: DepsMut,
    env: Env,
    amount: Option<Uint128>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    let offer_asset_info = AssetInfo::NativeToken {
        denom: "uusd".to_string(),
    };

    let mars_token_address = address_provider::helpers::query_address(
        &deps.querier,
        config.address_provider_address,
        MarsContract::MarsToken,
    )?;

    let ask_asset_info = AssetInfo::Token {
        contract_addr: mars_token_address.to_string(),
    };

    let astroport_max_spread = Some(config.astroport_max_spread);

    execute_swap(
        deps,
        env,
        offer_asset_info,
        ask_asset_info,
        amount,
        config.astroport_factory_address,
        astroport_max_spread,
    )
}

// QUERIES

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Cooldown { user_address } => to_binary(&query_cooldown(deps, user_address)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        owner: config.owner.to_string(),
        address_provider_address: config.address_provider_address.to_string(),
        astroport_max_spread: config.astroport_max_spread,
        cooldown_duration: config.cooldown_duration,
        unstake_window: config.unstake_window,
    })
}

fn query_cooldown(deps: Deps, user_address: String) -> StdResult<CooldownResponse> {
    let cooldown = COOLDOWNS.may_load(deps.storage, &deps.api.addr_validate(&user_address)?)?;

    match cooldown {
        Some(result) => Ok(CooldownResponse {
            timestamp: result.timestamp,
            amount: result.amount,
        }),
        None => Result::Err(StdError::not_found("No cooldown found")),
    }
}

// HELPERS

/// Gets mars and xmars token addresses from address provider and returns them in a tuple.
fn get_token_addresses(deps: &DepsMut, config: &Config) -> Result<(Addr, Addr), ContractError> {
    let mut addresses_query = address_provider::helpers::query_addresses(
        &deps.querier,
        config.address_provider_address.clone(),
        vec![MarsContract::MarsToken, MarsContract::XMarsToken],
    )?;
    let xmars_token_address = addresses_query.pop().unwrap();
    let mars_token_address = addresses_query.pop().unwrap();

    Ok((mars_token_address, xmars_token_address))
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{
        attr,
        testing::{mock_env, mock_info},
        Addr, BankMsg, Coin, CosmosMsg, Decimal, OwnedDeps, SubMsg, Timestamp,
    };
    use mars::testing::{self, mock_dependencies, MarsMockQuerier, MockEnvParams};

    use crate::msg::ExecuteMsg::UpdateConfig;
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};

    const TEST_COOLDOWN_DURATION: u64 = 1000;
    const TEST_UNSTAKE_WINDOW: u64 = 100;

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        // *
        // init config with empty params
        // *
        let empty_config = CreateOrUpdateConfig {
            owner: None,
            address_provider_address: None,
            astroport_factory_address: None,
            astroport_max_spread: None,
            cooldown_duration: None,
            unstake_window: None,
        };
        let msg = InstantiateMsg {
            config: empty_config,
        };
        let info = mock_info("owner", &[]);
        let response = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap_err();
        assert_eq!(
            response,
            ContractError::Mars(MarsError::InstantiateParamsUnavailable {})
        );

        let config = CreateOrUpdateConfig {
            owner: Some(String::from("owner")),
            address_provider_address: Some(String::from("address_provider")),
            astroport_factory_address: Some(String::from("astroport_factory")),
            astroport_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(20),
            unstake_window: Some(10),
        };
        let msg = InstantiateMsg { config };

        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        let config = CONFIG.load(deps.as_ref().storage).unwrap();
        assert_eq!(config.owner, Addr::unchecked("owner"));
        assert_eq!(
            config.address_provider_address,
            Addr::unchecked("address_provider")
        );
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(&[]);

        // *
        // init config with valid params
        // *
        let init_config = CreateOrUpdateConfig {
            owner: Some(String::from("owner")),
            address_provider_address: Some(String::from("address_provider")),
            astroport_factory_address: Some(String::from("astroport_factory")),
            astroport_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(20),
            unstake_window: Some(10),
        };
        let msg = InstantiateMsg {
            config: init_config.clone(),
        };
        let info = mock_info("owner", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            config: init_config,
        };
        let info = mock_info("somebody", &[]);
        let error_res = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
        assert_eq!(error_res, ContractError::Mars(MarsError::Unauthorized {}));

        // *
        // update config with all new params
        // *
        let config = CreateOrUpdateConfig {
            owner: Some(String::from("new_owner")),
            address_provider_address: Some(String::from("new_address_provider")),
            astroport_factory_address: Some(String::from("new_factory")),
            astroport_max_spread: Some(Decimal::from_ratio(2u128, 100u128)),
            cooldown_duration: Some(200),
            unstake_window: Some(100),
        };
        let msg = UpdateConfig {
            config: config.clone(),
        };
        let info = mock_info("owner", &[]);
        // we can just call .unwrap() to assert this was a success
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = CONFIG.load(deps.as_ref().storage).unwrap();

        assert_eq!(new_config.owner, "new_owner");
        assert_eq!(new_config.address_provider_address, "new_address_provider");
        assert_eq!(new_config.astroport_factory_address, "new_factory");
        assert_eq!(
            new_config.cooldown_duration,
            config.cooldown_duration.unwrap()
        );
        assert_eq!(new_config.unstake_window, config.unstake_window.unwrap());
    }

    #[test]
    fn test_staking() {
        let mut deps = th_setup(&[]);
        let staker_addr = Addr::unchecked("staker");

        // no Mars in pool
        // stake X Mars -> should receive X xMars
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: String::from("staker"),
            amount: Uint128::new(2_000_000),
            msg: to_binary(&ReceiveMsg::Stake { recipient: None }).unwrap(),
        });

        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked(MOCK_CONTRACT_ADDR), Uint128::new(2_000_000))],
        );

        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::zero());

        let info = mock_info("mars_token", &[]);
        let res = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

        assert_eq!(
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("xmars_token"),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: String::from("staker"),
                    amount: Uint128::new(2_000_000),
                })
                .unwrap(),
            }))],
            res.messages
        );
        assert_eq!(
            vec![
                attr("action", "stake"),
                attr("staker", String::from("staker")),
                attr("recipient", String::from("staker")),
                attr("mars_staked", 2_000_000.to_string()),
                attr("xmars_minted", 2_000_000.to_string()),
            ],
            res.attributes
        );

        // Some Mars in pool and some xMars supply
        // stake Mars -> should receive less xMars
        // set recipient -> should send xMars to recipient
        let stake_amount = Uint128::new(2_000_000);
        let mars_in_basecamp = Uint128::new(4_000_000);
        let xmars_supply = Uint128::new(1_000_000);

        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::Stake {
                recipient: Some(String::from("recipient")),
            })
            .unwrap(),

            sender: String::from("staker"),
            amount: stake_amount,
        });

        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked(MOCK_CONTRACT_ADDR), mars_in_basecamp)],
        );

        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), xmars_supply);

        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        let expected_minted_xmars =
            stake_amount.multiply_ratio(xmars_supply, mars_in_basecamp - stake_amount);

        assert_eq!(
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("xmars_token"),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: String::from("recipient"),
                    amount: expected_minted_xmars,
                })
                .unwrap(),
            }))],
            res.messages
        );
        assert_eq!(
            vec![
                attr("action", "stake"),
                attr("staker", String::from("staker")),
                attr("recipient", String::from("recipient")),
                attr("mars_staked", stake_amount),
                attr("xmars_minted", expected_minted_xmars),
            ],
            res.attributes
        );

        // stake other token -> Unauthorized
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: String::from("staker"),
            amount: Uint128::new(2_000_000),
            msg: to_binary(&ReceiveMsg::Stake { recipient: None }).unwrap(),
        });

        let info = mock_info("other_token", &[]);
        let res_error = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
        assert_eq!(res_error, ContractError::Mars(MarsError::Unauthorized {}));

        // setup variables for unstake
        let unstake_amount = Uint128::new(1_000_000);
        let unstake_mars_in_basecamp = Uint128::new(4_000_000);
        let unstake_xmars_supply = Uint128::new(3_000_000);
        let unstake_block_timestamp = 1_000_000_000;
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::Unstake { recipient: None }).unwrap(),
            sender: String::from("staker"),
            amount: unstake_amount,
        });

        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                unstake_mars_in_basecamp,
            )],
        );

        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), unstake_xmars_supply);

        // unstake Mars no cooldown -> unauthorized
        let info = mock_info("xmars_token", &[]);
        let env = testing::mock_env(MockEnvParams {
            block_time: Timestamp::from_seconds(unstake_block_timestamp),
            ..Default::default()
        });
        let response = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
        assert_eq!(response, ContractError::UnstakeNoCooldown {});

        // unstake Mars expired cooldown -> unauthorized
        COOLDOWNS
            .save(
                deps.as_mut().storage,
                &staker_addr,
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp
                        - TEST_COOLDOWN_DURATION
                        - TEST_UNSTAKE_WINDOW
                        - 1,
                },
            )
            .unwrap();

        let response = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
        assert_eq!(response, ContractError::UnstakeCooldownExpired {});

        // unstake Mars unfinished cooldown -> unauthorized
        COOLDOWNS
            .save(
                deps.as_mut().storage,
                &staker_addr,
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION + 1,
                },
            )
            .unwrap();

        let response = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
        assert_eq!(response, ContractError::UnstakeCooldownNotFinished {});

        // unstake Mars cooldown with low amount -> unauthorized
        COOLDOWNS
            .save(
                deps.as_mut().storage,
                &staker_addr,
                &Cooldown {
                    amount: unstake_amount - Uint128::new(1000),
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION,
                },
            )
            .unwrap();

        let response = execute(deps.as_mut(), env.clone(), info.clone(), msg.clone()).unwrap_err();
        assert_eq!(response, ContractError::UnstakeAmountTooLarge {});

        // partial unstake Mars valid cooldown -> burn xMars, receive Mars back,
        // deduct cooldown amount
        let pending_cooldown_amount = Uint128::new(300_000);
        let pending_cooldown_timestamp = unstake_block_timestamp - TEST_COOLDOWN_DURATION;

        COOLDOWNS
            .save(
                deps.as_mut().storage,
                &staker_addr,
                &Cooldown {
                    amount: unstake_amount + pending_cooldown_amount,
                    timestamp: pending_cooldown_timestamp,
                },
            )
            .unwrap();

        let res = execute(deps.as_mut(), env.clone(), info, msg.clone()).unwrap();
        let expected_returned_mars =
            unstake_amount.multiply_ratio(unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("xmars_token"),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Burn {
                        amount: unstake_amount,
                    })
                    .unwrap(),
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("mars_token"),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: String::from("staker"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                })),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                attr("action", "unstake"),
                attr("staker", String::from("staker")),
                attr("recipient", String::from("staker")),
                attr("mars_unstaked", expected_returned_mars),
                attr("xmars_burned", unstake_amount),
            ],
            res.attributes
        );

        let actual_cooldown = COOLDOWNS.load(deps.as_ref().storage, &staker_addr).unwrap();

        assert_eq!(actual_cooldown.amount, pending_cooldown_amount);
        assert_eq!(actual_cooldown.timestamp, pending_cooldown_timestamp);

        // unstake other token -> Unauthorized
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::Unstake { recipient: None }).unwrap(),
            sender: String::from("staker"),
            amount: pending_cooldown_amount,
        });

        let info = mock_info("other_token", &[]);
        let res_error = execute(deps.as_mut(), env.clone(), info, msg.clone()).unwrap_err();
        assert_eq!(res_error, ContractError::Mars(MarsError::Unauthorized {}));

        // unstake pending amount Mars -> cooldown is deleted
        let info = mock_info("xmars_token", &[]);
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::Unstake {
                recipient: Some(String::from("recipient")),
            })
            .unwrap(),
            sender: String::from("staker"),
            amount: pending_cooldown_amount,
        });
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        // NOTE: In reality the mars/xmars amounts would change but since they are being
        // mocked it does not really matter here.
        let expected_returned_mars =
            pending_cooldown_amount.multiply_ratio(unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("xmars_token"),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Burn {
                        amount: pending_cooldown_amount,
                    })
                    .unwrap(),
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("mars_token"),
                    funds: vec![],
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: String::from("recipient"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                })),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                attr("action", "unstake"),
                attr("staker", String::from("staker")),
                attr("recipient", String::from("recipient")),
                attr("mars_unstaked", expected_returned_mars),
                attr("xmars_burned", pending_cooldown_amount),
            ],
            res.attributes
        );

        let actual_cooldown = COOLDOWNS
            .may_load(deps.as_ref().storage, &staker_addr)
            .unwrap();

        assert_eq!(actual_cooldown, None);
    }

    #[test]
    fn test_cooldown() {
        let mut deps = th_setup(&[]);
        let staker_addr = Addr::unchecked("staker");

        let initial_block_time = 1_600_000_000;
        let ongoing_cooldown_block_time = initial_block_time + TEST_COOLDOWN_DURATION / 2;

        // staker with no xmars is unauthorized
        deps.querier.set_cw20_balances(
            Addr::unchecked("xmars_token"),
            &[(staker_addr.clone(), Uint128::zero())],
        );
        let msg = ExecuteMsg::Cooldown {};
        let info = mock_info("staker", &[]);
        let res_error = execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap_err();
        assert_eq!(res_error, ContractError::Mars(MarsError::Unauthorized {}));

        // staker with xmars gets a cooldown equal to the xmars balance
        let initial_xmars_balance = Uint128::new(1_000_000);
        deps.querier.set_cw20_balances(
            Addr::unchecked("xmars_token"),
            &[(staker_addr.clone(), initial_xmars_balance)],
        );

        let env = testing::mock_env(MockEnvParams {
            block_time: Timestamp::from_seconds(initial_block_time),
            ..Default::default()
        });
        let res = execute(deps.as_mut(), env, info.clone(), ExecuteMsg::Cooldown {}).unwrap();

        let cooldown = COOLDOWNS.load(deps.as_ref().storage, &staker_addr).unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);
        assert_eq!(
            vec![
                attr("action", "cooldown"),
                attr("user", "staker"),
                attr("cooldown_amount", initial_xmars_balance.to_string()),
                attr("cooldown_timestamp", initial_block_time.to_string())
            ],
            res.attributes
        );

        // same amount does not alter cooldown
        let env = testing::mock_env(MockEnvParams {
            block_time: Timestamp::from_seconds(ongoing_cooldown_block_time),
            ..Default::default()
        });
        let _res = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            ExecuteMsg::Cooldown {},
        )
        .unwrap();

        let cooldown = COOLDOWNS.load(deps.as_ref().storage, &staker_addr).unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);

        // additional amount gets a weighted average timestamp with the new amount
        let additional_xmars_balance = Uint128::new(500_000);

        deps.querier.set_cw20_balances(
            Addr::unchecked("xmars_token"),
            &[(
                staker_addr.clone(),
                initial_xmars_balance + additional_xmars_balance,
            )],
        );
        let _res = execute(deps.as_mut(), env, info.clone(), ExecuteMsg::Cooldown {}).unwrap();

        let cooldown = COOLDOWNS.load(deps.as_ref().storage, &staker_addr).unwrap();

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
        let expired_balance =
            initial_xmars_balance + additional_xmars_balance + Uint128::new(800_000);
        deps.querier.set_cw20_balances(
            Addr::unchecked("xmars_token"),
            &[(staker_addr.clone(), expired_balance)],
        );

        let env = testing::mock_env(MockEnvParams {
            block_time: Timestamp::from_seconds(expired_cooldown_block_time),
            ..Default::default()
        });
        let _res = execute(deps.as_mut(), env, info.clone(), ExecuteMsg::Cooldown {}).unwrap();

        let cooldown = COOLDOWNS.load(deps.as_ref().storage, &staker_addr).unwrap();

        assert_eq!(cooldown.timestamp, expired_cooldown_block_time);
        assert_eq!(cooldown.amount, expired_balance);
    }

    #[test]
    fn test_execute_cosmos_msg() {
        let mut deps = th_setup(&[]);

        let bank = BankMsg::Send {
            to_address: "destination".to_string(),
            amount: vec![Coin {
                denom: "uluna".to_string(),
                amount: Uint128::new(123456),
            }],
        };
        let cosmos_msg = CosmosMsg::Bank(bank);
        let msg = ExecuteMsg::ExecuteCosmosMsg(cosmos_msg.clone());

        // *
        // non owner is not authorized
        // *
        let info = mock_info("somebody", &[]);
        let error_res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap_err();
        assert_eq!(error_res, ContractError::Mars(MarsError::Unauthorized {}));

        // *
        // can execute Cosmos msg
        // *
        let info = mock_info("owner", &[]);
        let res = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap();
        assert_eq!(res.messages, vec![SubMsg::new(cosmos_msg)]);
        assert_eq!(res.attributes, vec![attr("action", "execute_cosmos_msg")]);
    }

    #[test]
    fn test_cannot_swap_mars() {
        let mut deps = th_setup(&[]);
        // *
        // can't swap Mars with SwapAssetToUusd
        // *
        let msg = ExecuteMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::Token {
                contract_addr: String::from("mars_token"),
            },
            amount: None,
        };
        let info = mock_info("owner", &[]);
        let response = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
        assert_eq!(response, ContractError::MarsCannotSwap {});
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> OwnedDeps<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(contract_balances);

        let config = CreateOrUpdateConfig {
            owner: Some(String::from("owner")),
            address_provider_address: Some(String::from("address_provider")),
            astroport_factory_address: Some(String::from("astroport_factory")),
            astroport_max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
            cooldown_duration: Some(TEST_COOLDOWN_DURATION),
            unstake_window: Some(TEST_UNSTAKE_WINDOW),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner", &[]);
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        deps
    }
}
