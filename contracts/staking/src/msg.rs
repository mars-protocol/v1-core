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
    pub unstake_window: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Update staking config
    UpdateConfig { config: CreateOrUpdateConfig },
    /// Implementation cw20 receive msg
    Receive(Cw20ReceiveMsg),
    /// Initialize or refresh cooldown if cooldown already active.
    /// Refresh cooldown will calculate a new cooldown using a
    /// weighted average and the difference between current xMars
    /// balance and balance at the time the first cooldown was activated.
    Cooldown {},
    /// Execute Cosmos msg. Only callable by owner.
    ExecuteCosmosMsg(CosmosMsg),
    /// Swap any asset on the contract to uusd. Meant for received protocol rewards
    /// as a middle step to be converted to Mars.
    SwapAssetToUusd {
        offer_asset_info: AssetInfo,
        amount: Option<Uint128>,
    },
    /// Swap uusd on the contract to Mars. Ment for received protocol rewards in order
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

    /// Unstake Mars and burn xMars
    Unstake {
        /// Address to receive the Mars tokens. Set to sender if not specified
        recipient: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Get contract config
    Config {},
    /// Get latest cooldown for given user. Even though a cooldown may be returned,
    /// it may have expired.
    Cooldown { user_address: String },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: String,
    pub address_provider_address: String,
    pub astroport_factory_address: String,
    pub astroport_max_spread: Decimal,
    pub cooldown_duration: u64,
    pub unstake_window: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CooldownResponse {
    /// Timestamp where the cooldown was activated
    pub timestamp: u64,
    /// Amount that the user is allowed to unstake during the unstake window
    pub amount: Uint128,
}
