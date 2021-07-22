use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static COOLDOWNS_NAMESPACE: &[u8] = b"cooldowns";

/// Treasury global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider address
    pub address_provider_address: Addr,
    /// Terraswap factory contract address
    pub terraswap_factory_address: Addr,
    /// Terraswap max spread
    pub terraswap_max_spread: Decimal,
    /// Cooldown duration in seconds
    pub cooldown_duration: u64,
    /// Time in seconds after the cooldown ends during which the unstaking of
    /// the associated amount is allowed
    pub unstake_window: u64,
}

pub fn config(storage: &mut dyn Storage) -> Singleton<Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read(storage: &dyn Storage) -> ReadonlySingleton<Config> {
    singleton_read(storage, CONFIG_KEY)
}

/// Unstaking cooldown data
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Cooldown {
    /// Timestamp where the cooldown was activated
    pub timestamp: u64,
    /// Amount that the user is allowed to unstake during the unstake window
    pub amount: Uint128,
}

pub fn cooldowns(storage: &mut dyn Storage) -> Bucket<Cooldown> {
    bucket(storage, COOLDOWNS_NAMESPACE)
}

pub fn cooldowns_read(storage: &dyn Storage) -> ReadonlyBucket<Cooldown> {
    bucket_read(storage, COOLDOWNS_NAMESPACE)
}
