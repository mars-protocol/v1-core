use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, Storage};
use cosmwasm_storage::{singleton, singleton_read, ReadonlySingleton, Singleton};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static COOLDOWNS_NAMESPACE: &[u8] = b"cooldowns";

/// Treasury global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: CanonicalAddr,
    /// Council contract address
    pub council_address: CanonicalAddr,
    /// Incentives contract address
    pub incentives_address: CanonicalAddr,
    /// Insurance fund contract address
    pub insurance_fund_address: CanonicalAddr,
    /// Mars token address
    pub mars_token_address: CanonicalAddr,
    /// Red bank contract address
    pub red_bank_address: CanonicalAddr,
    /// Staking contract address
    pub staking_address: CanonicalAddr,
    /// Treasury contract address
    pub treasury_address: CanonicalAddr,
    /// xMars token address
    pub xmars_token_address: CanonicalAddr,
}

pub fn config<S: Storage>(storage: &mut S) -> Singleton<S, Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, Config> {
    singleton_read(storage, CONFIG_KEY)
}
