use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{CanonicalAddr, StdResult, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};
use mars::liquidity_pool::msg::{AssetType, InitOrUpdateAssetParams};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";

// namespaces (for buckets)
pub static RESERVES_NAMESPACE: &[u8] = b"reserves";
pub static DEBTS_NAMESPACE: &[u8] = b"debts";
pub static USERS_NAMESPACE: &[u8] = b"users";
pub static RESERVE_REFERENCES_NAMESPACE: &[u8] = b"reserve_references";
pub static RESERVE_MA_TOKENS_NAMESPACE: &[u8] = b"reserve_ma_tokens";
pub static UNCOLLATERALIZED_LOAN_LIMITS_NAMESPACE: &[u8] = b"uncollateralized_loan_limits";

/// Lending pool global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: CanonicalAddr,
    /// Protocol treasury contract address
    pub treasury_contract_address: CanonicalAddr,
    /// Protocol insurance fund contract address
    pub insurance_fund_contract_address: CanonicalAddr,
    /// Protocol staking contract address
    pub staking_contract_address: CanonicalAddr,
    /// maToken code id used to instantiate new tokens
    pub ma_token_code_id: u64,
    /// Reserve count
    pub reserve_count: u32,
    // Maximum percentage of outstanding debt that can be covered by a liquidator
    pub close_factor: Decimal256,
    // Percentage of fees that are sent to the insurance fund
    pub insurance_fund_fee_share: Decimal256,
    // Percentage of fees that are sent to the treasury
    pub treasury_fee_share: Decimal256,
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

    /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
    pub reserve_factor: Decimal256,

    /// Timestamp (seconds) where indexes and rates where last updated
    pub interests_last_updated: u64,
    /// Total debt scaled for the reserve's currency
    pub debt_total_scaled: Uint256,

    /// Indicated whether the asset is native or a cw20 token
    pub asset_type: AssetType,

    // Percentage at which the loan is defined as under-collateralized
    pub liquidation_threshold: Decimal256,
    // Bonus on the price of assets of the collateral when liquidators purchase it
    pub liquidation_bonus: Decimal256,

    // Protocol income to be distributed to other contracts
    pub protocol_income_to_be_distributed: Uint256,
}

impl Reserve {
    /// Initialize new reserve
    pub fn create(
        block_time: u64,
        index: u32,
        asset_type: AssetType,
        params: InitOrUpdateAssetParams,
    ) -> StdResult<Self> {
        // Verify if there are all params
        params.validate_availability_of_all_params()?;

        // Verify correctness of params
        params.validate_for_initialization()?;

        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            borrow_slope,
            loan_to_value,
            reserve_factor,
            liquidation_threshold,
            liquidation_bonus,
        } = params;

        // Unwraps on params are save (validated with `validate_availability_of_all_params`)
        let new_reserve = Reserve {
            index,
            asset_type,
            ma_token_address: CanonicalAddr::default(),
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            borrow_rate: Decimal256::zero(),
            liquidity_rate: Decimal256::zero(),
            borrow_slope: borrow_slope.unwrap(),
            loan_to_value: loan_to_value.unwrap(),
            reserve_factor: reserve_factor.unwrap(),
            interests_last_updated: block_time,
            debt_total_scaled: Uint256::zero(),
            liquidation_threshold: liquidation_threshold.unwrap(),
            liquidation_bonus: liquidation_bonus.unwrap(),
        };
        Ok(new_reserve)
    }

    /// Update reserve based on new params
    pub fn update_with(self, params: InitOrUpdateAssetParams) -> StdResult<Self> {
        // Verify correctness of params
        params.validate_for_update(self.loan_to_value, self.liquidation_threshold)?;

        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            borrow_slope,
            loan_to_value,
            reserve_factor,
            liquidation_threshold,
            liquidation_bonus,
        } = params;

        let updated_reserve = Reserve {
            borrow_slope: borrow_slope.unwrap_or(self.borrow_slope),
            loan_to_value: loan_to_value.unwrap_or(self.loan_to_value),
            reserve_factor: reserve_factor.unwrap_or(self.reserve_factor),
            liquidation_threshold: liquidation_threshold.unwrap_or(self.liquidation_threshold),
            liquidation_bonus: liquidation_bonus.unwrap_or(self.liquidation_bonus),
            ..self
        };
        Ok(updated_reserve)
    }
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
    pub collateral_assets: Uint128,
}

impl Default for User {
    fn default() -> Self {
        User {
            borrowed_assets: Uint128::zero(),
            collateral_assets: Uint128::zero(),
        }
    }
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
// TODO: If we do not use the struct for anything else this struct should be deleted and
// the bucket should just store Vec<u8>
pub struct ReserveReferences {
    /// Reference of reserve
    pub reference: Vec<u8>,
}

pub fn reserve_references_state<S: Storage>(storage: &mut S) -> Bucket<S, ReserveReferences> {
    bucket(RESERVE_REFERENCES_NAMESPACE, storage)
}

pub fn reserve_references_state_read<S: Storage>(
    storage: &S,
) -> ReadonlyBucket<S, ReserveReferences> {
    bucket_read(RESERVE_REFERENCES_NAMESPACE, storage)
}

pub fn reserve_ma_tokens_state<S: Storage>(storage: &mut S) -> Bucket<S, Vec<u8>> {
    bucket(RESERVE_MA_TOKENS_NAMESPACE, storage)
}

pub fn reserve_ma_tokens_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Vec<u8>> {
    bucket_read(RESERVE_MA_TOKENS_NAMESPACE, storage)
}

/// Uncollateralized loan limits
pub fn uncollateralized_loan_limits<'a, S: Storage>(
    storage: &'a mut S,
    asset: &[u8],
) -> Bucket<'a, S, Uint128> {
    Bucket::multilevel(&[UNCOLLATERALIZED_LOAN_LIMITS_NAMESPACE, asset], storage)
}

pub fn uncollateralized_loan_limits_read<'a, S: Storage>(
    storage: &'a S,
    asset: &[u8],
) -> ReadonlyBucket<'a, S, Uint128> {
    ReadonlyBucket::multilevel(&[UNCOLLATERALIZED_LOAN_LIMITS_NAMESPACE, asset], storage)
}
