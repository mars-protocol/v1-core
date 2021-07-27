use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_storage_plus::{Item, Map};

// keys (for singleton)
pub const CONFIG: Item<Config> = Item::new("config");

// namespaces (for buckets)
pub const ASSET_INCENTIVES: Map<&Addr, AssetIncentive> = Map::new("asset_data");
pub const USER_ASSET_INDICES: Map<(&Addr, &Addr), Decimal> = Map::new("user_asset_indices");
pub const USER_UNCLAIMED_REWARDS: Map<&Addr, Uint128> = Map::new("user_unclaimed_rewards");

/// Global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider returns addresses for all protocol contracts
    pub address_provider_address: Addr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AssetIncentive {
    pub emission_per_second: Uint128,
    pub index: Decimal,
    pub last_updated: u64,
}
