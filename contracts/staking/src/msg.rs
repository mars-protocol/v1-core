use cosmwasm_std::{CosmosMsg, HumanAddr, Uint128};

use cw20::Cw20ReceiveMsg;
use mars::liquidity_pool::msg::Asset;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub cw20_code_id: u64,
    pub config: CreateOrUpdateConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CreateOrUpdateConfig {
    pub mars_token_address: Option<HumanAddr>,
    pub terraswap_factory_address: Option<HumanAddr>,
    pub terraswap_pair_address: Option<HumanAddr>,
    pub cooldown_duration: Option<u64>,
    pub unstake_window: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Update staking config
    UpdateConfig {
        owner: Option<HumanAddr>,
        xmars_token_address: Option<HumanAddr>,
        config: CreateOrUpdateConfig,
    },
    /// Implementation cw20 receive msg
    Receive(Cw20ReceiveMsg),
    /// Initialize or refresh cooldown
    Cooldown {},
    /// Callback to initialize xMars token
    InitTokenCallback {},
    /// Execute Cosmos msg
    ExecuteCosmosMsg(CosmosMsg),
    /// Swap contract reserves of specified asset into Mars
    Swap { asset: Asset },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Stake Mars and get minted xMars in return
    Stake,
    /// Unstake Mars and burn xMars
    Unstake,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    Cooldown { sender_address: HumanAddr },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: HumanAddr,
    pub mars_token_address: HumanAddr,
    pub xmars_token_address: HumanAddr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct CooldownResponse {
    /// Timestamp where the cooldown was activated
    pub timestamp: u64,
    /// Amount that the user is allowed to unstake during the unstake window
    pub amount: Uint128,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
