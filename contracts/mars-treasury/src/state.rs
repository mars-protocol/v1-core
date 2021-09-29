use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal as StdDecimal};
use cw_storage_plus::Item;

/// Stores config at the given key
pub const CONFIG: Item<Config> = Item::new("config");

/// Treasury global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    /// Astroport factory contract address
    pub astroport_factory_address: Addr,
    /// Astroport max spread
    pub astroport_max_spread: StdDecimal,
}
