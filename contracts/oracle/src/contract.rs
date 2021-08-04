use cosmwasm_std::{
    attr, entry_point, to_binary, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult, SubMsg,
};

use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg};
use crate::state::{Config, CONFIG};
use mars::error::MarsError;
use mars::asset::{Asset, AssetType};

// INIT

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    // initialize Config
    let config = Config {
        owner: deps.api.addr_validate(&msg.owner)?,
        astroport_factory_address: deps.api.addr_validate(&msg.astroport_factory_address)?,
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response {
        messages: vec![],
        attributes: vec![],
        events: vec![],
        data: None,
    })
}

// HANDLERS

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, MarsError> {
    match msg {
        ExecuteMsg::UpdateConfig {
            owner,
            astroport_factory_address,
        } => execute_update_config(
            deps,
            env,
            info,
            owner,
            astroport_factory_address,
        )
    }
}

pub fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    owner: Option<String>,
    astroport_factory_address: Option<String>,
) -> Result<Response, MarsError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    };

    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;
    config.astroport_factory_address = option_string_to_addr(
        deps.api,
        astroport_factory_address,
        config.astroport_factory_address,
    )?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response {
        attributes: vec![attr("action", "update_config")],
        ..Response::default(),
    })
}

// QUERIES

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::AssetPrice {asset} => to_binary(&query_config(deps, asset)?),
        QueryMsg::AssetPrices {assets} => to_binary(&query_config(deps, assets)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        owner: config.owner,
    })
}

fn query_asset_price(deps: Deps, _env: Env, asset: Asset) {
}

fn get_native_asset_prices(
    querier: &QuerierWrapper,
    assets_to_query: &[String],
) -> StdResult<Vec<(String, Decimal256)>> {
    let mut asset_prices: Vec<(String, Decimal256)> = vec![];

    if !assets_to_query.is_empty() {
        let assets_to_query: Vec<&str> = assets_to_query.iter().map(AsRef::as_ref).collect(); // type conversion
        let querier = TerraQuerier::new(querier);
        let asset_prices_in_uusd = querier
            .query_exchange_rates("uusd", assets_to_query)?
            .exchange_rates;
        for rate in asset_prices_in_uusd {
            asset_prices.push((rate.quote_denom, Decimal256::from(rate.exchange_rate)));
        }
    }

    Ok(asset_prices)
}

fn get_cw20_price(
    querier: &QuerierWrapper,
    asset: Asset) -> StdResult(Decimal256) {
}

// MIGRATION

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::mock_info;
    use cosmwasm_std::{Addr, BankMsg, Coin, CosmosMsg, Uint128};
    use mars::testing::{mock_dependencies, mock_env, MockEnvParams};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: String::from("owner"),
            astroport_factory_address: String::from("astroport"),
        };
        let info = mock_info("owner", &[]);

        let res =
            instantiate(deps.as_mut(), mock_env(MockEnvParams::default()), info, msg).unwrap();
        let empty_vec: Vec<SubMsg> = vec![];
        assert_eq!(empty_vec, res.messages);

        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(Addr::unchecked("owner"), config.owner);
        assert_eq!(Addr::unchecked("astroport"), config.astroport_factory_address);
    }
}
