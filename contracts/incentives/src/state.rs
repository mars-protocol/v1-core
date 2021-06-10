use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, Decimal, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static ASSET_DATA_NAMESPACE: &[u8] = b"asset_data";
pub static ASSET_USER_INDEXES_NAMESPACE: &[u8] = b"asset_user_indexes";

/// Insurance fund global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: CanonicalAddr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AssetIncentive {
    pub emission_per_second: Uint128,
    pub index: Decimal,
    pub last_updated_timestamp: u64,
}

pub fn config<S: Storage>(storage: &mut S) -> Singleton<S, Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, Config> {
    singleton_read(storage, CONFIG_KEY)
}

pub fn asset_incentives<S: Storage>(storage: &mut S) -> Bucket<S, AssetIncentive> {
    bucket(ASSET_DATA_NAMESPACE, storage)
}

pub fn asset_incentives_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, AssetIncentive> {
    bucket_read(ASSET_DATA_NAMESPACE, storage)
}
