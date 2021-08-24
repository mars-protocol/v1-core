use cosmwasm_std::{Decimal, Uint128};
use mars::math::reverse_decimal;
use schemars::JsonSchema;
use std::ops;

const SCALING_FACTOR: u128 = 1_000_000;

/// Amount which needs to be scaled by index. In order to keep better precision provided value is
/// scaled by SCALING_FACTOR.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
pub struct InputAmount(#[schemars(with = "String")] u128);

impl From<Uint128> for InputAmount {
    fn from(val: Uint128) -> Self {
        InputAmount(val.u128())
    }
}

impl ops::Div<Decimal> for InputAmount {
    type Output = Uint128;

    fn div(self, rhs: Decimal) -> Self::Output {
        // Scale by SCALING_FACTOR to have better precision in math
        let result = Uint128::from(self.0 * SCALING_FACTOR);
        // Different form for: result / rhs
        result * reverse_decimal(rhs)
    }
}

/// Scaled amount which needs to be descaled by index and SCALING_FACTOR.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
pub struct ScaledAmount(#[schemars(with = "String")] u128);

impl From<Uint128> for ScaledAmount {
    fn from(val: Uint128) -> Self {
        ScaledAmount(val.u128())
    }
}

impl ops::Mul<Decimal> for ScaledAmount {
    type Output = Uint128;

    fn mul(self, rhs: Decimal) -> Self::Output {
        // Multiply scaled amount by decimal (index)
        let result = Uint128::from(self.0) * rhs;
        // Descale by SCALING_FACTOR which is introduced in InputAmount
        // Unwrapping is safe
        result.checked_div(Uint128::from(SCALING_FACTOR)).unwrap()
    }
}
