use cosmwasm_std::{
    log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Decimal, Env, Extern, HandleResponse,
    HumanAddr, InitResponse, MigrateResponse, MigrateResult, Order, Querier, QueryRequest,
    StdError, StdResult, Storage, Uint128, WasmMsg, WasmQuery,
};

use mars::helpers::human_addr_into_canonical;

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg};
use crate::state;
use crate::state::{AssetIncentive, Config};

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // initialize Config
    let config = Config {
        owner: deps.api.canonical_address(&msg.owner)?,
        staking_address: deps.api.canonical_address(&msg.staking_address)?,
        mars_token_address: deps.api.canonical_address(&msg.mars_token_address)?,
    };

    state::config(&mut deps.storage).save(&config)?;

    Ok(InitResponse {
        log: vec![],
        messages: vec![],
    })
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::SetAssetIncentive {
            ma_token_address,
            emission_per_second,
        } => handle_set_asset_incentive(deps, env, ma_token_address, emission_per_second),
        HandleMsg::HandleBalanceChange {
            user_address,
            user_balance_before,
            total_supply_before,
        } => handle_balance_change(
            deps,
            env,
            user_address,
            user_balance_before,
            total_supply_before,
        ),
        HandleMsg::ClaimRewards => handle_claim_rewards(deps, env),
        HandleMsg::UpdateConfig {
            owner,
            mars_token_address,
            staking_address,
        } => handle_update_config(deps, env, owner, mars_token_address, staking_address),
        HandleMsg::ExecuteCosmosMsg(cosmos_msg) => handle_execute_cosmos_msg(deps, env, cosmos_msg),
    }
}

pub fn handle_set_asset_incentive<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    ma_token_address: HumanAddr,
    emission_per_second: Uint128,
) -> StdResult<HandleResponse> {
    // only owner can call this
    let owner = state::config_read(&deps.storage).load()?.owner;
    if deps.api.canonical_address(&env.message.sender)? != owner {
        return Err(StdError::unauthorized());
    }

    let ma_asset_canonical_address = deps.api.canonical_address(&ma_token_address)?;

    let mut asset_incentives_bucket = state::asset_incentives(&mut deps.storage);
    let new_asset_incentive =
        match asset_incentives_bucket.may_load(&ma_asset_canonical_address.as_slice())? {
            Some(mut asset_incentive) => {
                // Update index up to now
                let total_supply =
                    mars::helpers::cw20_get_total_supply(&deps.querier, ma_token_address.clone())?;
                asset_incentive_update_index(&mut asset_incentive, total_supply, env.block.time);

                // Set new emission
                asset_incentive.emission_per_second = emission_per_second;

                asset_incentive
            }
            None => AssetIncentive {
                emission_per_second: emission_per_second,
                index: Decimal::zero(),
                last_updated: env.block.time,
            },
        };

    asset_incentives_bucket.save(&ma_asset_canonical_address.as_slice(), &new_asset_incentive)?;

    Ok(HandleResponse {
        messages: vec![],
        data: None,
        log: vec![
            log("action", "set_asset_incentives"),
            log("ma_asset", ma_token_address),
            log("emission_per_second", emission_per_second),
            log("asset_index", new_asset_incentive.index),
        ],
    })
}

