use cosmwasm_std::{to_vec, CanonicalAddr, Env, StdResult, Storage, Uint128};
use cosmwasm_storage::{to_length_prefixed, to_length_prefixed_nested};
use mars::storage::{kv_build_key, may_deserialize, must_deserialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// STATE

pub const KEY_TOTAL_SUPPLY_SNAPSHOT_INFO: &[u8] = b"total_supply_snapshot_info";
pub const PREFIX_TOTAL_SUPPLY_SNAPSHOT: &[u8] = b"total_supply_snapshot";
pub const PREFIX_BALANCE_SNAPSHOT_INFO: &[u8] = b"balance_snapshot_info";
pub const PREFIX_BALANCE_SNAPSHOT: &[u8] = b"balance_snapshot";

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
/// Snapshot for a given amount, could be applied to the total supply or to the balance of
/// a specific address
pub struct Snapshot {
    pub block: u64,
    pub value: Uint128,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
/// Snapshot metadata for a given value
pub struct SnapshotInfo {
    /// Index where snapshot search should start. Could be different than 0 if the
    /// target sample should get smaller than all available snapshots to guarantee
    /// less operations when searching for a snapshot
    pub start_index: u64,
    /// Last index for snapshot search
    pub end_index: u64,
    /// Last block a snapshot was taken
    pub end_block: u64,
}

fn may_load_snapshot_info<S: Storage>(storage: &S, key: &[u8]) -> StdResult<Option<SnapshotInfo>> {
    let value = storage.get(key);
    may_deserialize(&value)
}

fn save_snapshot_info<S: Storage>(
    storage: &mut S,
    key: &[u8],
    snapshot_info: &SnapshotInfo,
) -> StdResult<()> {
    storage.set(key, &to_vec(snapshot_info)?);
    Ok(())
}

fn load_snapshot<S: Storage>(storage: &S, namespace: &[u8], index: u64) -> StdResult<Snapshot> {
    let value = storage.get(&kv_build_key(namespace, &index.to_be_bytes()));
    must_deserialize(&value)
}

fn save_snapshot<S: Storage>(
    storage: &mut S,
    namespace: &[u8],
    index: u64,
    snapshot: &Snapshot,
) -> StdResult<()> {
    storage.set(
        &kv_build_key(namespace, &index.to_be_bytes()),
        &to_vec(snapshot)?,
    );
    Ok(())
}

// CORE
//
fn capture_snapshot<S: Storage>(
    storage: &mut S,
    env: &Env,
    snapshot_info_key: &[u8],
    snapshot_namespace: &[u8],
    value: Uint128,
) -> StdResult<()> {
    // Update snapshot info
    let mut persist_snapshot_info = false;
    let mut snapshot_info = match may_load_snapshot_info(storage, snapshot_info_key)? {
        Some(some_snapshot_info) => some_snapshot_info,
        None => {
            persist_snapshot_info = true;
            SnapshotInfo {
                start_index: 0,
                end_index: 0,
                end_block: env.block.height,
            }
        }
    };

    if snapshot_info.end_block != env.block.height {
        persist_snapshot_info = true;
        snapshot_info.end_index += 1;
        snapshot_info.end_block = env.block.height;
    }

    if persist_snapshot_info {
        save_snapshot_info(storage, snapshot_info_key, &snapshot_info)?;
    }

    // Save new balance (always end index/end block)
    save_snapshot(
        storage,
        &snapshot_namespace,
        snapshot_info.end_index,
        &Snapshot {
            block: snapshot_info.end_block,
            value,
        },
    )?;

    Ok(())
}

fn get_snapshot_value_at<S: Storage>(
    storage: &S,
    snapshot_info_key: &[u8],
    snapshot_namespace: &[u8],
    block: u64,
) -> StdResult<Uint128> {
    let snapshot_info = match may_load_snapshot_info(storage, snapshot_info_key)? {
        Some(some_snapshot_info) => some_snapshot_info,
        None => return Ok(Uint128::zero()),
    };

    // If block is higher than end block, return last recorded balance
    if block >= snapshot_info.end_block {
        let value = load_snapshot(storage, snapshot_namespace, snapshot_info.end_index)?.value;
        return Ok(value);
    }

    // If block is lower than start block, return zero
    let start_snapshot = load_snapshot(storage, snapshot_namespace, snapshot_info.start_index)?;

    if block < start_snapshot.block {
        return Ok(Uint128::zero());
    }

    if block == start_snapshot.block {
        return Ok(start_snapshot.value);
    }

    let mut start_index = snapshot_info.start_index;
    let mut end_index = snapshot_info.end_index;

    let mut ret_value = start_snapshot.value;

    while end_index > start_index {
        let middle_index = end_index - ((end_index - start_index) / 2);

        let middle_snapshot = load_snapshot(storage, snapshot_namespace, middle_index)?;

        if block >= middle_snapshot.block {
            ret_value = middle_snapshot.value;
            if middle_snapshot.block == block {
                break;
            }
            start_index = middle_index;
        } else {
            end_index = middle_index - 1;
        }
    }

    Ok(ret_value)
}

// BALANCE

pub fn capture_balance_snapshot<S: Storage>(
    storage: &mut S,
    env: &Env,
    addr_raw: &CanonicalAddr,
    balance: Uint128,
) -> StdResult<()> {
    capture_snapshot(
        storage,
        env,
        &kv_build_key(
            &to_length_prefixed(PREFIX_BALANCE_SNAPSHOT_INFO),
            addr_raw.as_slice(),
        ),
        to_length_prefixed_nested(&[PREFIX_BALANCE_SNAPSHOT, addr_raw.as_slice()]).as_slice(),
        balance,
    )
}

pub fn get_balance_snapshot_value_at<S: Storage>(
    storage: &S,
    addr_raw: &CanonicalAddr,
    block: u64,
) -> StdResult<Uint128> {
    get_snapshot_value_at(
        storage,
        &kv_build_key(
            &to_length_prefixed(PREFIX_BALANCE_SNAPSHOT_INFO),
            addr_raw.as_slice(),
        ),
        to_length_prefixed_nested(&[PREFIX_BALANCE_SNAPSHOT, addr_raw.as_slice()]).as_slice(),
        block,
    )
}

// TOTAL SUPPLY

pub fn capture_total_supply_snapshot<S: Storage>(
    storage: &mut S,
    env: &Env,
    total_supply: Uint128,
) -> StdResult<()> {
    capture_snapshot(
        storage,
        env,
        &to_length_prefixed(KEY_TOTAL_SUPPLY_SNAPSHOT_INFO),
        &to_length_prefixed(PREFIX_TOTAL_SUPPLY_SNAPSHOT),
        total_supply,
    )
}

pub fn get_total_supply_snapshot_value_at<S: Storage>(
    storage: &S,
    block: u64,
) -> StdResult<Uint128> {
    get_snapshot_value_at(
        storage,
        &to_length_prefixed(KEY_TOTAL_SUPPLY_SNAPSHOT_INFO),
        &to_length_prefixed(PREFIX_TOTAL_SUPPLY_SNAPSHOT),
        block,
    )
}
