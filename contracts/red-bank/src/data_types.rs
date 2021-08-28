use cosmwasm_std::{Decimal, Uint128};
use mars::math::reverse_decimal;
use schemars::JsonSchema;
use serde::{de, ser, Deserialize, Deserializer, Serialize};
use std::{fmt, ops};

/// Scaling factor used to keep more precision during division / multiplication by index.
const SCALING_FACTOR: u128 = 1_000_000;

/// Scaled amount which needs to be descaled by index and SCALING_FACTOR.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
pub struct ScaledAmount(#[schemars(with = "String")] u128);

/// Scales the amount by factor for greater precision.
/// Example:
/// Current index is 10. We deposit 6.123456 UST (6123456 uusd). Scaled amount will be
/// 6123456 / 10 = 612345 so we loose some precision. In order to avoid this situation
/// we scale the amount by SCALING_FACTOR.
pub fn get_scaled_amount(amount: Uint128, index: Decimal) -> ScaledAmount {
    // Scale by SCALING_FACTOR to have better precision
    let scaled_amount = Uint128::from(amount.u128() * SCALING_FACTOR);
    // Different form for: scaled_amount / index
    let result = scaled_amount * reverse_decimal(index);
    ScaledAmount(result.u128())
}

/// Descales the amount introduced by `get_scaled_amount` (see function description).
pub fn get_descaled_amount(amount: ScaledAmount, index: Decimal) -> Uint128 {
    // Multiply scaled amount by decimal (index)
    let result = Uint128::from(amount.0) * index;
    // Descale by SCALING_FACTOR which is introduced by `get_scaled_amount`
    result.checked_div(Uint128::from(SCALING_FACTOR)).unwrap()
}

impl ScaledAmount {
    pub const fn zero() -> Self {
        ScaledAmount(0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl From<u128> for ScaledAmount {
    fn from(val: u128) -> Self {
        ScaledAmount(val)
    }
}

impl From<Uint128> for ScaledAmount {
    fn from(val: Uint128) -> Self {
        ScaledAmount(val.u128())
    }
}

impl Into<Uint128> for ScaledAmount {
    fn into(self) -> Uint128 {
        Uint128::new(self.0)
    }
}

impl ops::Add<ScaledAmount> for ScaledAmount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        ScaledAmount(self.0.checked_add(rhs.0).unwrap())
    }
}

impl ops::Sub<ScaledAmount> for ScaledAmount {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        ScaledAmount(self.0.checked_sub(rhs.0).unwrap())
    }
}

impl ops::AddAssign<ScaledAmount> for ScaledAmount {
    fn add_assign(&mut self, rhs: ScaledAmount) {
        self.0 = self.0.checked_add(rhs.0).unwrap();
    }
}

impl ops::SubAssign<ScaledAmount> for ScaledAmount {
    fn sub_assign(&mut self, rhs: ScaledAmount) {
        self.0 = self.0.checked_sub(rhs.0).unwrap();
    }
}

impl From<ScaledAmount> for String {
    fn from(original: ScaledAmount) -> Self {
        original.to_string()
    }
}

impl fmt::Display for ScaledAmount {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for ScaledAmount {
    /// Serializes as an integer string using base 10
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ScaledAmount {
    /// Deserialized from an integer string using base 10
    fn deserialize<D>(deserializer: D) -> Result<ScaledAmount, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(ScaledAmountVisitor)
    }
}

struct ScaledAmountVisitor;

impl<'de> de::Visitor<'de> for ScaledAmountVisitor {
    type Value = ScaledAmount;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("string-encoded integer")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        match v.parse::<u128>() {
            Ok(u) => Ok(ScaledAmount(u)),
            Err(e) => Err(E::custom(format!("invalid Uint128 '{}' - {}", v, e))),
        }
    }
}
