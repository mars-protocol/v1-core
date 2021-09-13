use cosmwasm_std::{
    entry_point, to_binary, Binary, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult, Uint128,
};
use terraswap::asset::AssetInfo;

use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, CONFIG};

use mars::error::MarsError;
use mars::helpers::option_string_to_addr;
use mars::swapping::execute_swap;

// INIT

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    // initialize Config
    let config = Config {
        owner: deps.api.addr_validate(&msg.owner)?,
        terraswap_factory_address: deps.api.addr_validate(&msg.terraswap_factory_address)?,
        terraswap_max_spread: msg.terraswap_max_spread,
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new())
}

// HANDLERS

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, MarsError> {
    match msg {
        ExecuteMsg::ExecuteCosmosMsg(cosmos_msg) => {
            execute_execute_cosmos_msg(deps, env, info, cosmos_msg)
        }
        ExecuteMsg::UpdateConfig {
            owner,
            terraswap_factory_address,
            terraswap_max_spread,
        } => execute_update_config(
            deps,
            env,
            info,
            owner,
            terraswap_factory_address,
            terraswap_max_spread,
        ),
        ExecuteMsg::SwapAssetToUusd {
            offer_asset_info,
            amount,
        } => Ok(execute_swap_asset_to_uusd(
            deps,
            env,
            offer_asset_info,
            amount,
        )?),
    }
}

/// Execute Cosmos message
pub fn execute_execute_cosmos_msg(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: CosmosMsg,
) -> Result<Response, MarsError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    }

    let response = Response::new()
        .add_message(msg)
        .add_attribute("action", "execute_cosmos_msg");

    Ok(response)
}

pub fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    owner: Option<String>,
    terraswap_factory_address: Option<String>,
    terraswap_max_spread: Option<Decimal>,
) -> Result<Response, MarsError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    };

    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;
    config.terraswap_factory_address = option_string_to_addr(
        deps.api,
        terraswap_factory_address,
        config.terraswap_factory_address,
    )?;
    config.terraswap_max_spread = terraswap_max_spread.unwrap_or(config.terraswap_max_spread);

    CONFIG.save(deps.storage, &config)?;

    let response = Response::new().add_attribute("action", "update_config");

    Ok(response)
}

/// Swap any asset on the contract to uusd
pub fn execute_swap_asset_to_uusd(
    deps: DepsMut,
    env: Env,
    offer_asset_info: AssetInfo,
    amount: Option<Uint128>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    let ask_asset_info = AssetInfo::NativeToken {
        denom: "uusd".to_string(),
    };

    let terraswap_max_spread = Some(config.terraswap_max_spread);

    execute_swap(
        deps,
        env,
        offer_asset_info,
        ask_asset_info,
        amount,
        config.terraswap_factory_address,
        terraswap_max_spread,
    )
}

// QUERIES

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(ConfigResponse {
        owner: config.owner,
    })
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::ExecuteMsg::UpdateConfig;
    use cosmwasm_std::{
        attr,
        testing::{mock_dependencies, mock_env, mock_info},
        Addr, BankMsg, Coin, Decimal, SubMsg, Uint128,
    };

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: String::from("owner"),
            terraswap_factory_address: String::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
        let info = mock_info("owner", &[]);

        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        let empty_vec: Vec<SubMsg> = vec![];
        assert_eq!(empty_vec, res.messages);

        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(config.owner, Addr::unchecked("owner"));
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(&[]);

        // *
        // init config with valid params
        // *
        let msg = InstantiateMsg {
            owner: String::from("owner"),
            terraswap_factory_address: String::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
        let info = mock_info("owner", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            owner: None,
            terraswap_factory_address: None,
            terraswap_max_spread: None,
        };
        let info = mock_info("somebody", &[]);
        let error_res = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {});

        // *
        // update config with all new params
        // *
        let msg = UpdateConfig {
            owner: Some(String::from("new_owner")),
            terraswap_factory_address: Some(String::from("new_factory")),
            terraswap_max_spread: Some(Decimal::from_ratio(10u128, 100u128)),
        };
        let info = mock_info("owner", &[]);
        // we can just call .unwrap() to assert this was a success
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = CONFIG.load(&deps.storage).unwrap();

        assert_eq!(new_config.owner, Addr::unchecked("new_owner"));
        assert_eq!(
            new_config.terraswap_factory_address,
            Addr::unchecked("new_factory")
        );
        assert_eq!(
            new_config.terraswap_max_spread,
            Decimal::from_ratio(10u128, 100u128)
        );
    }

    #[test]
    fn test_execute_cosmos_msg() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: String::from("owner"),
            terraswap_factory_address: String::from("terraswap_factory"),
            terraswap_max_spread: Decimal::from_ratio(1u128, 100u128),
        };
        let info = mock_info("owner", &[]);
        let _res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

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
        assert_eq!(error_res, MarsError::Unauthorized {});

        // *
        // can execute Cosmos msg
        // *
        let info = mock_info("owner", &[]);
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(res.messages, vec![SubMsg::new(cosmos_msg)]);
        assert_eq!(res.attributes, vec![attr("action", "execute_cosmos_msg")]);
    }
}
