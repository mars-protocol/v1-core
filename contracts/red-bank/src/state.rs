use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ContractError;
use crate::error::ContractError::{InvalidFeeShareAmounts, InvalidMaintenanceMargin};
use cosmwasm_std::{Addr, Decimal, DepsMut, Env, Timestamp, Uint128};
use cw_storage_plus::{Item, Map, U32Key};
use mars::asset::AssetType;
use mars::error::MarsError;
use mars::helpers::all_conditions_valid;

use crate::interest_rate_models::{InterestRateModel, InterestRateStrategy};
use crate::interest_rates::{apply_accumulated_interests, update_interest_rates};
use crate::msg::InitOrUpdateAssetParams;

pub const CONFIG: Item<Config> = Item::new("config");
pub const GLOBAL_STATE: Item<GlobalState> = Item::new("GLOBAL_STATE");
pub const USERS: Map<&Addr, User> = Map::new("users");
pub const MARKETS: Map<&[u8], Market> = Map::new("markets");
pub const MARKET_REFERENCES_BY_INDEX: Map<U32Key, Vec<u8>> = Map::new("market_refs_by_index");
pub const MARKET_REFERENCES_BY_MA_TOKEN: Map<&Addr, Vec<u8>> = Map::new("market_refs_by_ma_token");
pub const DEBTS: Map<(&[u8], &Addr), Debt> = Map::new("debts");
pub const UNCOLLATERALIZED_LOAN_LIMITS: Map<(&[u8], &Addr), Uint128> =
    Map::new("uncollateralized_loan_limits");

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
    /// Percentage of fees that are sent to the insurance fund
    pub insurance_fund_fee_share: Decimal,
    /// Percentage of fees that are sent to the treasury
    pub treasury_fee_share: Decimal,
}

impl Config {
    pub fn validate(&self) -> Result<(), ContractError> {
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
        if combined_fee_share > Decimal::one() {
            return Err(InvalidFeeShareAmounts {});
        }

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
    /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
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
    /// Income to be distributed to other protocol contracts
    pub protocol_income_to_distribute: Uint128,

    /// If false cannot do any action (deposit/withdraw/borrow/repay/liquidate)
    pub active: bool,
    /// If false cannot deposit
    pub deposit_enabled: bool,
    /// If false cannot borrow
    pub borrow_enabled: bool,
}

impl Market {
    /// Initialize new market
    pub fn create(
        block_time: Timestamp,
        index: u32,
        asset_type: AssetType,
        params: InitOrUpdateAssetParams,
    ) -> Result<Self, ContractError> {
        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            initial_borrow_rate: borrow_rate,
            max_loan_to_value,
            reserve_factor,
            maintenance_margin,
            liquidation_bonus,
            interest_rate_strategy,
            active,
            deposit_enabled,
            borrow_enabled,
        } = params;

        // All fields should be available
        let available = borrow_rate.is_some()
            && max_loan_to_value.is_some()
            && reserve_factor.is_some()
            && maintenance_margin.is_some()
            && liquidation_bonus.is_some()
            && interest_rate_strategy.is_some()
            && active.is_some()
            && deposit_enabled.is_some()
            && borrow_enabled.is_some();

        if !available {
            return Err(MarsError::InstantiateParamsUnavailable {}.into());
        }

        let new_market = Market {
            index,
            asset_type,
            ma_token_address: Addr::unchecked(""),
            borrow_index: Decimal::one(),
            liquidity_index: Decimal::one(),
            borrow_rate: borrow_rate.unwrap(),
            liquidity_rate: Decimal::zero(),
            max_loan_to_value: max_loan_to_value.unwrap(),
            reserve_factor: reserve_factor.unwrap(),
            interests_last_updated: block_time.seconds(),
            debt_total_scaled: Uint128::zero(),
            maintenance_margin: maintenance_margin.unwrap(),
            liquidation_bonus: liquidation_bonus.unwrap(),
            protocol_income_to_distribute: Uint128::zero(),
            interest_rate_strategy: interest_rate_strategy.unwrap(),
            active: active.unwrap(),
            deposit_enabled: deposit_enabled.unwrap(),
            borrow_enabled: borrow_enabled.unwrap(),
        };

        new_market.validate()?;

        Ok(new_market)
    }

    fn validate(&self) -> Result<(), ContractError> {
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
            return Err(InvalidMaintenanceMargin {
                maintenance_margin: self.maintenance_margin,
                max_loan_to_value: self.max_loan_to_value,
            });
        }

        Ok(())
    }

    /// Update market based on new params
    pub fn update(
        mut self,
        deps: &DepsMut,
        env: &Env,
        reference: &[u8],
        params: InitOrUpdateAssetParams,
    ) -> Result<(Self, bool), ContractError> {
        // Destructuring a struct’s fields into separate variables in order to force
        // compile error if we add more params
        let InitOrUpdateAssetParams {
            initial_borrow_rate: _,
            max_loan_to_value,
            reserve_factor,
            maintenance_margin,
            liquidation_bonus,
            interest_rate_strategy,
            active,
            deposit_enabled,
            borrow_enabled,
        } = params;

        // If reserve factor or interest rates are updated we update indexes with
        // current values before applying the change to prevent applying this
        // new params to a period where they were not valid yet. Interests rates are
        // recalculated after changes are applied.
        let should_update_interest_rates = (reserve_factor.is_some()
            && reserve_factor.unwrap() != self.reserve_factor)
            || interest_rate_strategy.is_some();

        if should_update_interest_rates {
            apply_accumulated_interests(env, &mut self);
        }

        let mut updated_market = Market {
            max_loan_to_value: max_loan_to_value.unwrap_or(self.max_loan_to_value),
            reserve_factor: reserve_factor.unwrap_or(self.reserve_factor),
            maintenance_margin: maintenance_margin.unwrap_or(self.maintenance_margin),
            liquidation_bonus: liquidation_bonus.unwrap_or(self.liquidation_bonus),
            interest_rate_strategy: interest_rate_strategy.unwrap_or(self.interest_rate_strategy),
            active: active.unwrap_or(self.active),
            deposit_enabled: deposit_enabled.unwrap_or(self.deposit_enabled),
            borrow_enabled: borrow_enabled.unwrap_or(self.borrow_enabled),
            ..self
        };

        updated_market.validate()?;

        if should_update_interest_rates {
            update_interest_rates(deps, env, reference, &mut updated_market, Uint128::zero())?;
        }

        Ok((updated_market, should_update_interest_rates))
    }

    pub fn allow_deposit(&self) -> bool {
        self.active && self.deposit_enabled
    }

    pub fn allow_withdraw(&self) -> bool {
        self.active
    }

    pub fn allow_borrow(&self) -> bool {
        self.active && self.borrow_enabled
    }

    pub fn allow_repay(&self) -> bool {
        self.active
    }

    pub fn allow_liquidate(&self) -> bool {
        self.active
    }
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
