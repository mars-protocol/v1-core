use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use cosmwasm_std::{StdResult};

/// Represents either a native asset or a cw20. Meant to be used as part of a msg
/// in a contract call and not to be used internally
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Asset {
    Cw20 { contract_addr: String },
    Native { denom: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Cw20,
    Native,
}

/// Get symbol (denom/addres), reference (bytes used as key for storage) and asset for an
/// Asset
pub fn asset_get_attributes(asset: &Asset) -> StdResult<(String, Vec<u8>, AssetType)> {
    match asset {
        Asset::Native { denom } => {
            let asset_reference = denom.as_bytes().to_vec();
            Ok((denom.to_string(), asset_reference, AssetType::Native))
        }
        Asset::Cw20 { contract_addr } => {
            let asset_reference = contract_addr.as_bytes().to_vec();
            Ok((contract_addr.to_string(), asset_reference, AssetType::Cw20))
        }
    }
}
