use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{CanonicalAddr, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static RESERVES_NAMESPACE: &[u8] = b"reserves";
pub static DEBTS_NAMESPACE: &[u8] = b"debts";
pub static USERS_NAMESPACE: &[u8] = b"users";
pub static RESERVE_DENOMS_NAMESPACE: &[u8] = b"reserve_denoms";

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

    /// Borrow index (Used to compute borrow interest)
    pub borrow_index: Decimal256,
    /// Liquidity index (Used to compute deposit interest)
    pub liquidity_index: Decimal256,
    /// Rate charged to borrowers
    pub borrow_rate: Decimal256,
    /// Rate paid to depositors
    pub liquidity_rate: Decimal256,

    /// Variable debt interest slope
    pub borrow_slope: Decimal256,
    /// Max percentage of collateral that can be borrowed 
    pub loan_to_value: Decimal256,

    /// timestamp (seconds) where indexes and rates where last updated
    pub interests_last_updated: u64,
    /// Total debt scaled for the reserve's currency
    pub debt_total_scaled: Uint256,
}

pub fn reserves_state<S: Storage>(storage: &mut S) -> Bucket<S, Reserve> {
    bucket(RESERVES_NAMESPACE, storage)
}

pub fn reserves_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Reserve> {
    bucket_read(RESERVES_NAMESPACE, storage)
}

/// Data for individual users
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct User {
    /// bitmap representing borrowed asset. 1 on the corresponding bit means asset is
    /// being borrowed
    pub borrowed_assets: Uint128,
    pub deposited_assets: Uint128,
}

pub fn users_state<S: Storage>(storage: &mut S) -> Bucket<S, User> {
    bucket(USERS_NAMESPACE, storage)
}

pub fn users_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, User> {
    bucket_read(USERS_NAMESPACE, storage)
}

/// Debt for each asset and user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Debt {
    /// Scaled debt amount
    // TODO(does this amount always have six decimals? How do we manage this?)
    pub amount_scaled: Uint256,
}

pub fn debts_asset_state<'a, S: Storage>(storage: &'a mut S, asset: &[u8]) -> Bucket<'a, S, Debt> {
    Bucket::multilevel(&[DEBTS_NAMESPACE, asset], storage)
}

pub fn debts_asset_state_read<'a, S: Storage>(
    storage: &'a S,
    asset: &[u8],
) -> ReadonlyBucket<'a, S, Debt> {
    ReadonlyBucket::multilevel(&[DEBTS_NAMESPACE, asset], storage)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ReserveDenoms {
    /// Denom of reserve
    pub denom: String,
}

pub fn reserve_denoms_state<S: Storage>(storage: &mut S) -> Bucket<S, ReserveDenoms> {
    bucket(RESERVE_DENOMS_NAMESPACE, storage)
}

pub fn reserve_denoms_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, ReserveDenoms> {
    bucket_read(RESERVE_DENOMS_NAMESPACE, storage)
}