pub fn handle_balance_change<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    user_address: HumanAddr,
    user_balance_before: Uint128,
    total_supply_before: Uint128,
) -> StdResult<HandleResponse> {
    let ma_token_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    let mut asset_incentives_bucket = state::asset_incentives(&mut deps.storage);
    let mut asset_incentive =
        match asset_incentives_bucket.may_load(ma_token_canonical_address.as_slice())? {
            // If there are no incentives, an empty successful response is returned as the
            // success of the call is needed to for the call that triggered the change to
            // succeed and be persisted to state.
            None => return Ok(HandleResponse::default()),
            Some(ai) if ai.emission_per_second.is_zero() => return Ok(HandleResponse::default()),
            Some(ai) => ai,
        };

    asset_incentive_update_index(&mut asset_incentive, total_supply_before, env.block.time);
    asset_incentives_bucket.save(&ma_token_canonical_address.as_slice(), &asset_incentive)?;

    // Check if user has accumulated uncomputed rewards (which means index is not up to date)
    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let user_asset_index =
        state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
            .may_load(&ma_token_canonical_address.as_slice())?
            .unwrap_or(Decimal::zero());
    if user_asset_index != asset_incentive.index {
        // Compute user accrued rewards and update state
        let accrued_rewards = user_compute_accrued_rewards(
            user_balance_before,
            user_asset_index,
            asset_incentive.index,
        )?;

        // Store user accrued rewards as unclaimed
        if !accrued_rewards.is_zero() {
            let mut user_unclaimed_rewards_bucket =
                state::user_unclaimed_rewards(&mut deps.storage);
            let current_unclaimed_rewards = user_unclaimed_rewards_bucket
                .may_load(&user_canonical_address.as_slice())?
                .unwrap_or(Uint128::zero());

            user_unclaimed_rewards_bucket.save(
                &user_canonical_address.as_slice(),
                &(current_unclaimed_rewards + accrued_rewards),
            )?
        }

        state::user_asset_indices(&mut deps.storage, &user_canonical_address.as_slice()).save(
            &ma_token_canonical_address.as_slice(),
            &asset_incentive.index,
        )?
    }

    Ok(HandleResponse::default())
}

pub fn handle_claim_rewards<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let user_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    let unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
        .may_load(&user_canonical_address.as_slice())?
        .unwrap_or(Uint128::zero());
    let mut accrued_rewards = unclaimed_rewards;
    // clear unclaimed rewards
    state::user_unclaimed_rewards(&mut deps.storage)
        .save(&user_canonical_address.as_slice(), &Uint128::zero())?;

    // Since we need the mutable storage reference while iterating we need to get all
    // values on the range first
    let result_user_asset_indices: Vec<StdResult<(Vec<u8>, Decimal)>> =
        state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
            .range(None, None, Order::Ascending)
            .collect();

    for result_kv_pair in result_user_asset_indices {
        let (ma_token_canonical_address_vec, user_asset_index) = result_kv_pair?;
        let ma_token_canonical_address = CanonicalAddr::from(ma_token_canonical_address_vec);
        let ma_token_address = deps.api.human_address(&ma_token_canonical_address)?;
        // Get asset user balances and total supply
        let balance_and_total_supply: mars::ma_token::msg::BalanceAndTotalSupplyResponse =
            deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: ma_token_address,
                msg: to_binary(&mars::ma_token::msg::QueryMsg::BalanceAndTotalSupply {
                    address: env.message.sender.clone(),
                })?,
            }))?;

        // Get asset pending rewards
        let mut asset_incentives_bucket = state::asset_incentives(&mut deps.storage);
        let mut asset_incentive =
            asset_incentives_bucket.load(ma_token_canonical_address.as_slice())?;
        asset_incentive_update_index(
            &mut asset_incentive,
            balance_and_total_supply.total_supply,
            env.block.time,
        );
        asset_incentives_bucket.save(&ma_token_canonical_address.as_slice(), &asset_incentive)?;

        if user_asset_index != asset_incentive.index {
            // Compute user accrued rewards and update user index
            let asset_accrued_rewards = user_compute_accrued_rewards(
                balance_and_total_supply.balance,
                user_asset_index,
                asset_incentive.index,
            )?;
            accrued_rewards += asset_accrued_rewards;

            state::user_asset_indices(&mut deps.storage, &user_canonical_address.as_slice()).save(
                &ma_token_canonical_address.as_slice(),
                &asset_incentive.index,
            )?
        }
    }

    let config = state::config_read(&deps.storage).load()?;
    let mars_token_address = deps.api.human_address(&config.mars_token_address)?;
    let staking_address = deps.api.human_address(&config.staking_address)?;

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: mars_token_address,
            msg: to_binary(&cw20::Cw20HandleMsg::Send {
                contract: staking_address,
                amount: accrued_rewards,
                msg: Some(to_binary(&mars::staking::msg::ReceiveMsg::Stake {
                    recipient: Some(env.message.sender.clone()),
                })?),
            })?,
            send: vec![],
        })],
        log: vec![
            log("action", "claim_rewards"),
            log("user", env.message.sender),
        ],
        data: None,
    })
}

pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    mars_token_address: Option<HumanAddr>,
    staking_address: Option<HumanAddr>,
) -> StdResult<HandleResponse> {
    let mut config_singleton = state::config(&mut deps.storage);
    let mut config = config_singleton.load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    };

    config.owner = human_addr_into_canonical(deps.api, owner, config.owner)?;
    config.mars_token_address =
        human_addr_into_canonical(deps.api, mars_token_address, config.mars_token_address)?;
    config.staking_address =
        human_addr_into_canonical(deps.api, staking_address, config.staking_address)?;

    config_singleton.save(&config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "update_config")],
        data: None,
    })
}

pub fn handle_execute_cosmos_msg<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: CosmosMsg,
) -> StdResult<HandleResponse> {
    let config = state::config_read(&deps.storage).load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    Ok(HandleResponse {
        messages: vec![msg],
        log: vec![log("action", "execute_cosmos_msg")],
        data: None,
    })
}

// HELPERS

/// Updates asset incentive index and last updated timestamp by computing
/// how many rewards were accrued since last time updated given incentive's
/// emission per second.
/// Total supply is the total (liquidity) token supply during the period being computed.
/// Note that this method does not commit updates to state as that should be handled by the
/// caller
fn asset_incentive_update_index(
    asset_incentive: &mut AssetIncentive,
    total_supply: Uint128,
    current_block_time: u64,
) {
    if !(current_block_time == asset_incentive.last_updated
        || total_supply.is_zero()
        || asset_incentive.emission_per_second.is_zero())
    {
        let seconds_elapsed = current_block_time - asset_incentive.last_updated;
        let new_index = asset_incentive.index
            + Decimal::from_ratio(
                asset_incentive.emission_per_second.u128() * seconds_elapsed as u128,
                total_supply,
            );

        asset_incentive.index = new_index;
    }
    asset_incentive.last_updated = current_block_time;
}

/// Computes user accrued rewards using the difference between asset_incentive index and
/// user current index
/// asset_incentives index should be up to date.
fn user_compute_accrued_rewards(
    user_balance: Uint128,
    user_asset_index: Decimal,
    asset_incentive_index: Decimal,
) -> StdResult<Uint128> {
    (user_balance * asset_incentive_index) - (user_balance * user_asset_index)
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
    let config = state::config_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
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
    use cosmwasm_std::testing::{MockApi, MockStorage};
    use cosmwasm_std::{BankMsg, Coin, CosmosMsg, HumanAddr, Uint128};
    use mars::testing::{mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            owner: HumanAddr::from("owner"),
            staking_address: HumanAddr::from("staking"),
            mars_token_address: HumanAddr::from("mars_token"),
        };
        let env = mock_env("sender", MockEnvParams::default());

        let res = init(&mut deps, env, msg).unwrap();
        let empty_vec: Vec<CosmosMsg> = vec![];
        assert_eq!(empty_vec, res.messages);

        let config = state::config_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("owner"))
                .unwrap(),
            config.owner
        );
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("staking"))
                .unwrap(),
            config.staking_address
        );
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
    }

    #[test]
    fn test_update_config() {
        let mut deps = th_setup(&[]);

        // *
        // non owner is not authorized
        // *
        let msg = HandleMsg::UpdateConfig {
            owner: None,
            mars_token_address: None,
            staking_address: None,
        };
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // update config with new params
        // *
        let msg = HandleMsg::UpdateConfig {
            owner: Some(HumanAddr::from("new_owner")),
            mars_token_address: None,
            staking_address: Some(HumanAddr::from("new_staking")),
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);

        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = state::config_read(&deps.storage).load().unwrap();

        assert_eq!(
            new_config.owner,
            deps.api
                .canonical_address(&HumanAddr::from("new_owner"))
                .unwrap()
        );
        assert_eq!(
            new_config.mars_token_address,
            deps.api
                .canonical_address(&HumanAddr::from("mars_token")) // should not change
                .unwrap()
        );
        assert_eq!(
            new_config.staking_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_staking"))
                .unwrap()
        );
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

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        let msg = InitMsg {
            owner: HumanAddr::from("owner"),
            staking_address: HumanAddr::from("staking"),
            mars_token_address: HumanAddr::from("mars_token"),
        };
        let env = mock_env("owner", MockEnvParams::default());
        init(&mut deps, env, msg).unwrap();

        deps
    }
}
