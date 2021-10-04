use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use cosmwasm_std::StdResult;

use crate::math::decimal::Decimal;

pub trait InterestRateModel {
    /// Updates borrow and liquidity rates based on model parameters
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal,
        borrow_rate: Decimal,
        reserve_factor: Decimal,
    ) -> StdResult<(Decimal, Decimal)>;

    /// Validate model parameters
    fn validate(&self) -> Result<(), InterestRateModelError>;
}

#[derive(Error, Debug, PartialEq)]
pub enum InterestRateModelError {
    #[error("max_borrow_rate should be greater than or equal to min_borrow_rate. max_borrow_rate: {max_borrow_rate:?}, min_borrow_rate: {min_borrow_rate:?}")]
    InvalidMinMaxBorrowRate {
        max_borrow_rate: Decimal,
        min_borrow_rate: Decimal,
    },

    #[error("Optimal utilization rate can't be greater than one")]
    InvalidOptimalUtilizationRate {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InterestRateStrategy {
    Dynamic(DynamicInterestRate),
    Linear(LinearInterestRate),
}

impl InterestRateModel for InterestRateStrategy {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal,
        borrow_rate: Decimal,
        reserve_factor: Decimal,
    ) -> StdResult<(Decimal, Decimal)> {
        match self {
            InterestRateStrategy::Dynamic(dynamic) => dynamic.get_updated_interest_rates(
                current_utilization_rate,
                borrow_rate,
                reserve_factor,
            ),
            InterestRateStrategy::Linear(linear) => linear.get_updated_interest_rates(
                current_utilization_rate,
                borrow_rate,
                reserve_factor,
            ),
        }
    }

    fn validate(&self) -> Result<(), InterestRateModelError> {
        match self {
            InterestRateStrategy::Dynamic(dynamic) => dynamic.validate(),
            InterestRateStrategy::Linear(linear) => linear.validate(),
        }
    }
}

/// Dynamic interest rate model
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DynamicInterestRate {
    /// Minimum borrow rate
    pub min_borrow_rate: Decimal,
    /// Maximum borrow rate
    pub max_borrow_rate: Decimal,
    /// Proportional parameter for the PID controller
    pub kp_1: Decimal,
    /// Optimal utilization rate targeted by the PID controller. Interest rate will decrease when lower and increase when higher
    pub optimal_utilization_rate: Decimal,
    /// Min error that triggers Kp augmentation
    pub kp_augmentation_threshold: Decimal,
    /// Kp value when error threshold is exceeded
    pub kp_2: Decimal,
}

impl InterestRateModel for DynamicInterestRate {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal,
        borrow_rate: Decimal,
        reserve_factor: Decimal,
    ) -> StdResult<(Decimal, Decimal)> {
        // error_value should be represented as integer number so we do this with help from boolean flag
        let (error_value, error_positive) =
            if self.optimal_utilization_rate > current_utilization_rate {
                (
                    self.optimal_utilization_rate - current_utilization_rate,
                    true,
                )
            } else {
                (
                    current_utilization_rate - self.optimal_utilization_rate,
                    false,
                )
            };

        let kp = if error_value >= self.kp_augmentation_threshold {
            self.kp_2
        } else {
            self.kp_1
        };

        let p = kp.checked_mul(error_value)?;
        let mut new_borrow_rate = if error_positive {
            // error_positive = true (u_optimal > u) means we want utilization rate to go up
            // we lower interest rate so more people borrow
            if borrow_rate > p {
                borrow_rate - p
            } else {
                Decimal::zero()
            }
        } else {
            // error_positive = false (u_optimal < u) means we want utilization rate to go down
            // we increase interest rate so less people borrow
            borrow_rate + p
        };

        // Check borrow rate conditions
        if new_borrow_rate < self.min_borrow_rate {
            new_borrow_rate = self.min_borrow_rate
        } else if new_borrow_rate > self.max_borrow_rate {
            new_borrow_rate = self.max_borrow_rate;
        };

        // This operation should not underflow as reserve_factor is checked to be <= 1
        let new_liquidity_rate = new_borrow_rate
            .checked_mul(current_utilization_rate)?
            .checked_mul(Decimal::one() - reserve_factor)?;

        Ok((new_borrow_rate, new_liquidity_rate))
    }

    fn validate(&self) -> Result<(), InterestRateModelError> {
        if self.min_borrow_rate > self.max_borrow_rate {
            return Err(InterestRateModelError::InvalidMinMaxBorrowRate {
                min_borrow_rate: self.min_borrow_rate,
                max_borrow_rate: self.max_borrow_rate,
            });
        }

        if self.optimal_utilization_rate > Decimal::one() {
            return Err(InterestRateModelError::InvalidOptimalUtilizationRate {});
        }

        Ok(())
    }
}

/// Linear interest rate model
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LinearInterestRate {
    /// Optimal utilization rate
    pub optimal_utilization_rate: Decimal,
    /// Base rate
    pub base: Decimal,
    /// Slope parameter for interest rate model function when utilization_rate < optimal_utilization_rate
    pub slope_1: Decimal,
    /// Slope parameter for interest rate model function when utilization_rate >= optimal_utilization_rate
    pub slope_2: Decimal,
}

