use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal};
use cw_storage_plus::Item;

// Key
pub const CONFIG: Item<Config> = Item::new("config");

/// Safety fund global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    /// Astroport factory contract address
    pub astroport_factory_address: Addr,
    /// Astroport max spread
    pub astroport_max_spread: Decimal,
}
