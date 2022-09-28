use crate::math::decimal::Decimal;
use cosmwasm_std::{Coin, Deps, StdResult, Uint128};

pub fn deduct_tax(deps: Deps, coin: Coin) -> StdResult<Coin> {
    let tax_amount = compute_tax(deps, &coin)?;
    Ok(Coin {
        denom: coin.denom,
        amount: coin.amount - tax_amount,
    })
}

pub fn compute_tax(deps: Deps, coin: &Coin) -> StdResult<Uint128> {
    panic!("#328")
}
