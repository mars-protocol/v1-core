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
        HandleMsg::BalanceChange {
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
        HandleMsg::ClaimRewards {} => handle_claim_rewards(deps, env),
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
                asset_incentive_update_index(&mut asset_incentive, total_supply, env.block.time)?;

                // Set new emission
                asset_incentive.emission_per_second = emission_per_second;

                asset_incentive
            }
            None => AssetIncentive {
                emission_per_second,
                index: Decimal::zero(),
                last_updated: env.block.time,
            },
        };

    asset_incentives_bucket.save(&ma_asset_canonical_address.as_slice(), &new_asset_incentive)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "set_asset_incentives"),
            log("ma_asset", ma_token_address),
            log("emission_per_second", emission_per_second),
        ],
        data: None,
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
            // If there are no incentives,
            // an empty successful response is returned as the
            // success of the call is needed for the call that triggered the change to
            // succeed and be persisted to state.
            None => return Ok(HandleResponse::default()),
            Some(ai) => ai,
        };

    asset_incentive_update_index(&mut asset_incentive, total_supply_before, env.block.time)?;
    asset_incentives_bucket.save(&ma_token_canonical_address.as_slice(), &asset_incentive)?;

    // Check if user has accumulated uncomputed rewards (which means index is not up to date)
    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let user_asset_index =
        state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
            .may_load(&ma_token_canonical_address.as_slice())?
            .unwrap_or_else(Decimal::zero);

    let mut accrued_rewards = Uint128::zero();

    if user_asset_index != asset_incentive.index {
        // Compute user accrued rewards and update state
        accrued_rewards = user_compute_accrued_rewards(
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
                .unwrap_or_else(Uint128::zero);

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

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "balance_change"),
            log("ma_asset", env.message.sender),
            log("user", user_address),
            log("rewards_accrued", accrued_rewards),
        ],
        data: None,
    })
}

