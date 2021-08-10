use crate::state::Market;
use cosmwasm_bignumber::Decimal256;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub trait InterestRateModel {
    /// Updates borrow and liquidity rates based on model parameters
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal256,
        market: &Market,
    ) -> (Decimal256, Decimal256);
}

/// Dynamic interest rate model - PID parameters
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

impl InterestRateModel for PidParameters {
    fn get_updated_interest_rates(
        &self,
        current_utilization_rate: Decimal256,
        market: &Market,
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
            if market.borrow_rate > p {
                market.borrow_rate - p
            } else {
                Decimal256::zero()
            }
        } else {
            // error_positive = false (u_optimal < u) means we want utilization rate to go down
            // we increase interest rate so less people borrow
            market.borrow_rate + p
        };

        // Check borrow rate conditions
        if new_borrow_rate < market.min_borrow_rate {
            new_borrow_rate = market.min_borrow_rate
        } else if new_borrow_rate > market.max_borrow_rate {
            new_borrow_rate = market.max_borrow_rate;
        };

        // This operation should not underflow as reserve_factor is checked to be <= 1
        let new_liquidity_rate = new_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        (new_borrow_rate, new_liquidity_rate)
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
        market: &Market,
    ) -> (Decimal256, Decimal256) {
        let mut new_borrow_rate = if current_utilization_rate < self.optimal_utilization_rate {
            // The borrow interest rates increase slowly with utilisation
            self.base + self.slope_1 * current_utilization_rate / self.optimal_utilization_rate
        } else {
            // The borrow interest rates increase sharply with utilisation
            self.base
                + self.slope_1
                + self.slope_2 * (current_utilization_rate - self.optimal_utilization_rate)
                    / (Decimal256::one() - self.optimal_utilization_rate)
        };

        // Check borrow rate conditions
        if new_borrow_rate < market.min_borrow_rate {
            new_borrow_rate = market.min_borrow_rate
        } else if new_borrow_rate > market.max_borrow_rate {
            new_borrow_rate = market.max_borrow_rate;
        };

        // This operation should not underflow as reserve_factor is checked to be <= 1
        let new_liquidity_rate = new_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        (new_borrow_rate, new_liquidity_rate)
    }
}
