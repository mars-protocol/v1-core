use cosmwasm_std::{
    attr, entry_point, to_binary, Attribute, Binary, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, Uint128,
};
use terra_cosmwasm::TerraQuerier;

use mars_core::asset::Asset;
use mars_core::helpers::option_string_to_addr;
use mars_core::math::decimal::Decimal;

use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{ASTROPORT_TWAP_SNAPSHOTS, CONFIG, PRICE_CONFIGS};
use crate::{
    AssetPriceResponse, AstroportTwapSnapshot, Config, PriceConfig, PriceSourceChecked,
    PriceSourceUnchecked,
};

use self::helpers::*;

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
        ExecuteMsg::RecordTwapSnapshot { assets } => {
            execute_record_twap_snapshot(deps, env, info, assets)
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

    Ok(Response::new().add_attribute("action", "update_config"))
}

pub fn execute_set_asset(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset: Asset,
    price_source_unchecked: PriceSourceUnchecked,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(StdError::generic_err("Only owner can set asset"));
    }

    let (asset_label, asset_reference, _) = asset.get_attributes();
    let price_source = price_source_unchecked.to_checked(deps.api)?;

    // for spot and TWAP sources, we must make sure: the astroport pair indicated by `pair_address`
    // must consists of UST and the asset of interest
    match &price_source {
        PriceSourceChecked::AstroportSpot { pair_address }
        | PriceSourceChecked::AstroportTwap { pair_address, .. } => {
            assert_astroport_pool_assets(&deps.querier, &asset, pair_address)?;
        }
        _ => (),
    }

    PRICE_CONFIGS.save(
        deps.storage,
        &asset_reference,
        &PriceConfig { price_source },
    )?;

    Ok(Response::new()
        .add_attribute("action", "set_asset")
        .add_attribute("asset", asset_label)
        .add_attribute("price_source", price_source_unchecked.label()))
}

