use crate::state::{
    balance_snapshot, balance_snapshot_info, balance_snapshot_info_read,
    total_supply_snapshot, total_supply_snapshot_info, total_supply_snapshot_info_read,
    Snapshot, SnapshotInfo,
};

use cosmwasm_std::{Api, CanonicalAddr, Env, Extern, Querier, StdResult, Storage, Uint128};

trait SnapshotOps {
    fn get_snapshot_info(&self) -> StdResult<Option<SnapshotInfo>>;
    fn save_snapshot_info(&mut self, snapshot_info: &SnapshotInfo) -> StdResult<()>;
    fn save_snapshot(&mut self, index: &[u8], snapshot: &Snapshot) -> StdResult<()>;
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

// BALANCE SNAPSHOT

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

pub fn capture_balance_snapshot<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    addr_raw: &CanonicalAddr,
    balance: Uint128,
) -> StdResult<()> {
    capture_snapshot(env, &mut BalanceSnapshotOps { deps, addr_raw }, balance) 
}

// TOTAL SUPPLY SNAPSHOT

struct TotalSupplySnapshotOps<'a, S: Storage, A: Api, Q: Querier> {
    pub deps: &'a mut Extern<S, A, Q>,
}

impl<'a, S, A, Q> SnapshotOps for TotalSupplySnapshotOps<'a, S, A, Q>
where
    S: Storage,
    A: Api,
    Q: Querier,
{

    fn get_snapshot_info(&self) -> StdResult<Option<SnapshotInfo>> {
        total_supply_snapshot_info_read(&self.deps.storage).may_load()
    }

    fn save_snapshot_info(
        &mut self,
        snapshot_info: &SnapshotInfo
    ) -> StdResult<()> {
        total_supply_snapshot_info(&mut self.deps.storage).save(snapshot_info)
    }

    fn save_snapshot(&mut self, index: &[u8], snapshot: &Snapshot) -> StdResult<()> {
        total_supply_snapshot(&mut self.deps.storage)
            .save(index, snapshot)
    }
}

pub fn capture_total_supply_snapshot<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    total_supply: Uint128,
) -> StdResult<()> {
    capture_snapshot(env, &mut TotalSupplySnapshotOps { deps }, total_supply) 
}

