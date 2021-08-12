use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::Addr;
use cw_storage_plus::{Item, Map};

use mars::oracle::PriceSourceChecked;

/// Stores config at the given key
pub const CONFIG: Item<Config> = Item::new("config");
pub const PRICE_CONFIGS: Map<&[u8], PriceConfig> = Map::new("price_configs");

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
