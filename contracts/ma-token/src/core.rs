use crate::state::balances;
use cosmwasm_std::{Api, CanonicalAddr, Extern, Querier, StdResult, Storage, Uint128};

pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    option_from: Option<&CanonicalAddr>,
    option_to: Option<&CanonicalAddr>,
    amount: Uint128,
) -> StdResult<()> {
    let mut accounts = balances(&mut deps.storage);
    if let Some(from_raw) = option_from {
        let from_balance_old = accounts.load(from_raw.as_slice()).unwrap_or_default();
        let from_balance_new = (from_balance_old - amount)?;
        accounts.save(from_raw.as_slice(), &from_balance_new)?;
    }

    if let Some(to_raw) = option_recipient {
        let to_balance_old = accounts.load(to_raw.as_slice()).unwrap_or_default();
        let to_balance_new = to_balance_old + amount;
        accounts.save(to_raw.as_slice(), &to_balance_new)?;
    }

    Ok(())
}
