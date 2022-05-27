use cosmwasm_std::{
    attr, entry_point, Addr, Attribute, DepsMut, Empty, Env, Event, Order, Response, StdResult,
};
use cw20_base::state::BALANCES;
use cw_storage_plus::Bound;

#[cfg_attr(not(feature = "library"), entry_point)]
/// delete all user addresses whose balances are zero
pub fn migrate(deps: DepsMut, _env: Env, _msg: Empty) -> StdResult<Response> {
    let mut start_after: Option<Bound> = None;
    let mut attrs: Vec<Attribute> = vec![];

    #[allow(while_true)]
    while true {
        // grab the addresses and balances of the first 10 users
        let users_balances = BALANCES
            .range(deps.storage, start_after, None, Order::Ascending)
            .take(10)
            .map(|item| -> StdResult<_> {
                let (user_bytes, balance) = item?;
                let user = String::from_utf8(user_bytes)?;
                Ok((Addr::unchecked(user), balance))
            })
            .collect::<StdResult<Vec<_>>>()?;

        // exit the loop if there is no more user positions to handle
        if users_balances.is_empty() {
            break;
        }

        // split the `users_balances` vector into users with zero balances and those with non-zero balances
        let (zeroes, nonzeroes): (Vec<_>, Vec<_>) = users_balances
            .into_iter()
            .partition(|(_, balance)| balance.is_zero());

        // delete all users with zero balance
        for (user, _) in &zeroes {
            BALANCES.remove(deps.storage, user);
            attrs.push(attr("user", user));
        }

        // update the pagination parameter
        start_after = nonzeroes
            .last()
            .map(|(user, _)| Bound::Exclusive(user.as_bytes().to_vec()));
    }

    Ok(Response::new().add_event(Event::new("mars_ma_token/storage_purged").add_attributes(attrs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::Uint128;
    use cw20_base::enumerable::query_all_accounts;

    #[test]
    fn purging_storage() {
        let mut deps = mock_dependencies(&[]);

        let users_balances: Vec<(&str, u128)> = vec![
            // the first 10 are all zero, so all purged
            ("a", 0),
            ("b", 0),
            ("c", 0),
            ("d", 0),
            ("e", 0),
            ("f", 0),
            ("g", 0),
            ("h", 0),
            ("i", 0),
            ("j", 0),
            // the next 10, half are zero; should update pagination to start after user `s`
            ("k", 11),
            ("l", 0),
            ("m", 13),
            ("n", 0),
            ("o", 15),
            ("p", 0),
            ("q", 17),
            ("r", 0),
            ("s", 19),
            ("t", 0),
            // the final few, all non-zero; should start after `z`
            ("u", 21),
            ("v", 22),
            ("w", 23),
            ("x", 24),
            ("y", 25),
            ("z", 26),
        ];

        for (user, balance) in users_balances {
            BALANCES
                .save(
                    deps.as_mut().storage,
                    &Addr::unchecked(user),
                    &Uint128::new(balance),
                )
                .unwrap();
        }

        // execute the migration
        migrate(deps.as_mut(), mock_env(), Empty {}).unwrap();

        // query all accounts after migration; should only return ones with non-zero balances
        let res = query_all_accounts(deps.as_ref(), None, Some(100)).unwrap();
        assert_eq!(
            res.accounts,
            vec!["k", "m", "o", "q", "s", "u", "v", "w", "x", "y", "z"]
        );
    }
}
