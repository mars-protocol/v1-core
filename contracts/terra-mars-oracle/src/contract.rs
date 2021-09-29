use cosmwasm_std::{
    entry_point, to_binary, Addr, Binary, Deps, DepsMut, Env, Event, MessageInfo, QuerierWrapper,
    QueryRequest, Response, StdError, StdResult, Uint128, WasmQuery,
};
use terra_cosmwasm::TerraQuerier;

use mars::asset::Asset;
use mars::helpers::option_string_to_addr;

use mars::oracle::msg::{AssetPriceResponse, ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use mars::oracle::{PriceSourceChecked, PriceSourceUnchecked};

use crate::state::{
    AstroportTwapData, Config, PriceConfig, ASTROPORT_TWAP_DATA, CONFIG, PRICE_CONFIGS,
};

// Once astroport package is published on crates.io, update Cargo.toml and change these to
// use astroport::asset::{...};
// and
// use astroport::pair::{...};
use crate::astroport::asset::{Asset as AstroportAsset, AssetInfo as AstroportAssetInfo};
use crate::astroport::pair::{
    CumulativePricesResponse, QueryMsg as AstroportQueryMsg, SimulationResponse,
};
use mars::math::decimal::Decimal;

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
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::UpdateConfig { owner } => execute_update_config(deps, env, info, owner),
        ExecuteMsg::SetAsset {
            asset,
            price_source,
        } => execute_set_asset(deps, env, info, asset, price_source),
        ExecuteMsg::UpdateAstroportTwapData { assets } => {
            execute_update_astroport_twap_data(deps, env, info, assets)
        }
    }
}

pub fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    owner: Option<String>,
) -> StdResult<Response> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(StdError::generic_err("Only owner can update config"));
    };

    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

pub fn execute_set_asset(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset: Asset,
    price_source_unchecked: PriceSourceUnchecked,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    let asset_reference = asset.get_reference();

    if info.sender != config.owner {
        return Err(StdError::generic_err("Only owner can set asset"));
    }

    let price_source: PriceSourceChecked = match price_source_unchecked {
        PriceSourceUnchecked::Fixed { price } => PriceSourceChecked::Fixed { price },
        PriceSourceUnchecked::Native { denom } => PriceSourceChecked::Native { denom },
        PriceSourceUnchecked::AstroportSpot {
            pair_address,
            asset_address,
        } => PriceSourceChecked::AstroportSpot {
            pair_address: deps.api.addr_validate(&pair_address)?,
            asset_address: deps.api.addr_validate(&asset_address)?,
        },
        PriceSourceUnchecked::AstroportTwap {
            pair_address,
            asset_address,
            min_period,
        } => PriceSourceChecked::AstroportTwap {
            pair_address: deps.api.addr_validate(&pair_address)?,
            asset_address: deps.api.addr_validate(&asset_address)?,
            min_period,
        },
    };

    // For TWAP, we need to record the initial cumulative prices as part of the setup
    if let PriceSourceChecked::AstroportTwap {
        pair_address,
        asset_address,
        ..
    } = &price_source
    {
        let price_cumulative = query_cumulative_price(deps.querier, pair_address, asset_address)?;

        ASTROPORT_TWAP_DATA.save(
            deps.storage,
            asset_reference.as_slice(),
            &AstroportTwapData {
                timestamp: env.block.time.seconds(),
                price_average: Decimal::zero(), // average price will be zero until the 1st update
                price_cumulative,
            },
        )?;
    };

    PRICE_CONFIGS.save(
        deps.storage,
        asset_reference.as_slice(),
        &PriceConfig { price_source },
    )?;

    Ok(Response::default())
}

