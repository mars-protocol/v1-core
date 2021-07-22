use cosmwasm_std::{Addr, CosmosMsg, Decimal, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use terraswap::asset::AssetInfo;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub owner: String,
    pub terraswap_factory_address: String,
    pub terraswap_max_spread: Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Execute Cosmos msg (only callable by owner)
    ExecuteCosmosMsg(CosmosMsg),

    /// Update contract config (only callable by owner)
    UpdateConfig {
        owner: Option<String>,
        terraswap_factory_address: Option<String>,
        terraswap_max_spread: Option<Decimal>,
    },

    /// Swap any asset on the contract to uusd
    SwapAssetToUusd {
        offer_asset_info: AssetInfo,
        amount: Option<Uint128>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: Addr,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