/// Modified from
/// https://github.com/Uniswap/uniswap-v2-periphery/blob/master/contracts/examples/ExampleOracleSimple.sol
pub fn execute_record_twap_snapshot(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    assets: Vec<Asset>,
) -> StdResult<Response> {
    let mut attrs: Vec<Attribute> = vec![];

    for asset in assets {
        let (asset_label, asset_reference, _) = asset.get_attributes();
        let price_config = PRICE_CONFIGS.load(deps.storage, &asset_reference)?;

        // Asset must be configured to use TWAP price source
        //
        // This block of code is really ugly because the same variable names are typed three times.
        // Is there a cleaner syntax?
        let (pair_address, window_size, tolerance) = match price_config.price_source {
            PriceSourceChecked::AstroportTwap {
                pair_address,
                window_size,
                tolerance,
            } => (pair_address, window_size, tolerance),
            _ => {
                return Err(StdError::generic_err("price source is not TWAP"));
            }
        };

        // Load existing snapshots. If there's none, we initialize an empty vector
        let mut snapshots = ASTROPORT_TWAP_SNAPSHOTS
            .load(deps.storage, &asset_reference)
            .unwrap_or_else(|_| vec![]);

        // Query new price data
        let timestamp = env.block.time.seconds();
        let price_cumulative = query_astroport_cumulative_price(&deps.querier, &pair_address)?;

        // Purge snapshots that are too old, i.e. more than [window_size + tolerance] away from the
        // most recent update. These snapshots will never be used in the future for calculating average
        // prices
        snapshots.retain(|snapshot| timestamp - snapshot.timestamp <= window_size + tolerance);

        snapshots.push(AstroportTwapSnapshot {
            timestamp,
            price_cumulative,
        });

        ASTROPORT_TWAP_SNAPSHOTS.save(deps.storage, &asset_reference, &snapshots)?;

        attrs.extend(vec![
            attr("asset", asset_label),
            attr("timestamp", timestamp.to_string()),
            attr("price_cumulative", price_cumulative),
        ]);
    }

    Ok(Response::new()
        .add_attribute("action", "record_twap_snapshot")
        .add_attributes(attrs))
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

fn query_config(deps: Deps, _env: Env) -> StdResult<Config> {
    CONFIG.load(deps.storage)
}

fn query_asset_config(deps: Deps, _env: Env, asset: Asset) -> StdResult<PriceConfig> {
    let asset_reference = asset.get_reference();
    PRICE_CONFIGS.load(deps.storage, &asset_reference)
}

fn query_asset_price(
    deps: Deps,
    env: Env,
    asset_reference: Vec<u8>,
) -> StdResult<AssetPriceResponse> {
    let price_config = PRICE_CONFIGS.load(deps.storage, &asset_reference)?;

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
        //
        // 1) Spot price is defined as the amount of the other asset in the pair to be
        // returned when swapping x units of the asset, divided by x. In this implementation,
        // x = 1,000,000. For example, for MARS-UST pair, if swapping 1,000,000 uMARS returns
        // 1,200,000 uusd (return amount plus commission), then 1 MARS = 1.2 UST.
        //
        // 2) Why not just take the quotient of the two assets reserves, for example if the
        // pool has 120 UST and 100 MARS, then 1 MARS = 1.2 UST? Because this only works for
        // XY-K pools, not StableSwap pools.
        //
        // 3) The price is quoted in the other asset in the pair. For example, for MARS-UST
        // pair, the price of MARS is quoted in UST; for bLUNA-LUNA pair, the price of bLUNA
        // is quoted in LUNA.
        PriceSourceChecked::AstroportSpot { pair_address } => {
            let price = query_astroport_spot_price(&deps.querier, &pair_address)?;
            Ok(AssetPriceResponse {
                price: price.into(), // cast cosmwasm_std::Decimal to mars_core::math::decimal::Decimal
                last_updated: env.block.time.seconds(),
            })
        }

        PriceSourceChecked::AstroportTwap {
            window_size,
            tolerance,
            ..
        } => {
            let mut snapshots = ASTROPORT_TWAP_SNAPSHOTS.load(deps.storage, &asset_reference)?;

            // First we grab the most recent snapshot and remove it from the vector
            let latest_snapshot = snapshots.last().unwrap().clone();
            snapshots.pop();

            // Among the test, we find the ones that are within [window_size +/- tolerance] from the
            // most recent snapshot
            snapshots.retain(|snapshot| {
                diff(latest_snapshot.timestamp - snapshot.timestamp, window_size) <= tolerance
            });

            if snapshots.is_empty() {
                return Err(StdError::generic_err("no snapshot within tolerable window"));
            }

            // We then sort all snapshot by the difference between their period and the desired window
            // and take the once whose difference is the smallest
            snapshots.sort_by(|a, b| {
                let diff_a = diff(latest_snapshot.timestamp - a.timestamp, window_size);
                let diff_b = diff(latest_snapshot.timestamp - b.timestamp, window_size);
                diff_a.cmp(&diff_b)
            });
            let earlier_snapshot = &snapshots[0];

            // Handle the case if Astroport's cumulative price overflows. In this case, cumulative price
            // of the latest snapshot warps back to zero
            let period = latest_snapshot.timestamp - earlier_snapshot.timestamp;
            let price = if latest_snapshot.price_cumulative >= earlier_snapshot.price_cumulative {
                Decimal::from_ratio(
                    latest_snapshot.price_cumulative - earlier_snapshot.price_cumulative,
                    period,
                )
            } else {
                Decimal::from_ratio(
                    latest_snapshot
                        .price_cumulative
                        .checked_add(Uint128::MAX - earlier_snapshot.price_cumulative)?,
                    period,
                )
            };

            Ok(AssetPriceResponse {
                price,
                last_updated: latest_snapshot.timestamp,
            })
        }
    }
}

// HELPERS

mod helpers {
    use cosmwasm_std::{
        to_binary, Addr, Decimal, QuerierWrapper, QueryRequest, StdError, StdResult, Uint128,
        WasmQuery,
    };
    use mars_core::asset::Asset;

    // Once astroport package is published on crates.io, update Cargo.toml and change these to
    // use astroport::asset::{...};
    // and
    // use astroport::pair::{...};
    use crate::astroport::asset::{Asset as AstroportAsset, AssetInfo as AstroportAssetInfo};
    use crate::astroport::pair::{
        CumulativePricesResponse, PoolResponse, QueryMsg as AstroportQueryMsg, SimulationResponse,
    };

