use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

// keys (for singleton)
pub const CONFIG: Item<Config> = Item::new("config");

// namespaces (for buckets)
pub const COOLDOWNS: Map<&Addr, Cooldown> = Map::new("cooldowns");

/// Treasury global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider address
    pub address_provider_address: Addr,
    /// Astroport factory contract address
    pub astroport_factory_address: Addr,
    /// Astroport max spread
    pub astroport_max_spread: Decimal,
    /// Cooldown duration in seconds
    pub cooldown_duration: u64,
    /// Time in seconds after the cooldown ends during which the unstaking of
    /// the associated amount is allowed
    pub unstake_window: u64,
}

/// Unstaking cooldown data
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Cooldown {
    /// Timestamp where the cooldown was activated
    pub timestamp: u64,
    /// Amount that the user is allowed to unstake during the unstake window
    pub amount: Uint128,
}
