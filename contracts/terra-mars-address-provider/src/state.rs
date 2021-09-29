use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::Addr;
use cw_storage_plus::Item;

// Key
pub const CONFIG: Item<Config> = Item::new("config");

/// Global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Council contract address
    pub council_address: Addr,
    /// Incentives contract address
    pub incentives_address: Addr,
    /// Safety fund contract address
    pub safety_fund_address: Addr,
    /// Mars token address
    pub mars_token_address: Addr,
    /// Oracle address
    pub oracle_address: Addr,
    /// Protocol admin address (admin for all the contracts)
    pub protocol_admin_address: Addr,
    /// Protocol Rewards Collector address
    pub protocol_rewards_collector_address: Addr,
    /// Red bank contract address
    pub red_bank_address: Addr,
    /// Staking contract address
    pub staking_address: Addr,
    /// Treasury contract address
    pub treasury_address: Addr,
    /// Vesting contract address
    pub vesting_address: Addr,
    /// xMars token address
    pub xmars_token_address: Addr,
}
