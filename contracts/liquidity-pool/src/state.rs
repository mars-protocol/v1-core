use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::{CanonicalAddr, Storage};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static RESERVES_NAMESPACE: &[u8] = b"reserves";
pub static DEBTS_NAMESPACE: &[u8] = b"debt";

/// Lending pool global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: CanonicalAddr,
    /// maToken code id used to instantiate new tokens
    pub ma_token_code_id: u64,
    /// Reserve count
    pub reserve_count: u32,
}

pub fn config_state<S: Storage>(storage: &mut S) -> Singleton<S, Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_state_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, Config> {
    singleton_read(storage, CONFIG_KEY)
}

/// Asset reserves
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Reserve {
    /// Reserve index (Bit position on data)
    pub index: u32,
    /// maToken contract address
    pub ma_token_address: CanonicalAddr,
    /// Liquidity index (Used to compute deposit interest)
    pub liquidity_index: Decimal256,
    pub borrow_index: Decimal256,
}

pub fn reserves_state<S: Storage>(storage: &mut S) -> Bucket<S, Reserve> {
    bucket(RESERVES_NAMESPACE, storage)
}

pub fn reserves_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Reserve> {
    bucket_read(RESERVES_NAMESPACE, storage)
}

/// Data for individual borrowers
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Borrower {
    /// list of borrowed assets ids
    // TODO: Could just be a uint256 that has dedicated bits for each asset.
    pub borrowed_assets: Vec<String>,
}

pub fn borrowers_state<S: Storage>(storage: &mut S) -> Bucket<S, Reserve> {
    bucket(RESERVES_NAMESPACE, storage)
}

pub fn borrowers_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Reserve> {
    bucket_read(RESERVES_NAMESPACE, storage)
}

/// Debt for each asset and user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Debt {
    /// Id list of borrowed assets
    // TODO: Could just be a uint256 that has dedicated bits for each asset.
    pub borrowed_assets: Vec<String>,
}

pub fn debt_asset_state<'a, S: Storage>(
    storage: &'a mut S,
    asset: &[u8],
) -> Bucket<'a, S, Reserve> {
    Bucket::multilevel(&[DEBTS_NAMESPACE, asset], storage)
}

pub fn debt_asset_state_read<'a, S: Storage>(
    storage: &'a S,
    asset: &[u8],
) -> ReadonlyBucket<'a, S, Reserve> {
    ReadonlyBucket::multilevel(&[DEBTS_NAMESPACE, asset], storage)
}
