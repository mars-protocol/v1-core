use cosmwasm_std::{Decimal, Fraction};

pub fn decimal_division(a: Decimal, b: Decimal) -> Decimal {
    Decimal::from_ratio(a.numerator(), b.numerator())
}

pub fn decimal_multiplication(a: Decimal, b: Decimal) -> Decimal {
    a * b
}

pub fn reverse_decimal(decimal: Decimal) -> Decimal {
    decimal.inv().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::math::{decimal_division, decimal_multiplication, reverse_decimal};
    use cosmwasm_std::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_decimal_division() {
        let a = Decimal::from_ratio(99988u128, 100u128);
        let b = Decimal::from_ratio(24997u128, 100u128);
        let c = decimal_division(a, b);
        assert_eq!(c, Decimal::from_str("4.0").unwrap());

        let a = Decimal::from_ratio(123456789u128, 1000000u128);
        let b = Decimal::from_ratio(33u128, 1u128);
        let c = decimal_division(a, b);
        assert_eq!(c, Decimal::from_str("3.741114818181818181").unwrap());
    }

    #[test]
    fn test_decimal_multiplication() {
        let a = Decimal::from_ratio(33u128, 10u128);
        let b = Decimal::from_ratio(45u128, 10u128);
        let c = decimal_multiplication(a, b);
        assert_eq!(c, Decimal::from_str("14.85").unwrap());

        // max allowed number for numerator to avoid overflow
        let a = Decimal::from_ratio(340282366920u128, 1u128);
        let b = Decimal::from_ratio(12345678u128, 100000000u128);
        let c = decimal_multiplication(a, b);
        assert_eq!(c, Decimal::from_str("42010165310.7217176").unwrap());
    }

    #[test]
    fn test_reverse_decimal() {
        let a = Decimal::from_ratio(33u128, 10u128);
        let rev_a = reverse_decimal(a);
        assert_eq!(rev_a, Decimal::from_ratio(10u128, 33u128));
    }
}
