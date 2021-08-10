use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, StdError, StdResult, Timestamp, Uint128};
use cw_storage_plus::{Item, Map, U32Key};
use mars::asset::AssetType;
use mars::helpers::all_conditions_valid;
use mars::red_bank::msg::InitOrUpdateAssetParams;

pub const CONFIG: Item<Config> = Item::new("config");
pub const RED_BANK: Item<RedBank> = Item::new("red_bank");
pub const USERS: Map<&Addr, User> = Map::new("users");
pub const MARKETS: Map<&[u8], Market> = Map::new("markets");
pub const MARKET_REFERENCES: Map<U32Key, MarketReferences> = Map::new("market_references");
pub const MARKET_MA_TOKENS: Map<&Addr, Vec<u8>> = Map::new("market_ma_tokens");
pub const DEBTS: Map<(&[u8], &Addr), Debt> = Map::new("debts");
pub const UNCOLLATERALIZED_LOAN_LIMITS: Map<(&[u8], &Addr), Uint128> =
    Map::new("uncollateralized_loan_limits");

/// Lending pool global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider returns addresses for all protocol contracts
    pub address_provider_address: Addr,
    /// maToken code id used to instantiate new tokens
    pub ma_token_code_id: u64,
    /// Maximum percentage of outstanding debt that can be covered by a liquidator
    pub close_factor: Decimal256,
    /// Percentage of fees that are sent to the insurance fund
    pub insurance_fund_fee_share: Decimal256,
    /// Percentage of fees that are sent to the treasury
    pub treasury_fee_share: Decimal256,
}

impl Config {
    pub fn validate(&self) -> StdResult<()> {
        let conditions_and_names = vec![
            (Self::less_or_equal_one(&self.close_factor), "close_factor"),
            (
                Self::less_or_equal_one(&self.insurance_fund_fee_share),
                "insurance_fund_fee_share",
            ),
            (
                Self::less_or_equal_one(&self.treasury_fee_share),
                "treasury_fee_share",
            ),
        ];
        all_conditions_valid(conditions_and_names)?;

        let combined_fee_share = self.insurance_fund_fee_share + self.treasury_fee_share;
        // Combined fee shares cannot exceed one
        if combined_fee_share > Decimal256::one() {
            return Err(StdError::generic_err(
                "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one",
            ));
        }

        Ok(())
    }

    fn less_or_equal_one(value: &Decimal256) -> bool {
        value.le(&Decimal256::one())
    }
}

/// RedBank global state
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct RedBank {
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

    /// Borrow index (Used to compute borrow interest)
    pub borrow_index: Decimal256,
    /// Liquidity index (Used to compute deposit interest)
    pub liquidity_index: Decimal256,
    /// Rate charged to borrowers
    pub borrow_rate: Decimal256,
    /// Minimum borrow rate
    pub min_borrow_rate: Decimal256,
    /// Maximum borrow rate
    pub max_borrow_rate: Decimal256,
    /// Rate paid to depositors
    pub liquidity_rate: Decimal256,

    /// Max percentage of collateral that can be borrowed
    pub max_loan_to_value: Decimal256,

    /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
    pub reserve_factor: Decimal256,

    /// Timestamp (seconds) where indexes and rates where last updated
    pub interests_last_updated: u64,
    /// Total debt scaled for the market's currency
    pub debt_total_scaled: Uint256,

    /// Indicated whether the asset is native or a cw20 token
    pub asset_type: AssetType,

    /// Percentage at which the loan is defined as under-collateralized
    pub maintenance_margin: Decimal256,
    /// Bonus on the price of assets of the collateral when liquidators purchase it
    pub liquidation_bonus: Decimal256,

    /// Income to be distributed to other protocol contracts
    pub protocol_income_to_distribute: Uint256,

    /// PID parameters
    pub pid_parameters: PidParameters,
}

/// PID parameters
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PidParameters {
    /// Proportional parameter for the PID controller
    pub kp_1: Decimal256,
    /// Optimal utilization rate targeted by the PID controller. Interest rate will decrease when lower and increase when higher
    pub optimal_utilization_rate: Decimal256,
    /// Min error that triggers Kp augmentation
    pub kp_augmentation_threshold: Decimal256,
    /// Kp value when error threshold is exceeded
    pub kp_2: Decimal256,
}

impl Market {
    /// Initialize new market
    pub fn create(
        block_time: Timestamp,
        index: u32,
        asset_type: AssetType,
        params: InitOrUpdateAssetParams,
    ) -> StdResult<Self> {
        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            initial_borrow_rate: borrow_rate,
            min_borrow_rate,
            max_borrow_rate,
            max_loan_to_value,
            reserve_factor,
            maintenance_margin,
            liquidation_bonus,
            kp_1,
            optimal_utilization_rate,
            kp_augmentation_threshold,
            kp_2,
        } = params;

        // All fields should be available
        let available = borrow_rate.is_some()
            && min_borrow_rate.is_some()
            && max_borrow_rate.is_some()
            && max_loan_to_value.is_some()
            && reserve_factor.is_some()
            && maintenance_margin.is_some()
            && liquidation_bonus.is_some()
            && kp_1.is_some()
            && optimal_utilization_rate.is_some()
            && kp_augmentation_threshold.is_some()
            && kp_2.is_some();

