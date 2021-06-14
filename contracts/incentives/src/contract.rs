use cosmwasm_std::{
    log, to_binary, Api, Binary, CosmosMsg, Decimal, Env, Extern, HandleResponse, HumanAddr,
    InitResponse, MigrateResponse, MigrateResult, Querier, StdError, StdResult, Storage, Uint128,
};

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg, SetAssetIncentive};

use crate::state;
use crate::state::{AssetIncentive, Config};

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    _msg: InitMsg,
) -> StdResult<InitResponse> {
    // initialize Config
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
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
        HandleMsg::SetAssetIncentives {
            set_asset_incentives,
        } => handle_set_asset_incentives(deps, env, set_asset_incentives),
        HandleMsg::HandleBalanceChange {
            user_address,
            user_balance_before,
            total_supply_before,
        } => handle_balance_change(deps, env, user_address, user_balance_before, total_supply_before),
        HandleMsg::ExecuteCosmosMsg(cosmos_msg) => handle_execute_cosmos_msg(deps, env, cosmos_msg),
    }
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
    

    // Compute asset new index and update state
    if !(
            env.block.time == asset_incentive.last_updated ||
            total_supply_before.is_zero() ||
            asset_incentive.emission_per_second.is_zero()
        )
    {
        let seconds_elapsed = env.block.time - asset_incentive.last_updated;
        let new_index =
            asset_incentive.index +
            Decimal::from_ratio(
                asset_incentive.emission_per_second.u128() * seconds_elapsed as u128,
                total_supply_before
            );

        asset_incentive.index = new_index;
    }
    asset_incentive.last_updated = env.block.time;
    asset_incentives_bucket.save(&ma_token_canonical_address.as_slice(), &asset_incentive)?;

    // Check if user has accumulated uncomputed rewards (which means index is not up to date)
    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let asset_user_index =
        state::asset_user_indices_read(&deps.storage, &ma_token_canonical_address.as_slice())
            .may_load(&user_canonical_address.as_slice())?
            .unwrap_or(Decimal::zero());
    if asset_user_index != asset_incentive.index {
        // Compute user accrued rewards and update state
        let accrued_rewards = 
            ((user_balance_before * asset_incentive.index) - (user_balance_before * asset_user_index))?;

        if !accrued_rewards.is_zero() {
            let mut user_unclaimed_rewards_bucket =
                state::user_unclaimed_rewards(&mut deps.storage);
            let current_unclaimed_rewards = user_unclaimed_rewards_bucket
                .may_load(&user_canonical_address.as_slice())?
                .unwrap_or(Uint128::zero());

            user_unclaimed_rewards_bucket.save(
                &user_canonical_address.as_slice(), 
                &(current_unclaimed_rewards + accrued_rewards)
            )?
        }

        state::asset_user_indices(&mut deps.storage, &ma_token_canonical_address.as_slice())
            .save(&user_canonical_address.as_slice(), &asset_incentive.index)?
    }

    
    Ok(HandleResponse::default())
}

pub fn handle_set_asset_incentives<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    set_asset_incentives: Vec<SetAssetIncentive>,
) -> StdResult<HandleResponse> {
    let mut asset_incentives_bucket = state::asset_incentives(&mut deps.storage);
    for set_asset_incentive in set_asset_incentives {
        let ma_asset_canonical_address = deps
            .api
            .canonical_address(&set_asset_incentive.ma_token_address)?;
        let new_asset_incentive =
            match asset_incentives_bucket.may_load(&ma_asset_canonical_address.as_slice())? {
                Some(mut asset_incentive) => {
                    // TODO: Update index up to now
                    asset_incentive.emission_per_second = set_asset_incentive.emission_per_second;
                    asset_incentive.last_updated = env.block.time;
                    asset_incentive
                }
                None => AssetIncentive {
                    emission_per_second: set_asset_incentive.emission_per_second,
                    index: Decimal::zero(),
                    last_updated: env.block.time,
                },
            };
        asset_incentives_bucket
            .save(&ma_asset_canonical_address.as_slice(), &new_asset_incentive)?;
    }

    //TODO: Maybe log this?
    Ok(HandleResponse::default())
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
    use cosmwasm_std::{BankMsg, Coin, CosmosMsg, HumanAddr, Uint128};
    use mars::testing::{mock_dependencies, mock_env, MockEnvParams};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {};
        let env = mock_env("owner", MockEnvParams::default());

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
    }

    #[test]
    fn test_execute_cosmos_msg() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {};
        let env = mock_env("owner", MockEnvParams::default());
        let _res = init(&mut deps, env, msg).unwrap();

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
}
