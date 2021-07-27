use cosmwasm_std::{from_slice, StdError, StdResult};
use serde::de::DeserializeOwned;
use std::any::type_name;

/// may_deserialize parses json bytes from storage (Option), returning Ok(None) if no data present
///
/// value is an odd type, but this is meant to be easy to use with output from storage.get (Option<Vec<u8>>)
/// and value.map(|s| s.as_slice()) seems trickier than &value
pub fn may_deserialize<T: DeserializeOwned>(value: &Option<Vec<u8>>) -> StdResult<Option<T>> {
    match value {
        Some(vec) => Ok(Some(from_slice(vec)?)),
        None => Ok(None),
    }
}

/// must_deserialize parses json bytes from storage (Option), returning NotFound error if no data present
pub fn must_deserialize<T: DeserializeOwned>(value: &Option<Vec<u8>>) -> StdResult<T> {
    match value {
        Some(vec) => from_slice(vec),
        None => Err(StdError::not_found(type_name::<T>())),
    }
}

#[inline]
pub fn kv_build_key(namespace: &[u8], key: &[u8]) -> Vec<u8> {
    let mut k = namespace.to_vec();
    k.extend_from_slice(key);
    k
}