pub fn handle_claim_rewards<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let user_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    let mut accrued_rewards = state::user_unclaimed_rewards_read(&deps.storage)
        .may_load(&user_canonical_address.as_slice())?
        .unwrap_or_else(Uint128::zero);

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
        )?;
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

    // clear unclaimed rewards
    state::user_unclaimed_rewards(&mut deps.storage)
        .save(&user_canonical_address.as_slice(), &Uint128::zero())?;

    let config = state::config_read(&deps.storage).load()?;
    let mars_token_address = deps.api.human_address(&config.mars_token_address)?;
    let staking_address = deps.api.human_address(&config.staking_address)?;

    let messages = if accrued_rewards > Uint128::zero() {
        vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: mars_token_address,
            msg: to_binary(&cw20::Cw20HandleMsg::Send {
                contract: staking_address,
                amount: accrued_rewards,
                msg: Some(to_binary(&mars::staking::msg::ReceiveMsg::Stake {
                    recipient: Some(env.message.sender.clone()),
                })?),
            })?,
            send: vec![],
        })]
    } else {
        vec![]
    };

    Ok(HandleResponse {
        messages,
        log: vec![
            log("action", "claim_rewards"),
            log("user", env.message.sender),
            log("mars_staked_as_rewards", accrued_rewards),
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
) -> StdResult<()> {
    if (current_block_time != asset_incentive.last_updated)
        && !total_supply.is_zero()
        && !asset_incentive.emission_per_second.is_zero()
    {
        asset_incentive.index = asset_incentive_compute_index(
            asset_incentive.index,
            asset_incentive.emission_per_second,
            total_supply,
            asset_incentive.last_updated,
            current_block_time,
        )?
    }
    asset_incentive.last_updated = current_block_time;
    Ok(())
}

fn asset_incentive_compute_index(
    previous_index: Decimal,
    emission_per_second: Uint128,
    total_supply: Uint128,
    time_start: u64,
    time_end: u64,
) -> StdResult<Decimal> {
    if time_start > time_end {
        return Err(StdError::underflow(time_end, time_start));
    }
    let seconds_elapsed = time_end - time_start;
    let new_index = previous_index
        + Decimal::from_ratio(
            emission_per_second.u128() * seconds_elapsed as u128,
            total_supply,
        );
    Ok(new_index)
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

    // init
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

    // SetAssetIncentive

    #[test]
    fn test_only_owner_can_set_asset_incentive() {
        let mut deps = th_setup(&[]);

        let env = mock_env("sender", MockEnvParams::default());

        let msg = HandleMsg::SetAssetIncentive {
            ma_token_address: HumanAddr::from("ma_token"),
            emission_per_second: Uint128(100),
        };

        let res_error = handle(&mut deps, env, msg).unwrap_err();

        assert_eq!(res_error, StdError::unauthorized());
    }

    #[test]
    fn test_set_new_asset_incentive() {
        let mut deps = th_setup(&[]);
        let (ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");

        let env = mock_env(
            "owner",
            MockEnvParams {
                block_time: 1_000_000,
                ..Default::default()
            },
        );
        let msg = HandleMsg::SetAssetIncentive {
            ma_token_address: ma_asset_address,
            emission_per_second: Uint128(100),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "set_asset_incentives"),
                log("ma_asset", "ma_asset"),
                log("emission_per_second", 100),
            ]
        );

        let asset_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();

        assert_eq!(asset_incentive.emission_per_second, Uint128(100));
        assert_eq!(asset_incentive.index, Decimal::zero());
        assert_eq!(asset_incentive.last_updated, 1_000_000);
    }

    #[test]
    fn test_set_existing_asset_incentive() {
        // setup
        let mut deps = th_setup(&[]);
        let (ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");
        let ma_asset_total_supply = Uint128(2_000_000);
        deps.querier
            .set_cw20_total_supply(ma_asset_address.clone(), ma_asset_total_supply);

        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_asset_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second: Uint128(100),
                    index: Decimal::from_ratio(1_u128, 2_u128),
                    last_updated: 500_000,
                },
            )
            .unwrap();

        // handle msg
        let env = mock_env(
            "owner",
            MockEnvParams {
                block_time: 1_000_000,
                ..Default::default()
            },
        );
        let msg = HandleMsg::SetAssetIncentive {
            ma_token_address: ma_asset_address,
            emission_per_second: Uint128(200),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // tests
        assert_eq!(
            res.log,
            vec![
                log("action", "set_asset_incentives"),
                log("ma_asset", "ma_asset"),
                log("emission_per_second", 200),
            ]
        );

        let asset_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();

        let expected_index = asset_incentive_compute_index(
            Decimal::from_ratio(1_u128, 2_u128),
            Uint128(100),
            ma_asset_total_supply,
            500_000,
            1_000_000,
        )
        .unwrap();

        assert_eq!(asset_incentive.emission_per_second, Uint128(200));
        assert_eq!(asset_incentive.index, expected_index);
        assert_eq!(asset_incentive.last_updated, 1_000_000);
    }

    // BalanceChange

    #[test]
    fn test_handle_balance_change_noops() {
        let mut deps = th_setup(&[]);

        // non existing incentive returns a no op
        {
            let env = mock_env("ma_asset", MockEnvParams::default());
            let msg = HandleMsg::BalanceChange {
                user_address: HumanAddr::from("user"),
                user_balance_before: Uint128(100000),
                total_supply_before: Uint128(100000),
            };
            let res = handle(&mut deps, env, msg).unwrap();
            assert_eq!(res, HandleResponse::default())
        }
    }

    #[test]
    fn test_balance_change_zero_emission() {
        let mut deps = th_setup(&[]);
        let (_ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");
        let (_user_address, user_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "user");
        let asset_incentive_index = Decimal::from_ratio(1_u128, 2_u128);

        state::asset_incentives(&mut deps.storage)
            .save(
                &deps
                    .api
                    .canonical_address(&HumanAddr::from("ma_asset"))
                    .unwrap()
                    .as_slice(),
                &AssetIncentive {
                    emission_per_second: Uint128(0),
                    index: asset_incentive_index,
                    last_updated: 500_000,
                },
            )
            .unwrap();

        let env = mock_env(
            "ma_asset",
            MockEnvParams {
                block_time: 600_000,
                ..Default::default()
            },
        );
        let msg = HandleMsg::BalanceChange {
            user_address: HumanAddr::from("user"),
            user_balance_before: Uint128(100_000),
            total_supply_before: Uint128(100_000),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_accrued_rewards =
            user_compute_accrued_rewards(Uint128(100_000), Decimal::zero(), asset_incentive_index)
                .unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "balance_change"),
                log("ma_asset", "ma_asset"),
                log("user", "user"),
                log("rewards_accrued", expected_accrued_rewards),
            ]
        );

        // asset incentive index stays the same
        let asset_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();
        assert_eq!(asset_incentive.index, asset_incentive_index);
        assert_eq!(asset_incentive.last_updated, 600_000);

        // user index is set to asset's index
        let user_asset_index =
            state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
                .load(&ma_asset_canonical_address.as_slice())
                .unwrap();
        assert_eq!(user_asset_index, asset_incentive_index);

        // rewards get updated
        let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
            .load(&user_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_unclaimed_rewards, expected_accrued_rewards)
    }

    #[test]
    fn test_balance_change_user_with_zero_balance() {
        let mut deps = th_setup(&[]);
        let (_ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");
        let (user_address, user_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "user");

        let start_index = Decimal::from_ratio(1_u128, 2_u128);
        let emission_per_second = Uint128(100);
        let total_supply = Uint128(100_000);
        let time_last_updated = 500_000_u64;
        let time_contract_call = 600_000_u64;

        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_asset_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second,
                    index: start_index,
                    last_updated: time_last_updated,
                },
            )
            .unwrap();

        let env = mock_env(
            "ma_asset",
            MockEnvParams {
                block_time: time_contract_call,
                ..Default::default()
            },
        );
        let msg = HandleMsg::BalanceChange {
            user_address,
            user_balance_before: Uint128::zero(),
            total_supply_before: total_supply,
        };
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.log,
            vec![
                log("action", "balance_change"),
                log("ma_asset", "ma_asset"),
                log("user", "user"),
                log("rewards_accrued", 0),
            ]
        );

        let expected_index = asset_incentive_compute_index(
            start_index,
            emission_per_second,
            total_supply,
            time_last_updated,
            time_contract_call,
        )
        .unwrap();

        // asset incentive gets updated
        let asset_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();
        assert_eq!(asset_incentive.index, expected_index);
        assert_eq!(asset_incentive.last_updated, time_contract_call);

        // user index is set to asset's index
        let user_asset_index =
            state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
                .load(&ma_asset_canonical_address.as_slice())
                .unwrap();
        assert_eq!(user_asset_index, expected_index);

        // no new rewards
        let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
            .may_load(&user_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_unclaimed_rewards, None)
    }

    #[test]
    fn test_balance_change_user_non_zero_balance() {
        let mut deps = th_setup(&[]);
        let (_ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");
        let (user_address, user_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "user");

        let emission_per_second = Uint128(100);
        let total_supply = Uint128(100_000);

        let mut expected_asset_incentive_index = Decimal::from_ratio(1_u128, 2_u128);
        let mut expected_time_last_updated = 500_000_u64;
        let mut expected_accumulated_rewards = Uint128::zero();

        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_asset_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second,
                    index: expected_asset_incentive_index,
                    last_updated: expected_time_last_updated,
                },
            )
            .unwrap();

        // first call no previous rewards
        {
            let time_contract_call = 600_000_u64;
            let user_balance = Uint128(10_000);
            let env = mock_env(
                "ma_asset",
                MockEnvParams {
                    block_time: time_contract_call,
                    ..Default::default()
                },
            );
            let msg = HandleMsg::BalanceChange {
                user_address: user_address.clone(),
                user_balance_before: user_balance,
                total_supply_before: total_supply,
            };
            let res = handle(&mut deps, env, msg).unwrap();

            expected_asset_incentive_index = asset_incentive_compute_index(
                expected_asset_incentive_index,
                emission_per_second,
                total_supply,
                expected_time_last_updated,
                time_contract_call,
            )
            .unwrap();

            let expected_accrued_rewards = user_compute_accrued_rewards(
                user_balance,
                Decimal::zero(),
                expected_asset_incentive_index,
            )
            .unwrap();
            assert_eq!(
                res.log,
                vec![
                    log("action", "balance_change"),
                    log("ma_asset", "ma_asset"),
                    log("user", "user"),
                    log("rewards_accrued", expected_accrued_rewards),
                ]
            );

            // asset incentive gets updated
            expected_time_last_updated = time_contract_call;

            let asset_incentive = state::asset_incentives_read(&deps.storage)
                .load(&ma_asset_canonical_address.as_slice())
                .unwrap();
            assert_eq!(asset_incentive.index, expected_asset_incentive_index);
            assert_eq!(asset_incentive.last_updated, expected_time_last_updated);

            // user index is set to asset's index
            let user_asset_index =
                state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
                    .load(&ma_asset_canonical_address.as_slice())
                    .unwrap();
            assert_eq!(user_asset_index, expected_asset_incentive_index);

            // user gets new rewards
            let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
                .load(&user_canonical_address.as_slice())
                .unwrap();
            expected_accumulated_rewards += expected_accrued_rewards;
            assert_eq!(user_unclaimed_rewards, expected_accumulated_rewards)
        }

        // Second call accumulates new rewards
        {
            let time_contract_call = 700_000_u64;
            let user_balance = Uint128(20_000);
            let env = mock_env(
                "ma_asset",
                MockEnvParams {
                    block_time: time_contract_call,
                    ..Default::default()
                },
            );
            let msg = HandleMsg::BalanceChange {
                user_address: user_address.clone(),
                user_balance_before: user_balance,
                total_supply_before: total_supply,
            };
            let res = handle(&mut deps, env, msg).unwrap();

            let previous_user_index = expected_asset_incentive_index;
            expected_asset_incentive_index = asset_incentive_compute_index(
                expected_asset_incentive_index,
                emission_per_second,
                total_supply,
                expected_time_last_updated,
                time_contract_call,
            )
            .unwrap();

            let expected_accrued_rewards = user_compute_accrued_rewards(
                user_balance,
                previous_user_index,
                expected_asset_incentive_index,
            )
            .unwrap();
            assert_eq!(
                res.log,
                vec![
                    log("action", "balance_change"),
                    log("ma_asset", "ma_asset"),
                    log("user", "user"),
                    log("rewards_accrued", expected_accrued_rewards),
                ]
            );

            // asset incentive gets updated
            expected_time_last_updated = time_contract_call;

            let asset_incentive = state::asset_incentives_read(&deps.storage)
                .load(&ma_asset_canonical_address.as_slice())
                .unwrap();
            assert_eq!(asset_incentive.index, expected_asset_incentive_index);
            assert_eq!(asset_incentive.last_updated, expected_time_last_updated);

            // user index is set to asset's index
            let user_asset_index =
                state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
                    .load(&ma_asset_canonical_address.as_slice())
                    .unwrap();
            assert_eq!(user_asset_index, expected_asset_incentive_index);

            // user gets new rewards
            let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
                .load(&user_canonical_address.as_slice())
                .unwrap();
            expected_accumulated_rewards += expected_accrued_rewards;
            assert_eq!(user_unclaimed_rewards, expected_accumulated_rewards)
        }

        // Third call same block does not change anything
        {
            let time_contract_call = 700_000_u64;
            let user_balance = Uint128(20_000);
            let env = mock_env(
                "ma_asset",
                MockEnvParams {
                    block_time: time_contract_call,
                    ..Default::default()
                },
            );
            let msg = HandleMsg::BalanceChange {
                user_address,
                user_balance_before: user_balance,
                total_supply_before: total_supply,
            };
            let res = handle(&mut deps, env, msg).unwrap();
            assert_eq!(
                res.log,
                vec![
                    log("action", "balance_change"),
                    log("ma_asset", "ma_asset"),
                    log("user", "user"),
                    log("rewards_accrued", 0),
                ]
            );

            // asset incentive is still the same
            let asset_incentive = state::asset_incentives_read(&deps.storage)
                .load(&ma_asset_canonical_address.as_slice())
                .unwrap();
            assert_eq!(asset_incentive.index, expected_asset_incentive_index);
            assert_eq!(asset_incentive.last_updated, expected_time_last_updated);

            // user index is still the same
            let user_asset_index =
                state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice())
                    .load(&ma_asset_canonical_address.as_slice())
                    .unwrap();
            assert_eq!(user_asset_index, expected_asset_incentive_index);

            // user gets no new rewards
            let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
                .load(&user_canonical_address.as_slice())
                .unwrap();
            assert_eq!(user_unclaimed_rewards, expected_accumulated_rewards)
        }
    }

    #[test]
    fn test_handle_claim_rewards() {
        // SETUP
        let mut deps = th_setup(&[]);
        let (user_address, user_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "user");

        let previous_unclaimed_rewards = Uint128(50_000);
        let ma_asset_total_supply = Uint128(100_000);
        let ma_asset_user_balance = Uint128(10_000);
        let ma_zero_total_supply = Uint128(200_000);
        let ma_zero_user_balance = Uint128(10_000);
        let time_start = 500_000_u64;
        let time_contract_call = 600_000_u64;

        // addresses
        // ma_asset with ongoing rewards
        let (ma_asset_address, ma_asset_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_asset");
        // ma_asset with no pending rewards but with user index (so it had active incentives
        // at some point)
        let (ma_zero_address, ma_zero_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_zero");
        // ma_asset where the user never had a balance during an active
        // incentive -> hence no associated index
        let (_ma_no_user_address, ma_no_user_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "ma_no_user");

        deps.querier
            .set_cw20_total_supply(ma_asset_address.clone(), ma_asset_total_supply);
        deps.querier
            .set_cw20_total_supply(ma_zero_address.clone(), ma_zero_total_supply);
        deps.querier.set_cw20_balances(
            ma_asset_address,
            &[(user_address.clone(), ma_asset_user_balance)],
        );
        deps.querier.set_cw20_balances(
            ma_zero_address,
            &[(user_address.clone(), ma_zero_user_balance)],
        );

        // incentives
        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_asset_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second: Uint128(100),
                    index: Decimal::one(),
                    last_updated: time_start,
                },
            )
            .unwrap();
        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_zero_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second: Uint128(0),
                    index: Decimal::one(),
                    last_updated: time_start,
                },
            )
            .unwrap();
        state::asset_incentives(&mut deps.storage)
            .save(
                &ma_no_user_canonical_address.as_slice(),
                &AssetIncentive {
                    emission_per_second: Uint128(200),
                    index: Decimal::one(),
                    last_updated: time_start,
                },
            )
            .unwrap();

        // user indices
        let mut user_asset_indices_bucket =
            state::user_asset_indices(&mut deps.storage, &user_canonical_address.as_slice());
        user_asset_indices_bucket
            .save(&ma_asset_canonical_address.as_slice(), &Decimal::one())
            .unwrap();
        user_asset_indices_bucket
            .save(
                &ma_zero_canonical_address.as_slice(),
                &Decimal::from_ratio(1_u128, 2_u128),
            )
            .unwrap();

        // unclaimed_rewards
        state::user_unclaimed_rewards(&mut deps.storage)
            .save(
                &user_canonical_address.as_slice(),
                &previous_unclaimed_rewards,
            )
            .unwrap();

        // MSG
        let env = mock_env(
            "user",
            MockEnvParams {
                block_time: time_contract_call,
                ..Default::default()
            },
        );
        let msg = HandleMsg::ClaimRewards {};
        let res = handle(&mut deps, env, msg).unwrap();

        // ASSERT
        let expected_ma_asset_incentive_index = asset_incentive_compute_index(
            Decimal::one(),
            Uint128(100),
            ma_asset_total_supply,
            time_start,
            time_contract_call,
        )
        .unwrap();

        let expected_ma_asset_accrued_rewards = user_compute_accrued_rewards(
            ma_asset_user_balance,
            Decimal::one(),
            expected_ma_asset_incentive_index,
        )
        .unwrap();

        let expected_ma_zero_accrued_rewards = user_compute_accrued_rewards(
            ma_zero_user_balance,
            Decimal::from_ratio(1_u128, 2_u128),
            Decimal::one(),
        )
        .unwrap();

        let expected_accrued_rewards = previous_unclaimed_rewards
            + expected_ma_asset_accrued_rewards
            + expected_ma_zero_accrued_rewards;

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                msg: to_binary(&cw20::Cw20HandleMsg::Send {
                    contract: HumanAddr::from("staking"),
                    amount: expected_accrued_rewards,
                    msg: Some(
                        to_binary(&mars::staking::msg::ReceiveMsg::Stake {
                            recipient: Some(user_address),
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
                log("action", "claim_rewards"),
                log("user", "user"),
                log("mars_staked_as_rewards", expected_accrued_rewards),
            ]
        );

        // ma_asset and ma_zero incentives get updated, ma_no_user does not
        let ma_asset_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();
        assert_eq!(ma_asset_incentive.index, expected_ma_asset_incentive_index);
        assert_eq!(ma_asset_incentive.last_updated, time_contract_call);

        let ma_zero_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_zero_canonical_address.as_slice())
            .unwrap();
        assert_eq!(ma_zero_incentive.index, Decimal::one());
        assert_eq!(ma_zero_incentive.last_updated, time_contract_call);

        let ma_no_user_incentive = state::asset_incentives_read(&deps.storage)
            .load(&ma_no_user_canonical_address.as_slice())
            .unwrap();
        assert_eq!(ma_no_user_incentive.index, Decimal::one());
        assert_eq!(ma_no_user_incentive.last_updated, time_start);

        let user_asset_index_bucket =
            state::user_asset_indices_read(&deps.storage, &user_canonical_address.as_slice());

        // user's ma_asset and ma_zero indices are updated
        let user_ma_asset_index = user_asset_index_bucket
            .load(&ma_asset_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_ma_asset_index, expected_ma_asset_incentive_index);

        let user_ma_zero_index = user_asset_index_bucket
            .load(&ma_zero_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_ma_zero_index, Decimal::one());

        // user's ma_no_user does not get updated
        let user_ma_no_user_index = user_asset_index_bucket
            .may_load(&ma_no_user_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_ma_no_user_index, None);

        // user rewards are cleared
        let user_unclaimed_rewards = state::user_unclaimed_rewards_read(&deps.storage)
            .load(&user_canonical_address.as_slice())
            .unwrap();
        assert_eq!(user_unclaimed_rewards, Uint128::zero())
    }

    #[test]
    fn test_claim_zero_rewards() {
        // SETUP
        let mut deps = th_setup(&[]);

        let env = mock_env("user", MockEnvParams::default());
        let msg = HandleMsg::ClaimRewards {};
        let res = handle(&mut deps, env, msg).unwrap();

        // ASSERT
        assert_eq!(res.messages.len(), 0);

        assert_eq!(
            res.log,
            vec![
                log("action", "claim_rewards"),
                log("user", "user"),
                log("mars_staked_as_rewards", 0),
            ]
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
        let env = mock_env("somebody", MockEnvParams::default());
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

    #[test]
    fn test_asset_incentive_compute_index() {
        assert_eq!(
            asset_incentive_compute_index(
                Decimal::zero(),
                Uint128(100),
                Uint128(200_000),
                1000,
                10
            ),
            Err(StdError::underflow(10, 1000)),
        );

        assert_eq!(
            asset_incentive_compute_index(Decimal::zero(), Uint128(100), Uint128(200_000), 0, 1000)
                .unwrap(),
            Decimal::from_ratio(1_u128, 2_u128)
        );
        assert_eq!(
            asset_incentive_compute_index(
                Decimal::from_ratio(1_u128, 2_u128),
                Uint128(2000),
                Uint128(5_000_000),
                20_000,
                30_000
            )
            .unwrap(),
            Decimal::from_ratio(9_u128, 2_u128)
        );
    }

    #[test]
    fn test_user_compute_accrued_rewards() {
        assert_eq!(
            user_compute_accrued_rewards(
                Uint128::zero(),
                Decimal::one(),
                Decimal::from_ratio(2_u128, 1_u128)
            )
            .unwrap(),
            Uint128::zero()
        );

        assert_eq!(
            user_compute_accrued_rewards(
                Uint128(100),
                Decimal::zero(),
                Decimal::from_ratio(2_u128, 1_u128)
            )
            .unwrap(),
            Uint128(200)
        );
        assert_eq!(
            user_compute_accrued_rewards(
                Uint128(100),
                Decimal::one(),
                Decimal::from_ratio(2_u128, 1_u128)
            )
            .unwrap(),
            Uint128(100)
        );
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
