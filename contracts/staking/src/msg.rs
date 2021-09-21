use cosmwasm_std::{CosmosMsg, Decimal, Uint128};

use cw20::Cw20ReceiveMsg;
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
    pub astroport_factory_address: Option<String>,
    pub astroport_max_spread: Option<Decimal>,
    pub cooldown_duration: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Update staking config
    UpdateConfig { config: CreateOrUpdateConfig },

    /// Implementation for cw20 receive msg
    Receive(Cw20ReceiveMsg),

    /// Close Claim sending the claimable Mars to the specified address (sender is the default)
    Claim { recipient: Option<String> },

    /// Execute Cosmos msg. Only callable by owner.
    ExecuteCosmosMsg(CosmosMsg),

    /// Swap any asset on the contract to uusd. Meant for received protocol rewards
    /// as a middle step to be converted to Mars.
    SwapAssetToUusd {
        offer_asset_info: AssetInfo,
        amount: Option<Uint128>,
    },

    /// Swap uusd on the contract to Mars. Meant for received protocol rewards in order
    /// for them to belong to xMars holders as underlying Mars.
    SwapUusdToMars { amount: Option<Uint128> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Stake Mars and mint xMars in return
    Stake {
        /// Address to receive the xMars tokens. Set to sender if not specified
        recipient: Option<String>,
    },

    /// Burn xMars and initiate a cooldown period on which the underlying Mars
    /// will be claimable
    Unstake {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Get contract config
    Config {},
    /// Get contract global state
    GlobalState {},
    /// Get open claim for given user
    Claim { user_address: String },
}
