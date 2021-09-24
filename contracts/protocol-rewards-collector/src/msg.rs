use cosmwasm_std::{CosmosMsg, Uint128};
use mars::asset::Asset;
use mars::math::decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use terraswap::asset::AssetInfo;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub config: CreateOrUpdateConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CreateOrUpdateConfig {
    pub owner: Option<String>,
    pub address_provider_address: Option<String>,
    pub safety_fund_fee_share: Option<Decimal>,
    pub treasury_fee_share: Option<Decimal>,
    pub astroport_factory_address: Option<String>,
    pub astroport_max_spread: Option<Decimal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Update contract config
    UpdateConfig { config: CreateOrUpdateConfig },

    /// Update asset config
    UpdateAssetConfig { asset: Asset, enabled: bool },

    /// Withdraw maTokens from the red bank
    WithdrawFromRedBank {
        asset: Asset,
        amount: Option<Uint128>,
    },

    /// Distribute the accrued protocol income to the safety fund, treasury and staking contracts,
    /// according to the split set in config.
    /// Callable by any address.
    DistributeProtocolRewards {
        /// Asset market fees to distribute
        asset: Asset,
        /// Amount to distribute to protocol contracts, defaults to contract balance if not specified
        amount: Option<Uint128>,
    },

    /// Swap any asset on the contract to uusd
    SwapAssetToUusd {
        offer_asset_info: AssetInfo,
        amount: Option<Uint128>,
    },

    /// Execute Cosmos msg (only callable by owner)
    ExecuteCosmosMsg(CosmosMsg),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Get config parameters
    Config {},
    /// Get asset config parameters
    AssetConfig { asset: Asset },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub address_provider_address: String,
    pub safety_fund_fee_share: Decimal,
    pub treasury_fee_share: Decimal,
    pub astroport_factory_address: String,
    pub astroport_max_spread: Decimal,
}