        if !available {
            return Err(StdError::generic_err(
                "All params should be available during initialization",
            ));
        }

        // Unwraps on params are save (validated with `validate_availability_of_all_params`)
        let new_pid_params = PidParameters {
            kp_1: kp_1.unwrap(),
            optimal_utilization_rate: optimal_utilization_rate.unwrap(),
            kp_augmentation_threshold: kp_augmentation_threshold.unwrap(),
            kp_2: kp_2.unwrap(),
        };
        let new_market = Market {
            index,
            asset_type,
            ma_token_address: Addr::unchecked(""),
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            borrow_rate: borrow_rate.unwrap(),
            min_borrow_rate: min_borrow_rate.unwrap(),
            max_borrow_rate: max_borrow_rate.unwrap(),
            liquidity_rate: Decimal256::zero(),
            max_loan_to_value: max_loan_to_value.unwrap(),
            reserve_factor: reserve_factor.unwrap(),
            interests_last_updated: block_time.seconds(),
            debt_total_scaled: Uint256::zero(),
            maintenance_margin: maintenance_margin.unwrap(),
            liquidation_bonus: liquidation_bonus.unwrap(),
            protocol_income_to_distribute: Uint256::zero(),
            pid_parameters: new_pid_params,
        };

        new_market.validate()?;

        Ok(new_market)
    }

    fn validate(&self) -> StdResult<()> {
        if self.min_borrow_rate >= self.max_borrow_rate {
            return Err(StdError::generic_err(format!(
                "max_borrow_rate should be greater than min_borrow_rate. \
                    max_borrow_rate: {}, \
                    min_borrow_rate: {}",
                self.max_borrow_rate, self.min_borrow_rate
            )));
        }

        if self.pid_parameters.optimal_utilization_rate > Decimal256::one() {
            return Err(StdError::generic_err(
                "Optimal utilization rate can't be greater than one",
            ));
        }

        // max_loan_to_value, reserve_factor, maintenance_margin and liquidation_bonus should be less or equal 1
        let conditions_and_names = vec![
            (
                self.max_loan_to_value.le(&Decimal256::one()),
                "max_loan_to_value",
            ),
            (self.reserve_factor.le(&Decimal256::one()), "reserve_factor"),
            (
                self.maintenance_margin.le(&Decimal256::one()),
                "maintenance_margin",
            ),
            (
                self.liquidation_bonus.le(&Decimal256::one()),
                "liquidation_bonus",
            ),
        ];
        all_conditions_valid(conditions_and_names)?;

        // maintenance_margin should be greater than max_loan_to_value
        if self.maintenance_margin <= self.max_loan_to_value {
            return Err(StdError::generic_err(format!(
                "maintenance_margin should be greater than max_loan_to_value. \
                    maintenance_margin: {}, \
                    max_loan_to_value: {}",
                self.maintenance_margin, self.max_loan_to_value
            )));
        }

        Ok(())
    }

    /// Update market based on new params
    pub fn update_with(self, params: InitOrUpdateAssetParams) -> StdResult<Self> {
        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            initial_borrow_rate: _,
            min_borrow_rate,
            max_borrow_rate,
            max_loan_to_value,
            reserve_factor,
            maintenance_margin,
            liquidation_bonus,
            kp_1: kp,
            optimal_utilization_rate: u_optimal,
            kp_augmentation_threshold,
            kp_2: kp_multiplier,
        } = params;

        let updated_pid_params = PidParameters {
            kp_1: kp.unwrap_or(self.pid_parameters.kp_1),
            optimal_utilization_rate: u_optimal
                .unwrap_or(self.pid_parameters.optimal_utilization_rate),
            kp_augmentation_threshold: kp_augmentation_threshold
                .unwrap_or(self.pid_parameters.kp_augmentation_threshold),
            kp_2: kp_multiplier.unwrap_or(self.pid_parameters.kp_2),
        };
        let updated_market = Market {
            min_borrow_rate: min_borrow_rate.unwrap_or(self.min_borrow_rate),
            max_borrow_rate: max_borrow_rate.unwrap_or(self.max_borrow_rate),
            max_loan_to_value: max_loan_to_value.unwrap_or(self.max_loan_to_value),
            reserve_factor: reserve_factor.unwrap_or(self.reserve_factor),
            maintenance_margin: maintenance_margin.unwrap_or(self.maintenance_margin),
            liquidation_bonus: liquidation_bonus.unwrap_or(self.liquidation_bonus),
            pid_parameters: updated_pid_params,
            ..self
        };

        updated_market.validate()?;

        Ok(updated_market)
    }
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

/// Debt for each asset and user
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Debt {
    /// Scaled debt amount
    // TODO(does this amount always have six decimals? How do we manage this?)
    pub amount_scaled: Uint256,

    /// Marker for uncollateralized debt
    pub uncollateralized: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
// TODO: If we do not use the struct for anything else this struct should be deleted and
// the bucket should just store Vec<u8>
pub struct MarketReferences {
    /// Reference of market
    pub reference: Vec<u8>,
}
