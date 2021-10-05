pub mod interest_rate_models;
pub mod msg;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use cosmwasm_std::{Addr, Uint128};

use crate::asset::AssetType;
use crate::error::MarsError;
use crate::helpers::{all_conditions_valid, zero_address};
use crate::math::decimal::Decimal;

use self::interest_rate_models::{
    DynamicInterestRate, InterestRateModel, InterestRateModelError, InterestRateStrategy,
};

/// Global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider returns addresses for all protocol contracts
    pub address_provider_address: Addr,
    /// maToken code id used to instantiate new tokens
    pub ma_token_code_id: u64,
    /// Maximum percentage of outstanding debt that can be covered by a liquidator
    pub close_factor: Decimal,
}

impl Config {
    pub fn validate(&self) -> Result<(), MarsError> {
        let conditions_and_names =
            vec![(Self::less_or_equal_one(&self.close_factor), "close_factor")];
        all_conditions_valid(conditions_and_names)?;

        Ok(())
    }

    fn less_or_equal_one(value: &Decimal) -> bool {
        value.le(&Decimal::one())
    }
}

/// RedBank global state
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GlobalState {
    /// Market count
    pub market_count: u32,
}

/// Asset markets
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Market {
    /// Market index (Bit position on data)
    pub index: u32,
    /// maToken contract address
    pub ma_token_address: Addr,
    /// Indicated whether the asset is native or a cw20 token
    pub asset_type: AssetType,

    /// Max uusd that can be borrowed per uusd collateral when using the asset as collateral
    pub max_loan_to_value: Decimal,
    /// LTV (if using only the asset as collateral) that if surpassed, makes the user's position
    /// liquidatable
    pub maintenance_margin: Decimal,
    /// Bonus amount of collateral liquidator get when repaying user's debt (Will get collateral
    /// from user in an amount equal to debt repayed + bonus)
    pub liquidation_bonus: Decimal,
    /// Portion of the borrow rate that is kept as protocol rewards
    pub reserve_factor: Decimal,

    /// Interest rate strategy to calculate borrow_rate and liquidity_rate
    pub interest_rate_strategy: InterestRateStrategy,

    /// Borrow index (Used to compute borrow interest)
    pub borrow_index: Decimal,
    /// Liquidity index (Used to compute deposit interest)
    pub liquidity_index: Decimal,
    /// Rate charged to borrowers
    pub borrow_rate: Decimal,
    /// Rate paid to depositors
    pub liquidity_rate: Decimal,
    /// Timestamp (seconds) where indexes and rates where last updated
    pub interests_last_updated: u64,

    /// Total debt scaled for the market's currency
    pub debt_total_scaled: Uint128,

    /// If false cannot do any action (deposit/withdraw/borrow/repay/liquidate)
    pub active: bool,
    /// If false cannot deposit
    pub deposit_enabled: bool,
    /// If false cannot borrow
    pub borrow_enabled: bool,
}

impl Market {
    pub fn validate(&self) -> Result<(), MarketError> {
        self.interest_rate_strategy.validate()?;

        // max_loan_to_value, reserve_factor, maintenance_margin and liquidation_bonus should be less or equal 1
        let conditions_and_names = vec![
            (
                self.max_loan_to_value.le(&Decimal::one()),
                "max_loan_to_value",
            ),
            (self.reserve_factor.le(&Decimal::one()), "reserve_factor"),
            (
                self.maintenance_margin.le(&Decimal::one()),
                "maintenance_margin",
            ),
            (
                self.liquidation_bonus.le(&Decimal::one()),
                "liquidation_bonus",
            ),
        ];
        all_conditions_valid(conditions_and_names)?;

        // maintenance_margin should be greater than max_loan_to_value
        if self.maintenance_margin <= self.max_loan_to_value {
            return Err(MarketError::InvalidMaintenanceMargin {
                maintenance_margin: self.maintenance_margin,
                max_loan_to_value: self.max_loan_to_value,
            });
        }

        Ok(())
    }
}

impl Default for Market {
    fn default() -> Self {
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::zero(),
            max_borrow_rate: Decimal::one(),
            kp_1: Default::default(),
            optimal_utilization_rate: Default::default(),
            kp_augmentation_threshold: Default::default(),
            kp_2: Default::default(),
        };
        Market {
            index: 0,
            ma_token_address: zero_address(),
            liquidity_index: Default::default(),
            borrow_index: Default::default(),
            borrow_rate: Default::default(),
            liquidity_rate: Default::default(),
            max_loan_to_value: Default::default(),
            reserve_factor: Default::default(),
            interests_last_updated: 0,
            debt_total_scaled: Default::default(),
            asset_type: AssetType::Native,
            maintenance_margin: Decimal::one(),
            liquidation_bonus: Decimal::zero(),
            interest_rate_strategy: InterestRateStrategy::Dynamic(dynamic_ir),
            active: true,
            deposit_enabled: true,
            borrow_enabled: true,
        }
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum MarketError {
    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    InterestRateModel(#[from] InterestRateModelError),

    #[error("maintenance_margin should be greater than max_loan_to_value. maintenance_margin: {maintenance_margin:?}, max_loan_to_value: {max_loan_to_value:?}")]
    InvalidMaintenanceMargin {
        maintenance_margin: Decimal,
        max_loan_to_value: Decimal,
    },
}

/// Data for individual users
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct User {
    /// bits representing borrowed assets. 1 on the corresponding bit means asset is
    /// being borrowed
    pub borrowed_assets: Uint128,
    /// bits representing collateral assets. 1 on the corresponding bit means asset is
    /// being used as collateral
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

/// Debt for each asset and user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Debt {
    /// Scaled debt amount
    pub amount_scaled: Uint128,

    /// Marker for uncollateralized debt
    pub uncollateralized: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserHealthStatus {
    NotBorrowing,
    Borrowing(Decimal),
}
// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: Addr,
    pub address_provider_address: Addr,
    pub ma_token_code_id: u64,
    pub market_count: u32,
    pub close_factor: Decimal,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MarketsListResponse {
    pub markets_list: Vec<MarketInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MarketInfo {
    /// Asset denom
    pub denom: String,
    /// Address for the corresponding maToken
    pub ma_token_address: Addr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DebtResponse {
    pub debts: Vec<DebtInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DebtInfo {
    /// Asset denom
    pub denom: String,
    /// Scaled amount stored in contract state
    pub amount_scaled: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CollateralResponse {
    pub collateral: Vec<CollateralInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CollateralInfo {
    /// Asset denom
    pub denom: String,
    /// Wether the user is using asset as collateral or not
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UserPositionResponse {
    pub total_collateral_in_uusd: Uint128,
    pub total_debt_in_uusd: Uint128,
    /// Total debt minus the uncollateralized debt
    pub total_collateralized_debt_in_uusd: Uint128,
    pub max_debt_in_uusd: Uint128,
    pub weighted_maintenance_margin_in_uusd: Uint128,
    pub health_status: UserHealthStatus,
}
