use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

use mars::oracle::PriceSourceChecked;

/// Stores config at the given key
pub const CONFIG: Item<Config> = Item::new("config");
pub const PRICE_CONFIGS: Map<&[u8], PriceConfig> = Map::new("price_configs");
pub const ASTROPORT_TWAP_DATA: Map<&[u8], AstroportTwapData> = Map::new("twap_data");

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
