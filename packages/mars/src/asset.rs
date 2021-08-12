use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Represents either a native asset or a cw20. Meant to be used as part of a msg
/// in a contract call and not to be used internally
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Asset {
    Cw20 { contract_addr: String },
    Native { denom: String },
}

// TODO: Should we implement a checked/unchecked version of this?
impl Asset {
    /// Get symbol (denom/addres), reference (bytes used as key for storage) and asset type
    pub fn get_attributes(&self) -> (String, Vec<u8>, AssetType) {
        match &self {
            Asset::Native { denom } => {
                let asset_reference = denom.as_bytes().to_vec();
                (denom.to_string(), asset_reference, AssetType::Native)
            }
            Asset::Cw20 { contract_addr } => {
                let asset_reference = contract_addr.as_bytes().to_vec();
                (contract_addr.to_string(), asset_reference, AssetType::Cw20)
            }
        }
    }

    /// Return bytes used as key for storage
    pub fn get_reference(&self) -> Vec<u8> {
        match &self {
            Asset::Native { denom } => denom.as_bytes().to_vec(),
            Asset::Cw20 { contract_addr } => contract_addr.as_bytes().to_vec(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Cw20,
    Native,
}
