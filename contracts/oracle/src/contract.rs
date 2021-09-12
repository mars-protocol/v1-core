use cosmwasm_std::{
    entry_point, to_binary, Binary, Decimal, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};
use terra_cosmwasm::TerraQuerier;

use mars::asset::Asset;
use mars::error::MarsError;
use mars::helpers::option_string_to_addr;

use mars::oracle::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use mars::oracle::{PriceSourceChecked, PriceSourceUnchecked};

use crate::state::{Config, PriceConfig, CONFIG, PRICE_CONFIGS};

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
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
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
        ExecuteMsg::UpdateConfig { owner } => execute_update_config(deps, env, info, owner),

        ExecuteMsg::SetAsset {
            asset,
            price_source,
        } => execute_set_asset(deps, env, info, asset, price_source),
    }
}

pub fn execute_set_asset(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset: Asset,
    price_source_unchecked: PriceSourceUnchecked,
) -> Result<Response, MarsError> {
    let config = CONFIG.load(deps.storage)?;

    let asset_reference = asset.get_reference();

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    }

    let price_source: PriceSourceChecked = match price_source_unchecked {
        PriceSourceUnchecked::Native { denom } => PriceSourceChecked::Native { denom },
        PriceSourceUnchecked::TerraswapUusdPair { pair_address } => {
            PriceSourceChecked::TerraswapUusdPair {
                pair_address: deps.api.addr_validate(&pair_address)?,
            }
        }
        PriceSourceUnchecked::Fixed { price } => PriceSourceChecked::Fixed { price },
    };

    PRICE_CONFIGS.save(
        deps.storage,
        asset_reference.as_slice(),
        &PriceConfig { price_source },
    )?;

    Ok(Response::default())
}

pub fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    owner: Option<String>,
) -> Result<Response, MarsError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {});
    };

    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

// QUERIES

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps, env)?),
        QueryMsg::AssetPriceByReference { asset_reference } => {
            to_binary(&query_asset_price(deps, env, asset_reference)?)
        }
        QueryMsg::AssetPrice { asset } => {
            let asset_reference = asset.get_reference();
            to_binary(&query_asset_price(deps, env, asset_reference)?)
        }
        QueryMsg::AssetPriceConfig { asset } => {
            to_binary(&query_asset_price_config(deps, env, asset)?)
        }
    }
}

fn query_config(deps: Deps, _env: Env) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        owner: config.owner.into(),
    })
}

fn query_asset_price(deps: Deps, _env: Env, asset_reference: Vec<u8>) -> StdResult<Decimal> {
    let price_config = PRICE_CONFIGS.load(deps.storage, asset_reference.as_slice())?;

    match price_config.price_source {
        PriceSourceChecked::Native { denom } => {
            let terra_querier = TerraQuerier::new(&deps.querier);
            // NOTE: Exchange rate returns how much of the quote (second argument) is required to
            // buy one unit of the base_denom (first argument).
            // We want to know how much uusd we need to buy 1 of the target currency
            let asset_prices_query = terra_querier
                .query_exchange_rates(denom, vec!["uusd".to_string()])?
                .exchange_rates
                .pop();

            match asset_prices_query {
                Some(exchange_rate_item) => Ok(exchange_rate_item.exchange_rate),
                None => Err(StdError::generic_err("No native price found")),
            }
        }

        PriceSourceChecked::TerraswapUusdPair { .. } => {
            // TODO: implement
            Ok(Decimal::one())
        }

        PriceSourceChecked::Fixed { price } => Ok(price),
    }
}

