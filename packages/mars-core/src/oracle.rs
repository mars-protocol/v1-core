use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Uint128};

use crate::math::decimal::Decimal;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource<A> {
    /// Returns a fixed value; used for UST
    Fixed { price: Decimal },
    /// Native Terra stablecoins transaction rate quoted in UST
    Native { denom: String },
    /// Astroport spot price quoted in the other asset of the pair
    AstroportSpot { pair_address: A, asset_address: A },
    /// Astroport TWAP price quoted in the other asset of the pair
    AstroportTwap {
        pair_address: A,
        asset_address: A,
        /// Minimum time (in seconds) required to pass between two TWAP data updates.
        /// E.g. if set to 300, then prices will be averaged over periods of no less than 5 minutes.
        min_period: u64,
    },
}

pub type PriceSourceUnchecked = PriceSource<String>;
pub type PriceSourceChecked = PriceSource<Addr>;

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
pub struct AstroportTwapData {
    /// Timestamp of the most recent TWAP data update
    pub timestamp: u64,
    /// Cumulative price of the asset retrieved by the most recent TWAP data update
    pub price_cumulative: Uint128,
    /// Price of the asset averaged over the last two TWAP data updates
    pub price_average: Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AssetPriceResponse {
    /// Price of the asset averaged over the last two TWAP data updates
    pub price: Decimal,
    /// Timestamp of the most recent TWAP data update. Contracts querying the price data are recommended
    /// to check this value and determine if the data is too old to be considered valid.
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
        UpdateAstroportTwapData { assets: Vec<Asset> },
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
