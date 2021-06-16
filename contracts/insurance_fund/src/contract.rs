use cosmwasm_std::{
    log, to_binary, Api, Binary, CosmosMsg, Decimal, Env, Extern, HandleResponse, HumanAddr,
    InitResponse, MigrateResponse, MigrateResult, Querier, StdError, StdResult, Storage, Uint128,
};

use crate::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg};
use crate::state::{config_state, config_state_read, Config};
use mars::helpers::human_addr_into_canonical;
use mars::swapping::handle_swap;
use terraswap::asset::AssetInfo;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // initialize Config
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        terraswap_factory_address: deps.api.canonical_address(&msg.terraswap_factory_address)?,
        terraswap_max_spread: msg.terraswap_max_spread,
    };

    config_state(&mut deps.storage).save(&config)?;

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
        HandleMsg::ExecuteCosmosMsg(cosmos_msg) => handle_execute_cosmos_msg(deps, env, cosmos_msg),
        HandleMsg::UpdateConfig {
            owner,
            terraswap_factory_address,
            terraswap_max_spread,
        } => handle_update_config(
            deps,
            env,
            owner,
            terraswap_factory_address,
            terraswap_max_spread,
        ),
        HandleMsg::SwapAssetToUusd {
            offer_asset_info,
            amount,
        } => handle_swap_asset_to_uusd(deps, env, offer_asset_info, amount),
    }
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

pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    terraswap_factory_address: Option<HumanAddr>,
    terraswap_max_spread: Option<Decimal>,
) -> StdResult<HandleResponse> {
    let mut config_singleton = config_state(&mut deps.storage);
    let mut config = config_singleton.load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    };

    config.owner = human_addr_into_canonical(deps.api, owner, config.owner)?;
    config.terraswap_factory_address = human_addr_into_canonical(
        deps.api,
        terraswap_factory_address,
        config.terraswap_factory_address,
    )?;
    config.terraswap_max_spread = terraswap_max_spread.unwrap_or(config.terraswap_max_spread);
    config_singleton.save(&config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![log("action", "update_config")],
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

    let terraswap_factory_human_addr = deps.api.human_address(&config.terraswap_factory_address)?;
    let terraswap_max_spread = Some(config.terraswap_max_spread);

    handle_swap(
        deps,
        env,
        offer_asset_info,
        ask_asset_info,
        amount,
        terraswap_factory_human_addr,
        terraswap_max_spread,
    )
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
    use cosmwasm_std::{BankMsg, Coin, CosmosMsg, Decimal, HumanAddr, Uint128};
    use mars::testing::{mock_dependencies, mock_env, MockEnvParams};

    use crate::msg::HandleMsg::UpdateConfig;
    use crate::state::config_state_read;

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            terraswap_factory_address: HumanAddr::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
        let env = mock_env("owner", MockEnvParams::default());

        let res = init(&mut deps, env, msg).unwrap();
        let empty_vec: Vec<CosmosMsg> = vec![];
        assert_eq!(empty_vec, res.messages);

        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("owner"))
                .unwrap(),
            config.owner
        );
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with valid params
        // *
        let msg = InitMsg {
            terraswap_factory_address: HumanAddr::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let _res = init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            owner: None,
            terraswap_factory_address: None,
            terraswap_max_spread: None,
        };
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // update config with all new params
        // *
        let msg = UpdateConfig {
            owner: Some(HumanAddr::from("new_owner")),
            terraswap_factory_address: Some(HumanAddr::from("new_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(10u128, 100u128)),
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
            new_config.terraswap_factory_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_factory"))
                .unwrap()
        );
        assert_eq!(
            new_config.terraswap_max_spread,
            Decimal::from_ratio(10u128, 100u128)
        );
    }

    #[test]
    fn test_execute_cosmos_msg() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            terraswap_factory_address: HumanAddr::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
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
