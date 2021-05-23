use crate::state::balances;
use cosmwasm_std::{Api, CanonicalAddr, Extern, Querier, StdResult, Storage, Uint128};

pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    option_sender: Option<&CanonicalAddr>,
    option_recipient: Option<&CanonicalAddr>,
    amount: Uint128,
) -> StdResult<()> {
    let mut accounts = balances(&mut deps.storage);
    if let Some(sender_raw) = option_sender {
        let sender_balance_old = accounts.load(sender_raw.as_slice()).unwrap_or_default();
        let sender_balance_new = (sender_balance_old - amount)?;
        accounts.save(sender_raw.as_slice(), &sender_balance_new)?;
    }

    if let Some(rcpt_raw) = option_recipient {
        let rcpt_balance_old = accounts.load(rcpt_raw.as_slice()).unwrap_or_default();
        let rcpt_balance_new = rcpt_balance_old + amount;
        accounts.save(rcpt_raw.as_slice(), &rcpt_balance_new)?;
    }

    Ok(())
}