/// Modified from
/// https://github.com/Uniswap/uniswap-v2-periphery/blob/master/contracts/examples/ExampleOracleSimple.sol
pub fn execute_update_astroport_twap_data(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    assets: Vec<Asset>,
) -> StdResult<Response> {
    let mut events: Vec<Event> = vec![];

    for asset in assets {
        let asset_reference = asset.get_reference();
        let price_config = PRICE_CONFIGS.load(deps.storage, asset_reference.as_slice())?;
        let twap_data_last = ASTROPORT_TWAP_DATA.load(deps.storage, asset_reference.as_slice())?;

        // Asset must be configured to use TWAP price source
        let (pair_address, asset_address, min_period) = match &price_config.price_source {
            PriceSourceChecked::AstroportTwap {
                pair_address,
                asset_address,
                min_period,
            } => (pair_address, asset_address, min_period),
            _ => {
                return Err(StdError::generic_err("Price source is not TWAP!"));
            }
        };

        // Enough time must have elapsed since the last update
        let timestamp = env.block.time.seconds();
        let time_elapsed = timestamp - twap_data_last.timestamp;

        if time_elapsed < *min_period {
            return Err(StdError::generic_err("Minimum period not elapsed"));
        }

        // Query new price data
        let price_cumulative = query_cumulative_price(deps.querier, pair_address, asset_address)?;

        // If the cumulative price overflows on Astroport pair contract, then for this update we don't
        // change `price_average`. On the next update, `price_average` will resume updating as usual.
        let price_average = if price_cumulative >= twap_data_last.price_cumulative {
            Decimal::from_ratio(
                price_cumulative - twap_data_last.price_cumulative,
                time_elapsed,
            )
        } else {
            Decimal::from_ratio(
                price_cumulative.checked_add(Uint128::MAX - twap_data_last.price_cumulative)?,
                time_elapsed,
            )
        };

        let twap_data = AstroportTwapData {
            timestamp,
            price_average,
            price_cumulative,
        };

        ASTROPORT_TWAP_DATA.save(deps.storage, asset_reference.as_slice(), &twap_data)?;

        events.push(
            Event::new("update_astroport_twap_data")
                .add_attribute("asset", String::from_utf8(asset_reference).unwrap())
                .add_attribute("timestamp_last", twap_data_last.price_cumulative)
                .add_attribute("timestamp", twap_data.price_cumulative)
                .add_attribute("price_cumulative_last", twap_data_last.price_cumulative)
                .add_attribute("price_cumulative", twap_data.price_cumulative)
                .add_attribute("price_average", format!("{}", twap_data.price_average)),
        );
    }

    Ok(Response::new().add_events(events))
}

// QUERIES

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps, env)?),
        QueryMsg::AssetConfig { asset } => to_binary(&query_asset_config(deps, env, asset)?),
        QueryMsg::AssetPrice { asset } => {
            let asset_reference = asset.get_reference();
            to_binary(&query_asset_price(deps, env, asset_reference)?)
        }
        QueryMsg::AssetPriceByReference { asset_reference } => {
            to_binary(&query_asset_price(deps, env, asset_reference)?)
        }
    }
}

fn query_config(deps: Deps, _env: Env) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        owner: config.owner.into(),
    })
}

fn query_asset_config(deps: Deps, _env: Env, asset: Asset) -> StdResult<PriceConfig> {
    let asset_reference = asset.get_reference();
    let price_config = PRICE_CONFIGS.load(deps.storage, asset_reference.as_slice())?;

    Ok(price_config)
}

