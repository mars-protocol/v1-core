use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal};
use cw_storage_plus::Item;

/// Stores config at the given key
pub const CONFIG: Item<Config> = Item::new("config");

/// Treasury global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    /// Terraswap factory contract address
    pub terraswap_factory_address: Addr,
    /// Terraswap max spread
    pub terraswap_max_spread: Decimal,
}
