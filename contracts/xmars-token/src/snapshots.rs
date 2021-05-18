use schemars::JsonSchema;
use std::any::type_name; 
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use cosmwasm_std::{to_vec, from_slice, CanonicalAddr, Env, StdError, StdResult, Storage, Uint128};
use cosmwasm_storage::{to_length_prefixed, to_length_prefixed_nested};

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
/// Metadata snapshots for a given address
pub struct SnapshotInfo {
    /// Index where snapshot search should start (Could be different than 0 if, in the
    /// future, sample should get smaller than all available to guarantee less operations
    /// when searching for a snapshot
    pub start_index: u64,
    /// Last index for snapshot search
    pub end_index: u64,
    /// Last block a snapshot was taken
    pub end_block: u64,
}

fn may_load_snapshot_info<S: Storage>(storage: &S, key: &[u8])
-> StdResult<Option<SnapshotInfo>> {
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

fn save_snapshot<S: Storage>(
    storage: &mut S,
    namespace: &[u8],
    index: u64,
    snapshot: &Snapshot,
) -> StdResult<()> {
    storage.set(&kv_build_key(namespace, &index.to_be_bytes()), &to_vec(snapshot)?);
    Ok(())
} 

// STORAGE HELPERS (Taken from cosmwasm storage)
/// may_deserialize parses json bytes from storage (Option), returning Ok(None) if no data present
///
/// value is an odd type, but this is meant to be easy to use with output from storage.get (Option<Vec<u8>>)
/// and value.map(|s| s.as_slice()) seems trickier than &value
fn may_deserialize<T: DeserializeOwned>(
    value: &Option<Vec<u8>>,
) -> StdResult<Option<T>> {
    match value {
        Some(vec) => Ok(Some(from_slice(&vec)?)),
        None => Ok(None),
    }
}

/// must_deserialize parses json bytes from storage (Option), returning NotFound error if no data present
fn must_deserialize<T: DeserializeOwned>(value: &Option<Vec<u8>>) -> StdResult<T> {
    match value {
        Some(vec) => from_slice(&vec),
        None => Err(StdError::not_found(type_name::<T>())),
    }
}

#[inline]
fn kv_build_key(namespace: &[u8], key: &[u8]) -> Vec<u8> {
    let mut k = namespace.to_vec();
    k.extend_from_slice(key);
    k
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
    let mut snapshot_info = 
       match may_load_snapshot_info(storage, snapshot_info_key)? {
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
        save_snapshot_info(storage, snapshot_info_key, &snapshot_info)?;
    }

    // Save new balance (always end index/end block)
    save_snapshot(
        storage,
        &snapshot_namespace,
        snapshot_info.end_index,
        &Snapshot {
            block: snapshot_info.end_block,
            value: value,
        },
    )?;

    Ok(())
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
        &kv_build_key(&to_length_prefixed(PREFIX_BALANCE_SNAPSHOT_INFO), addr_raw.as_slice()),
        to_length_prefixed_nested(&[PREFIX_BALANCE_SNAPSHOT, addr_raw.as_slice()]).as_slice(),
        balance
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
        total_supply
    ) 
}

