use crate::state::{
    balance_snapshot, balance_snapshot_info, balance_snapshot_info_read, balances, token_info, Snapshot, SnapshotInfo,
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
        capture_balance_snapshot(deps, &env, &sender_raw, sender_balance_new)?;
    }

    if let Some((rcpt_raw, rcpt_balance_new)) = option_rcpt_balance_new {
        capture_balance_snapshot(deps, &env, &rcpt_raw, rcpt_balance_new)?;
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

trait SnapshotOps {
    fn get_snapshot_info(&self) -> StdResult<Option<SnapshotInfo>>;
    fn save_snapshot_info(&mut self, snapshot_info: &SnapshotInfo) -> StdResult<()>;
    fn save_snapshot(&mut self, index: &[u8], snapshot: &Snapshot) -> StdResult<()>;
}

struct BalanceSnapshotOps<'a, S: Storage, A: Api, Q: Querier> {
    pub deps: &'a mut Extern<S, A, Q>,
    pub addr_raw: &'a CanonicalAddr,
}

impl<'a, S, A, Q> SnapshotOps for BalanceSnapshotOps<'a, S, A, Q>
where
    S: Storage,
    A: Api,
    Q: Querier,
{

    fn get_snapshot_info(&self) -> StdResult<Option<SnapshotInfo>> {
        balance_snapshot_info_read(&self.deps.storage).may_load(self.addr_raw.as_slice())
    }

    fn save_snapshot_info(
        &mut self,
        snapshot_info: &SnapshotInfo
    ) -> StdResult<()> {
        balance_snapshot_info(&mut self.deps.storage).save(self.addr_raw.as_slice(), snapshot_info)
    }

    fn save_snapshot(&mut self, index: &[u8], snapshot: &Snapshot) -> StdResult<()> {
        balance_snapshot(&mut self.deps.storage, self.addr_raw)
            .save(index, snapshot)
    }
}

fn capture_balance_snapshot<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    addr_raw: &CanonicalAddr,
    balance: Uint128,
) -> StdResult<()> {
    capture_snapshot(env, &mut BalanceSnapshotOps { deps, addr_raw }, balance) 
}

fn capture_snapshot(
    env: &Env,
    snapshot_ops: &mut impl SnapshotOps,
    value: Uint128,
) -> StdResult<()> {
    // Update snapshot info
    let mut persist_snapshot_info = false;
    let mut snapshot_info = 
       match snapshot_ops.get_snapshot_info()? {
           Some(some_snapshot_info) => some_snapshot_info,
           None => {
               persist_snapshot_info = true;
               SnapshotInfo {
                   start_index: 0,
                   end_index: 0,
                   end_block: env.block.height,
               }
           },
       };

    if snapshot_info.end_block != env.block.height {
        persist_snapshot_info = true;
        snapshot_info.end_index += 1;
        snapshot_info.end_block = env.block.height;
    }

    if persist_snapshot_info {
        snapshot_ops.save_snapshot_info(&snapshot_info)?;
    }

    // Save new balance (always end index/end block)
    snapshot_ops.save_snapshot(
        &snapshot_info.end_index.to_be_bytes(),
        &Snapshot {
            block: snapshot_info.end_block,
            value: value,
        },
    )?;

    Ok(())
}