fn query_asset_price_config(deps: Deps, _env: Env, asset: Asset) -> StdResult<PriceConfig> {
    let (_, asset_reference, _) = asset.get_attributes();
    let price_config = PRICE_CONFIGS.load(deps.storage, asset_reference.as_slice())?;

    Ok(price_config)
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_info, MockApi, MockStorage};
    use cosmwasm_std::{from_binary, Addr, OwnedDeps};
    use mars::testing::{mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: String::from("owner"),
        };
        let info = mock_info("owner", &[]);

        let res =
            instantiate(deps.as_mut(), mock_env(MockEnvParams::default()), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(Addr::unchecked("owner"), config.owner);
    }

    #[test]
    fn test_update_config() {
        let mut deps = th_setup();
        let env = mock_env(MockEnvParams::default());

        // only owner can update
        {
            let msg = ExecuteMsg::UpdateConfig {
                owner: Some(String::from("new_owner")),
            };
            let info = mock_info("another_one", &[]);
            let err = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
            assert_eq!(err, MarsError::Unauthorized {}.into());
        }

        let info = mock_info("owner", &[]);
        // no change
        {
            let msg = ExecuteMsg::UpdateConfig { owner: None };
            execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();

            let config = CONFIG.load(&deps.storage).unwrap();
            assert_eq!(config.owner, Addr::unchecked("owner"));
        }

        // new owner
        {
            let msg = ExecuteMsg::UpdateConfig {
                owner: Some(String::from("new_owner")),
            };
            execute(deps.as_mut(), env, info, msg).unwrap();

            let config = CONFIG.load(&deps.storage).unwrap();
            assert_eq!(config.owner, Addr::unchecked("new_owner"));
        }
    }

    #[test]
    fn test_set_asset() {
        let mut deps = th_setup();
        let env = mock_env(MockEnvParams::default());

        // only owner can set asset
        {
            let msg = ExecuteMsg::SetAsset {
                asset: Asset::Native {
                    denom: "luna".to_string(),
                },
                price_source: PriceSourceUnchecked::Native {
                    denom: "luna".to_string(),
                },
            };
            let info = mock_info("another_one", &[]);
            let err = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
            assert_eq!(err, MarsError::Unauthorized {}.into());
        }

        let info = mock_info("owner", &[]);
        // native
        {
            let asset = Asset::Native {
                denom: String::from("luna"),
            };
            let reference = asset.get_reference();
            let msg = ExecuteMsg::SetAsset {
                asset: asset,
                price_source: PriceSourceUnchecked::Native {
                    denom: "luna".to_string(),
                },
            };
            execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
            let price_config = PRICE_CONFIGS
                .load(&deps.storage, reference.as_slice())
                .unwrap();
            assert_eq!(
                price_config.price_source,
                PriceSourceChecked::Native {
                    denom: "luna".to_string()
                }
            );
        }

        // cw20 terraswap
        {
            let asset = Asset::Cw20 {
                contract_addr: String::from("token"),
            };
            let reference = asset.get_reference();
            let msg = ExecuteMsg::SetAsset {
                asset: asset,
                price_source: PriceSourceUnchecked::TerraswapUusdPair {
                    pair_address: "token".to_string(),
                },
            };
            execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
            let price_config = PRICE_CONFIGS
                .load(&deps.storage, reference.as_slice())
                .unwrap();
            assert_eq!(
                price_config.price_source,
                PriceSourceChecked::TerraswapUusdPair {
                    pair_address: Addr::unchecked("token")
                }
            );
        }

        // cw20 fixed
        {
            let asset = Asset::Cw20 {
                contract_addr: String::from("token"),
            };
            let reference = asset.get_reference();
            let msg = ExecuteMsg::SetAsset {
                asset: asset,
                price_source: PriceSourceUnchecked::Fixed {
                    price: Decimal::from_ratio(1_u128, 2_u128),
                },
            };
            execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
            let price_config = PRICE_CONFIGS
                .load(&deps.storage, reference.as_slice())
                .unwrap();
            assert_eq!(
                price_config.price_source,
                PriceSourceChecked::Fixed {
                    price: Decimal::from_ratio(1_u128, 2_u128)
                }
            );
        }
    }

    #[test]
    fn test_query_price_native() {
        let mut deps = th_setup();
        let asset = Asset::Native {
            denom: String::from("nativecoin"),
        };
        let reference = asset.get_reference();

        deps.querier.set_native_exchange_rates(
            "nativecoin".to_string(),
            &[("uusd".to_string(), Decimal::from_ratio(4_u128, 1_u128))],
        );

        PRICE_CONFIGS
            .save(
                &mut deps.storage,
                reference.as_slice(),
                &PriceConfig {
                    price_source: PriceSourceChecked::Native {
                        denom: "nativecoin".to_string(),
                    },
                },
            )
            .unwrap();

        let env = mock_env(MockEnvParams::default());
        let query: Decimal = from_binary(
            &query(
                deps.as_ref(),
                env,
                QueryMsg::AssetPriceByReference {
                    asset_reference: b"nativecoin".to_vec(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(query, Decimal::from_ratio(4_u128, 1_u128));
    }

    #[test]
    fn test_query_price_fixed() {
        let mut deps = th_setup();
        let asset = Asset::Cw20 {
            contract_addr: String::from("cw20token"),
        };
        let reference = asset.get_reference();

        PRICE_CONFIGS
            .save(
                &mut deps.storage,
                reference.as_slice(),
                &PriceConfig {
                    price_source: PriceSourceChecked::Fixed {
                        price: Decimal::from_ratio(3_u128, 2_u128),
                    },
                },
            )
            .unwrap();

        let env = mock_env(MockEnvParams::default());
        let query: Decimal = from_binary(
            &query(
                deps.as_ref(),
                env,
                QueryMsg::AssetPriceByReference {
                    asset_reference: Addr::unchecked("cw20token").as_bytes().to_vec(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(query, Decimal::from_ratio(3_u128, 2_u128));
    }

    // TEST_HELPERS
    fn th_setup() -> OwnedDeps<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: String::from("owner"),
        };
        let info = mock_info("owner", &[]);
        instantiate(deps.as_mut(), mock_env(MockEnvParams::default()), info, msg).unwrap();

        deps
    }
}
