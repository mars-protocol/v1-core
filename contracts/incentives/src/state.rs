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
pub static ASSET_INCENTIVES_NAMESPACE: &[u8] = b"asset_data";
pub static USER_ASSET_INDICES_NAMESPACE: &[u8] = b"user_asset_indices";
pub static USER_UNCLAIMED_REWARDS_NAMESPACE: &[u8] = b"user_unclaimed_rewards";

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

pub fn config(storage: &mut dyn Storage) -> Singleton<Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read(storage: &dyn Storage) -> ReadonlySingleton<Config> {
    singleton_read(storage, CONFIG_KEY)
}

pub fn asset_incentives(storage: &mut dyn Storage) -> Bucket<AssetIncentive> {
    bucket(storage, ASSET_INCENTIVES_NAMESPACE)
}

pub fn asset_incentives_read(storage: &dyn Storage) -> ReadonlyBucket<AssetIncentive> {
    bucket_read(storage, ASSET_INCENTIVES_NAMESPACE)
}

pub fn user_asset_indices<'a>(
    storage: &'a mut dyn Storage,
    user_reference: &[u8],
) -> Bucket<'a, Decimal> {
    Bucket::multilevel(storage, &[USER_ASSET_INDICES_NAMESPACE, user_reference])
}

pub fn user_asset_indices_read<'a>(
    storage: &'a dyn Storage,
    user_reference: &[u8],
) -> ReadonlyBucket<'a, Decimal> {
    ReadonlyBucket::multilevel(storage, &[USER_ASSET_INDICES_NAMESPACE, user_reference])
}

pub fn user_unclaimed_rewards(storage: &mut dyn Storage) -> Bucket<Uint128> {
    bucket(storage, USER_UNCLAIMED_REWARDS_NAMESPACE)
}

pub fn user_unclaimed_rewards_read(storage: &dyn Storage) -> ReadonlyBucket<Uint128> {
    bucket_read(storage, USER_UNCLAIMED_REWARDS_NAMESPACE)
}