impl InterestRateModel for LinearInterestRate {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal,
        _borrow_rate: Decimal,
        reserve_factor: Decimal,
    ) -> StdResult<(Decimal, Decimal)> {
        let new_borrow_rate = if current_utilization_rate <= self.optimal_utilization_rate {
            // The borrow interest rates increase slowly with utilisation
            self.base
                + self.slope_1.checked_mul(
                    current_utilization_rate.checked_div(self.optimal_utilization_rate)?,
                )?
        } else {
            // The borrow interest rates increase sharply with utilisation
            self.base
                + self.slope_1
                + self
                    .slope_2
                    .checked_mul(current_utilization_rate - self.optimal_utilization_rate)?
                    .checked_div(Decimal::one() - self.optimal_utilization_rate)?
        };

        // This operation should not underflow as reserve_factor is checked to be <= 1
        let new_liquidity_rate = new_borrow_rate
            .checked_mul(current_utilization_rate)?
            .checked_mul(Decimal::one() - reserve_factor)?;

        Ok((new_borrow_rate, new_liquidity_rate))
    }

    fn validate(&self) -> Result<(), InterestRateModelError> {
        if self.optimal_utilization_rate > Decimal::one() {
            return Err(InterestRateModelError::InvalidOptimalUtilizationRate {});
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::interest_rate_models::{DynamicInterestRate, InterestRateModel, LinearInterestRate};
    use mars::math::decimal::Decimal;

    #[test]
    fn test_dynamic_interest_rates_calculation() {
        let borrow_rate = Decimal::percent(5);
        let reserve_factor = Default::default();
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::percent(1),
            max_borrow_rate: Decimal::percent(90),

            kp_1: Decimal::from_ratio(2u128, 1u128),
            optimal_utilization_rate: Decimal::percent(60),
            kp_augmentation_threshold: Decimal::percent(10),
            kp_2: Decimal::from_ratio(3u128, 1u128),
        };

        // *
        // current utilization rate > optimal utilization rate
        // *
        let current_utilization_rate = Decimal::percent(61);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_error = current_utilization_rate - dynamic_ir.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate =
            borrow_rate + dynamic_ir.kp_1.checked_mul(expected_error).unwrap();
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate
        // *
        let current_utilization_rate = Decimal::percent(59);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_error = dynamic_ir.optimal_utilization_rate - current_utilization_rate;
        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate =
            borrow_rate - Decimal::checked_mul(dynamic_ir.kp_1, expected_error).unwrap();
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, increment KP by a multiplier if error goes beyond threshold
        // *
        let current_utilization_rate = Decimal::percent(72);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_error = current_utilization_rate - dynamic_ir.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate =
            borrow_rate + dynamic_ir.kp_2.checked_mul(expected_error).unwrap();
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate, borrow rate can't be less than min borrow rate
        // *
        let current_utilization_rate = Decimal::percent(10);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate = dynamic_ir.min_borrow_rate;
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, borrow rate can't be less than max borrow rate
        // *
        let current_utilization_rate = Decimal::percent(90);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate = dynamic_ir.max_borrow_rate;
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);
    }

    fn th_expected_liquidity_rate(br: Decimal, ur: Decimal, rf: Decimal) -> Decimal {
        br.checked_mul(ur)
            .unwrap()
            .checked_mul(Decimal::one() - rf)
            .unwrap()
    }

    #[test]
    fn test_linear_interest_rates_calculation() {
        let borrow_rate = Decimal::percent(8);
        let reserve_factor = Decimal::percent(1);
        let linear_ir = LinearInterestRate {
            optimal_utilization_rate: Decimal::percent(80),
            base: Decimal::from_ratio(0u128, 100u128),
            slope_1: Decimal::from_ratio(7u128, 100u128),
            slope_2: Decimal::from_ratio(45u128, 100u128),
        };

        // *
        // current utilization rate < optimal utilization rate
        // *
        let current_utilization_rate = Decimal::percent(79);
        let (new_borrow_rate, new_liquidity_rate) = linear_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_borrow_rate = linear_ir.base
            + linear_ir
                .slope_1
                .checked_mul(current_utilization_rate)
                .unwrap()
                .checked_div(linear_ir.optimal_utilization_rate)
                .unwrap();
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate == optimal utilization rate
        // *
        let current_utilization_rate = Decimal::percent(80);
        let (new_borrow_rate, new_liquidity_rate) = linear_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_borrow_rate = linear_ir.base
            + linear_ir
                .slope_1
                .checked_mul(current_utilization_rate)
                .unwrap()
                .checked_div(linear_ir.optimal_utilization_rate)
                .unwrap();

        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate >= optimal utilization rate
        // *
        let current_utilization_rate = Decimal::percent(81);
        let (new_borrow_rate, new_liquidity_rate) = linear_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_borrow_rate = linear_ir.base
            + linear_ir.slope_1
            + linear_ir
                .slope_2
                .checked_mul(current_utilization_rate - linear_ir.optimal_utilization_rate)
                .unwrap()
                .checked_div(Decimal::one() - linear_ir.optimal_utilization_rate)
                .unwrap();
        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate == 100% and optimal utilization rate == 100%
        // *

        let borrow_rate = Decimal::percent(0);
        let reserve_factor = Decimal::percent(1);
        let linear_ir = LinearInterestRate {
            optimal_utilization_rate: Decimal::percent(100),
            base: Decimal::from_ratio(0u128, 100u128),
            slope_1: Decimal::from_ratio(7u128, 100u128),
            slope_2: Decimal::from_ratio(0u128, 100u128),
        };

        let current_utilization_rate = Decimal::percent(100);
        let (new_borrow_rate, new_liquidity_rate) = linear_ir
            .get_updated_interest_rates(current_utilization_rate, borrow_rate, reserve_factor)
            .unwrap();

        let expected_borrow_rate = Decimal::percent(7);

        let expected_liquidity_rate = th_expected_liquidity_rate(
            expected_borrow_rate,
            current_utilization_rate,
            reserve_factor,
        );

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);
    }
}
