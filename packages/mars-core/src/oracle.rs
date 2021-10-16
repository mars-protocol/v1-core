use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Api, StdResult, Uint128};

use crate::math::decimal::Decimal;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource<A> {
    /// Returns a fixed value; used for UST
    Fixed { price: Decimal },
    /// Native Terra stablecoins transaction rate quoted in UST
    Native { denom: String },
    /// Astroport spot price quoted in UST
    ///
    /// NOTE: `pair_address` must point to an astroport pair consists of the asset of intereset and UST
    AstroportSpot {
        /// Address of the Astroport pair
        pair_address: A,
    },
    /// Astroport TWAP price quoted in UST
    ///
    /// NOTE: `pair_address` must point to an astroport pair consists of the asset of intereset and UST
    AstroportTwap {
        /// Address of the Astroport pair
        pair_address: A,
        /// Address of the asset of interest
        ///
        /// NOTE: Spot price in intended for CW20 tokens. Terra native tokens should use Fixed or
        /// Native price sources.
        window_size: u64,
        /// When calculating averaged price, we take the most recent TWAP snapshot and find a second
        /// snapshot in the range of window_size +/- tolerance. For example, if window size is 5 minutes
        /// and tolerance is 1 minute, we look for snapshots that are 4 - 6 minutes back in time from
        /// the most recent snapshot.
        ///
        /// If there are multiple snapshots within the range, we take the one that is closest to the
        /// desired window size.
        tolerance: u64,
    },
}

impl<A> PriceSource<A> {
    /// Used for logging
    pub fn label(&self) -> &str {
        match self {
            PriceSource::Fixed { .. } => "fixed",
            PriceSource::Native { .. } => "native",
            PriceSource::AstroportSpot { .. } => "astroport_spot",
            PriceSource::AstroportTwap { .. } => "astroport_twap",
        }
    }
}

pub type PriceSourceUnchecked = PriceSource<String>;
pub type PriceSourceChecked = PriceSource<Addr>;

impl PriceSourceUnchecked {
    pub fn to_checked(&self, api: &dyn Api) -> StdResult<PriceSourceChecked> {
        Ok(match self {
            PriceSourceUnchecked::Fixed { price } => PriceSourceChecked::Fixed { price: *price },
            PriceSourceUnchecked::Native { denom } => PriceSourceChecked::Native {
                denom: denom.clone(),
            },
            PriceSourceUnchecked::AstroportSpot { pair_address } => {
                PriceSourceChecked::AstroportSpot {
                    pair_address: api.addr_validate(pair_address)?,
                }
            }
            PriceSourceUnchecked::AstroportTwap {
                pair_address,
                window_size,
                tolerance,
            } => PriceSourceChecked::AstroportTwap {
                pair_address: api.addr_validate(pair_address)?,
                window_size: *window_size,
                tolerance: *tolerance,
            },
        })
    }
}

/// Contract global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
}

/// Price source configuration for a given asset
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PriceConfig {
    pub price_source: PriceSourceChecked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AstroportTwapSnapshot {
    /// Timestamp of the most recent TWAP data update
    pub timestamp: u64,
    /// Cumulative price of the asset retrieved by the most recent TWAP data update
    pub price_cumulative: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AssetPriceResponse {
    /// Price of the asset averaged over the last two TWAP data updates
    pub price: Decimal,
    /// The last time the price data was updated. Contracts querying the price data are recommended
    /// to check this value and determine if the data is too old to be considered valid.
    ///
    /// For Fixed, Native, and AstroportSpot price sources, this is simply the current block timestamp
    /// as prices are updated instantaneously with these sources.
    ///
    /// For AstroportTwap price source, this is the timestamp of the most recent TWAP snapshot.
    /// E.g. If two TWAP price snapshots were taken at block A and B, where B is more recent than A,
    /// and the price is averaged over A to B, then `last_updated` is the timestamp of block B.
    pub last_updated: u64,
}

pub mod msg {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::PriceSourceUnchecked;
    use crate::asset::Asset;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InstantiateMsg {
        pub owner: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Update contract config
        UpdateConfig { owner: Option<String> },
        /// Specify parameters to query asset price
        SetAsset {
            asset: Asset,
            price_source: PriceSourceUnchecked,
        },
        /// Fetch cumulative price from Astroport pair and record in contract storage
        RecordTwapSnapshot { assets: Vec<Asset> },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Query contract config
        Config {},
        /// Get asset's price config
        AssetConfig { asset: Asset },
        /// Query asset price given an asset
        AssetPrice { asset: Asset },
        /// Query asset price given it's internal reference
        /// (meant to be used by protocol contracts only)
        AssetPriceByReference { asset_reference: Vec<u8> },
    }
}

pub mod helpers {
    use cosmwasm_std::{to_binary, Addr, QuerierWrapper, QueryRequest, StdResult, WasmQuery};

    use crate::asset::AssetType;
    use crate::math::decimal::Decimal;

    use super::msg::QueryMsg;
    use super::AssetPriceResponse;

    pub fn query_price(
        querier: QuerierWrapper,
        oracle_address: Addr,
        asset_label: &str,
        asset_reference: Vec<u8>,
        asset_type: AssetType,
    ) -> StdResult<Decimal> {
        let query: Decimal = if asset_type == AssetType::Native && asset_label == "uusd" {
            Decimal::one()
        } else {
            let asset_price: AssetPriceResponse =
                querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                    contract_addr: oracle_address.into(),
                    msg: to_binary(&QueryMsg::AssetPriceByReference { asset_reference })?,
                }))?;
            asset_price.price
        };

        Ok(query)
    }
}
