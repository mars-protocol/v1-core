use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, ReadonlyStorage, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket,
    ReadonlyPrefixedStorage, ReadonlySingleton, Singleton,
};
use cw20::AllowanceResponse;

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub struct TokenInfo {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub total_supply: Uint128,
    pub mint: Option<MinterData>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct MinterData {
    pub minter: CanonicalAddr,
    /// cap is how many more tokens can be issued by the minter
    pub cap: Option<Uint128>,
}

impl TokenInfo {
    pub fn get_cap(&self) -> Option<Uint128> {
        self.mint.as_ref().and_then(|v| v.cap)
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
/// Snapshot for a given amount, could be applied to the total supply or to the balance of
/// a specific address
pub struct Snapshot {
    pub block: u64,
    pub value: Uint128,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
/// Metadata snapshots for a given address
pub struct SnapshotInfo {
    /// Index where snapshot search should start (Could be different than 0 if, in the
    /// future, sample should get smaller than all available to guarantee less operations
    /// when searching for a snapshot
    pub start_index: u64,
    /// Last index for snapshot search
    pub end_index: u64,
    /// Last block a snapshot was taken
    pub end_block: u64,
}

const TOKEN_INFO_KEY: &[u8] = b"token_info";
const TOTAL_SUPPLY_SNAPSHOT_INFO_KEY: &[u8] = b"total_supply_snapshot_info";
const PREFIX_BALANCE: &[u8] = b"balance";
const PREFIX_ALLOWANCE: &[u8] = b"allowance";
const PREFIX_TOTAL_SUPPLY_SNAPSHOT: &[u8] = b"total_supply_snapshot";
const PREFIX_BALANCE_SNAPSHOT_INFO: &[u8] = b"balance_snapshot_info";
const PREFIX_BALANCE_SNAPSHOT: &[u8] = b"balance_snapshot";

// meta is the token definition as well as the total_supply
pub fn token_info<S: Storage>(storage: &mut S) -> Singleton<S, TokenInfo> {
    singleton(storage, TOKEN_INFO_KEY)
}

pub fn token_info_read<S: ReadonlyStorage>(storage: &S) -> ReadonlySingleton<S, TokenInfo> {
    singleton_read(storage, TOKEN_INFO_KEY)
}

/// balances are state of the erc20 tokens
pub fn balances<S: Storage>(storage: &mut S) -> Bucket<S, Uint128> {
    bucket(PREFIX_BALANCE, storage)
}

/// balances are state of the erc20 tokens (read-only version for queries)
pub fn balances_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyBucket<S, Uint128> {
    bucket_read(PREFIX_BALANCE, storage)
}

pub fn balances_prefix_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyPrefixedStorage<S> {
    ReadonlyPrefixedStorage::new(PREFIX_BALANCE, storage)
}

/// returns a bucket with all allowances authorized by this owner (query it by spender)
pub fn allowances<'a, S: Storage>(
    storage: &'a mut S,
    owner: &CanonicalAddr,
) -> Bucket<'a, S, AllowanceResponse> {
    Bucket::multilevel(&[PREFIX_ALLOWANCE, owner.as_slice()], storage)
}

/// returns a bucket with all allowances authorized by this owner (query it by spender)
/// (read-only version for queries)
pub fn allowances_read<'a, S: ReadonlyStorage>(
    storage: &'a S,
    owner: &CanonicalAddr,
) -> ReadonlyBucket<'a, S, AllowanceResponse> {
    ReadonlyBucket::multilevel(&[PREFIX_ALLOWANCE, owner.as_slice()], storage)
}

/// Metadata for total supply snapshot
pub fn total_supply_snapshot_info<S: Storage>(storage: &mut S) -> Singleton<S, SnapshotInfo> {
    singleton(storage, TOTAL_SUPPLY_SNAPSHOT_INFO_KEY)
}

pub fn total_supply_snapshot_info_read<S: ReadonlyStorage>(
    storage: &S,
) -> ReadonlySingleton<S, SnapshotInfo> {
    singleton_read(storage, TOTAL_SUPPLY_SNAPSHOT_INFO_KEY)
}

/// Snapshots for total supply
pub fn total_supply_snapshot<S: Storage>(storage: &mut S) -> Bucket<S, Snapshot> {
    bucket(PREFIX_TOTAL_SUPPLY_SNAPSHOT, storage)
}

pub fn total_supply_snapshot_read<S: ReadonlyStorage>(storage: &S) -> ReadonlyBucket<S, Snapshot> {
    bucket_read(PREFIX_TOTAL_SUPPLY_SNAPSHOT, storage)
}

/// Metadata for balance snapshots
pub fn balance_snapshot_info<S: Storage>(storage: &mut S) -> Bucket<S, SnapshotInfo> {
    bucket(PREFIX_BALANCE_SNAPSHOT_INFO, storage)
}

pub fn balance_snapshot_info_read<S: ReadonlyStorage>(
    storage: &S,
) -> ReadonlyBucket<S, SnapshotInfo> {
    bucket_read(PREFIX_BALANCE_SNAPSHOT_INFO, storage)
}

/// balance Shapshots for a given address
pub fn balance_snapshot<'a, S: Storage>(
    storage: &'a mut S,
    address_raw: &CanonicalAddr,
) -> Bucket<'a, S, Snapshot> {
    Bucket::multilevel(&[PREFIX_BALANCE_SNAPSHOT, address_raw.as_slice()], storage)
}

pub fn balance_snapshot_read<'a, S: Storage>(
    storage: &'a S,
    address_raw: &CanonicalAddr,
) -> ReadonlyBucket<'a, S, Snapshot> {
    ReadonlyBucket::multilevel(&[PREFIX_BALANCE_SNAPSHOT, address_raw.as_slice()], storage)
}
