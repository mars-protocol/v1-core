use crate::state::{
    balance_snapshot, balance_snapshot_info, balances, token_info, Snapshot, SnapshotInfo,
};
use cosmwasm_std::{Api, CanonicalAddr, Env, Extern, Querier, StdResult, Storage, Uint128};

pub fn transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    option_sender: Option<&CanonicalAddr>,
    option_recipient: Option<&CanonicalAddr>,
    amount: Uint128,
) -> StdResult<()> {
    let mut accounts = balances(&mut deps.storage);
    let option_sender_balance_new = if let Some(sender_raw) = option_sender {
        let sender_balance_old = accounts.load(sender_raw.as_slice()).unwrap_or_default();
        let sender_balance_new = (sender_balance_old - amount)?;
        accounts.save(sender_raw.as_slice(), &sender_balance_new)?;
        Some((sender_raw, sender_balance_new))
    } else {
        None
    };

    let option_rcpt_balance_new = if let Some(rcpt_raw) = option_recipient {
        let rcpt_balance_old = accounts.load(rcpt_raw.as_slice()).unwrap_or_default();
        let rcpt_balance_new = rcpt_balance_old + amount;
        accounts.save(rcpt_raw.as_slice(), &rcpt_balance_new)?;
        Some((rcpt_raw, rcpt_balance_new))
    } else {
        None
    };

    if let Some((sender_raw, sender_balance_new)) = option_sender_balance_new {
        save_balance_snapshot(deps, &env, &sender_raw, sender_balance_new)?;
    }

    if let Some((rcpt_raw, rcpt_balance_new)) = option_rcpt_balance_new {
        save_balance_snapshot(deps, &env, &rcpt_raw, rcpt_balance_new)?;
    }

    Ok(())
}

pub fn burn<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    sender_raw: &CanonicalAddr,
    amount: Uint128,
) -> StdResult<()> {
    // lower balance
    transfer(deps, env, Some(&sender_raw), None, amount)?;

    // reduce total_supply
    let mut new_total_supply = Uint128::zero();
    token_info(&mut deps.storage).update(|mut info| {
        info.total_supply = (info.total_supply - amount)?;
        new_total_supply = info.total_supply;
        Ok(info)
    })?;

    //save_total_supply_snapshot(deps, &env, new_total_supply);
    Ok(())
}

fn save_balance_snapshot<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    addr_raw: &CanonicalAddr,
    balance: Uint128,
) -> StdResult<()> {
    let mut balance_snapshot_info_bucket = balance_snapshot_info(&mut deps.storage);
    // Update snapshot info
    let mut balance_snapshot_info = balance_snapshot_info_bucket
        .may_load(addr_raw.as_slice())?
        .unwrap_or(SnapshotInfo {
            start_index: 0,
            end_index: 0,
            end_block: env.block.height,
        });

    if balance_snapshot_info.end_block != env.block.height {
        balance_snapshot_info.end_index += 1;
        balance_snapshot_info.end_block = env.block.height;
    }

    balance_snapshot_info_bucket.save(addr_raw.as_slice(), &balance_snapshot_info)?;

    // Save new balance (always end index/end block)
    let mut balance_snapshot_bucket = balance_snapshot(&mut deps.storage, &addr_raw);

    balance_snapshot_bucket.save(
        &balance_snapshot_info.end_index.to_be_bytes(),
        &Snapshot {
            block: balance_snapshot_info.end_block,
            value: balance,
        },
    )?;

    Ok(())
}
