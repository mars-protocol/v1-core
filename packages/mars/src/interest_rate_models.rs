use cosmwasm_bignumber::Decimal256;
use cosmwasm_std::{StdError, StdResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub trait InterestRateModel {
    /// Updates borrow and liquidity rates based on model parameters
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal256,
        borrow_rate: Decimal256,
        reserve_factor: Decimal256,
    ) -> (Decimal256, Decimal256);

    /// Validate model parameters
    fn validate(&self) -> StdResult<()>;
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
        current_utilization_rate: Decimal256,
        borrow_rate: Decimal256,
        reserve_factor: Decimal256,
    ) -> (Decimal256, Decimal256) {
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

    fn validate(&self) -> StdResult<()> {
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
    pub min_borrow_rate: Decimal256,
    /// Maximum borrow rate
    pub max_borrow_rate: Decimal256,
    /// Proportional parameter for the PID controller
    pub kp_1: Decimal256,
    /// Optimal utilization rate targeted by the PID controller. Interest rate will decrease when lower and increase when higher
    pub optimal_utilization_rate: Decimal256,
    /// Min error that triggers Kp augmentation
    pub kp_augmentation_threshold: Decimal256,
    /// Kp value when error threshold is exceeded
    pub kp_2: Decimal256,
}

impl InterestRateModel for DynamicInterestRate {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal256,
        borrow_rate: Decimal256,
        reserve_factor: Decimal256,
    ) -> (Decimal256, Decimal256) {
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

        let p = kp * error_value;
        let mut new_borrow_rate = if error_positive {
            // error_positive = true (u_optimal > u) means we want utilization rate to go up
            // we lower interest rate so more people borrow
            if borrow_rate > p {
                borrow_rate - p
            } else {
                Decimal256::zero()
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
        let new_liquidity_rate =
            new_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        (new_borrow_rate, new_liquidity_rate)
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

        if self.optimal_utilization_rate > Decimal256::one() {
            return Err(StdError::generic_err(
                "Optimal utilization rate can't be greater than one",
            ));
        }

        Ok(())
    }
}

/// Linear interest rate model
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LinearInterestRate {
    /// Optimal utilization rate
    pub optimal_utilization_rate: Decimal256,
    /// Base rate
    pub base: Decimal256,
    /// Slope parameter for interest rate model function when utilization_rate < optimal_utilization_rate
    pub slope_1: Decimal256,
    /// Slope parameter for interest rate model function when utilization_rate >= optimal_utilization_rate
    pub slope_2: Decimal256,
}

impl InterestRateModel for LinearInterestRate {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal256,
        _borrow_rate: Decimal256,
        reserve_factor: Decimal256,
    ) -> (Decimal256, Decimal256) {
        let new_borrow_rate = if current_utilization_rate < self.optimal_utilization_rate {
            // The borrow interest rates increase slowly with utilisation
            self.base + self.slope_1 * current_utilization_rate / self.optimal_utilization_rate
        } else {
            // The borrow interest rates increase sharply with utilisation
            self.base
                + self.slope_1
                + self.slope_2 * (current_utilization_rate - self.optimal_utilization_rate)
                    / (Decimal256::one() - self.optimal_utilization_rate)
        };

        // This operation should not underflow as reserve_factor is checked to be <= 1
        let new_liquidity_rate =
            new_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        (new_borrow_rate, new_liquidity_rate)
    }

    fn validate(&self) -> StdResult<()> {
        if self.optimal_utilization_rate > Decimal256::one() {
            return Err(StdError::generic_err(
                "Optimal utilization rate can't be greater than one",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::interest_rate_models::{DynamicInterestRate, InterestRateModel};
    use cosmwasm_bignumber::Decimal256;

    #[test]
    fn test_dynamic_interest_rates_calculation() {
        let borrow_rate = Decimal256::from_ratio(5, 100);
        let reserve_factor = Default::default();
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal256::from_ratio(1, 100),
            max_borrow_rate: Decimal256::from_ratio(90, 100),
            kp_1: Decimal256::from_ratio(2, 1),
            optimal_utilization_rate: Decimal256::from_ratio(60, 100),
            kp_augmentation_threshold: Decimal256::from_ratio(10, 100),
            kp_2: Decimal256::from_ratio(3, 1),
        };

        // *
        // current utilization rate > optimal utilization rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(61, 100);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir.get_updated_interest_rates(
            current_utilization_rate,
            borrow_rate,
            reserve_factor,
        );

        let expected_error = current_utilization_rate - dynamic_ir.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate = borrow_rate + (dynamic_ir.kp_1 * expected_error);
        let expected_liquidity_rate =
            expected_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(59, 100);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir.get_updated_interest_rates(
            current_utilization_rate,
            borrow_rate,
            reserve_factor,
        );

        let expected_error = dynamic_ir.optimal_utilization_rate - current_utilization_rate;
        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate = borrow_rate - (dynamic_ir.kp_1 * expected_error);
        let expected_liquidity_rate =
            expected_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, increment KP by a multiplier if error goes beyond threshold
        // *
        let current_utilization_rate = Decimal256::from_ratio(72, 100);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir.get_updated_interest_rates(
            current_utilization_rate,
            borrow_rate,
            reserve_factor,
        );

        let expected_error = current_utilization_rate - dynamic_ir.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate = borrow_rate + (dynamic_ir.kp_2 * expected_error);
        let expected_liquidity_rate =
            expected_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate, borrow rate can't be less than min borrow rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(10, 100);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir.get_updated_interest_rates(
            current_utilization_rate,
            borrow_rate,
            reserve_factor,
        );

        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate = dynamic_ir.min_borrow_rate;
        let expected_liquidity_rate =
            expected_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, borrow rate can't be less than max borrow rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(90, 100);
        let (new_borrow_rate, new_liquidity_rate) = dynamic_ir.get_updated_interest_rates(
            current_utilization_rate,
            borrow_rate,
            reserve_factor,
        );

        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate = dynamic_ir.max_borrow_rate;
        let expected_liquidity_rate =
            expected_borrow_rate * current_utilization_rate * (Decimal256::one() - reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);
    }
}