    // See comments for `query_astroport_spot_price`
    const PROBE_AMOUNT: Uint128 = Uint128::new(1_000_000);

    pub fn diff(a: u64, b: u64) -> u64 {
        if a > b {
            a - b
        } else {
            b - a
        }
    }

    pub fn ust() -> AstroportAssetInfo {
        AstroportAssetInfo::NativeToken {
            denom: "uusd".to_string(),
        }
    }

    // Cast astroport::asset::AssetInfo into mars_core::asset::Asset so that they can be compared
    impl From<&AstroportAssetInfo> for Asset {
        fn from(info: &AstroportAssetInfo) -> Self {
            match info {
                AstroportAssetInfo::Token { contract_addr } => Asset::Cw20 {
                    contract_addr: contract_addr.to_string(),
                },
                AstroportAssetInfo::NativeToken { denom } => Asset::Native {
                    denom: denom.clone(),
                },
            }
        }
    }

    /// Assert the astroport pair indicated by `pair_address` consists of UST and `asset`
    pub fn assert_astroport_pool_assets(
        querier: &QuerierWrapper,
        asset: &Asset,
        pair_address: &Addr,
    ) -> StdResult<()> {
        let response: PoolResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: pair_address.to_string(),
            msg: to_binary(&AstroportQueryMsg::Pool {})?,
        }))?;

        let asset0: Asset = (&response.assets[0].info).into();
        let asset1: Asset = (&response.assets[1].info).into();
        let ust: Asset = (&ust()).into();

        if (asset0 == ust && &asset1 == asset) || (asset1 == ust && &asset0 == asset) {
            Ok(())
        } else {
            Err(StdError::generic_err("invalid pair"))
        }
    }

    /// When calculating Spot price, we simulate a swap by offering PROBE_AMOUNT of the asset of interest,
    /// the find the return amount
    ///
    /// The Spot price is defined as: (return_amount + commission) / PROBE_AMOUNT
    pub fn query_astroport_spot_price(
        querier: &QuerierWrapper,
        pair_address: &Addr,
    ) -> StdResult<Decimal> {
        let response: PoolResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: pair_address.to_string(),
            msg: to_binary(&AstroportQueryMsg::Pool {})?,
        }))?;

        // to calculate spot price, we offer one of the asset that is not UST, and the offer amount
        // is PROBE_AMOUNT
        //
        // NOTE: here we assume one of the assets in the astroport pair must be UST. a check for this
        // must be perform when configuring asset price sources
        let offer_asset_info = if response.assets[0].info == ust() {
            response.assets[1].info.clone()
        } else {
            response.assets[0].info.clone()
        };
        let offer_asset = AstroportAsset {
            info: offer_asset_info,
            amount: PROBE_AMOUNT,
        };

        let response: SimulationResponse =
            querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: pair_address.to_string(),
                msg: to_binary(&AstroportQueryMsg::Simulation { offer_asset })?,
            }))?;

        Ok(Decimal::from_ratio(
            response.return_amount + response.commission_amount,
            PROBE_AMOUNT,
        ))
    }

    pub fn query_astroport_cumulative_price(
        querier: &QuerierWrapper,
        pair_address: &Addr,
    ) -> StdResult<Uint128> {
        let response: CumulativePricesResponse =
            querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: pair_address.to_string(),
                msg: to_binary(&AstroportQueryMsg::CumulativePrices {})?,
            }))?;

        // if asset0 is UST, we return the cumulative price of asset1; otherwise, return the cumulative
        // price of asset0
        //
        // NOTE: here we assume one of the assets in the astroport pair must be UST. a check for this
        // must be perform when configuring asset price sources
        let price_cumulative = if response.assets[0].info == ust() {
            response.price1_cumulative_last
        } else {
            response.price0_cumulative_last
        };
        Ok(price_cumulative)
    }
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_info, MockApi, MockStorage};
    use cosmwasm_std::{from_binary, Addr, OwnedDeps};
    use mars_core::testing::{mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams};

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