fn query_asset_price(
    deps: Deps,
    env: Env,
    asset_reference: Vec<u8>,
) -> StdResult<AssetPriceResponse> {
    let price_config = PRICE_CONFIGS.load(deps.storage, asset_reference.as_slice())?;

    match price_config.price_source {
        PriceSourceChecked::Fixed { price } => Ok(AssetPriceResponse {
            price,
            last_updated: env.block.time.seconds(),
        }),

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
                Some(exchange_rate_item) => Ok(AssetPriceResponse {
                    price: exchange_rate_item.exchange_rate.into(),
                    last_updated: env.block.time.seconds(),
                }),
                None => Err(StdError::generic_err("No native price found")),
            }
        }

        // NOTE:
        // 1) Spot price is defined as the amount of the other asset in the pair to be
        // returned when swapping x units of the asset, divided by x. In this implementation,
        // x = 1,000,000. For example, for MARS-UST pair, if swapping 1,000,000 uMARS returns
        // 1,200,000 uusd (return amount plus commission), then 1 MARS = 1.2 UST.
        // 2) Why not just take the quotient of the two assets reserves, for example if the
        // pool has 120 UST and 100 MARS, then 1 MARS = 1.2 UST? Because this only works for
        // XY-K pools, not StableSwap pools.
        // 3) The price is quoted in the other asset in the pair. For example, for MARS-UST
        // pair, the price of MARS is quoted in UST; for bLUNA-LUNA pair, the price of bLUNA
        // is quoted in LUNA.
        PriceSourceChecked::AstroportSpot {
            pair_address,
            asset_address,
        } => {
            let response: SimulationResponse =
                deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                    contract_addr: pair_address.to_string(),
                    msg: to_binary(&AstroportQueryMsg::Simulation {
                        offer_asset: AstroportAsset {
                            info: AstroportAssetInfo::Token {
                                contract_addr: asset_address,
                            },
                            amount: Uint128::new(1000000u128),
                        },
                    })?,
                }))?;

            let price = Decimal::from_ratio(
                response.return_amount + response.commission_amount,
                1000000u128,
            );

            Ok(AssetPriceResponse {
                price,
                last_updated: env.block.time.seconds(),
            })
        }

        PriceSourceChecked::AstroportTwap { .. } => {
            let twap_data = ASTROPORT_TWAP_DATA.load(deps.storage, asset_reference.as_slice())?;

            Ok(AssetPriceResponse {
                price: twap_data.price_average,
                last_updated: twap_data.timestamp,
            })
        }
    }
}

fn query_cumulative_price(
    querier: QuerierWrapper,
    pair_address: &Addr,
    asset_address: &Addr,
) -> StdResult<Uint128> {
    let response: CumulativePricesResponse =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: pair_address.to_string(),
            msg: to_binary(&AstroportQueryMsg::CumulativePrices {})?,
        }))?;

    // If the asset matches asset 0 in the pair, then we return `price0_cumulative_last`;
    // if it matches asset 1, we return `price1_cumulative_last`. If it matches neither,
    // we throw an error.
    let asset_index = response.assets.iter().position(|asset| match &asset.info {
        AstroportAssetInfo::Token { contract_addr } => contract_addr == asset_address,
        AstroportAssetInfo::NativeToken { .. } => false,
    });

    match asset_index {
        Some(index) if index == 0 => Ok(response.price0_cumulative_last),
        Some(_) => Ok(response.price1_cumulative_last),
        None => Err(StdError::generic_err("Asset mismatch")),
    }
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
            assert_eq!(err, StdError::generic_err("Only owner can update config"));
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
            assert_eq!(err, StdError::generic_err("Only owner can set asset"));
        }

        let info = mock_info("owner", &[]);

        // Fixed
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

        // Native
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

        // Astroport spot price
        {
            let asset = Asset::Cw20 {
                contract_addr: String::from("token"),
            };
            let reference = asset.get_reference();
            let msg = ExecuteMsg::SetAsset {
                asset: asset,
                price_source: PriceSourceUnchecked::AstroportSpot {
                    pair_address: "pair".to_string(),
                    asset_address: "asset".to_string(),
                },
            };
            execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
            let price_config = PRICE_CONFIGS
                .load(&deps.storage, reference.as_slice())
                .unwrap();
            assert_eq!(
                price_config.price_source,
                PriceSourceChecked::AstroportSpot {
                    pair_address: Addr::unchecked("pair"),
                    asset_address: Addr::unchecked("asset")
                }
            );
        }
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
        let response: AssetPriceResponse = from_binary(
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

        assert_eq!(response.price, Decimal::from_ratio(3_u128, 2_u128));
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
        let response: AssetPriceResponse = from_binary(
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

        assert_eq!(response.price, Decimal::from_ratio(4_u128, 1_u128));
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
