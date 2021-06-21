use cosmwasm_std::{CosmosMsg, HumanAddr, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub owner: HumanAddr,
    pub staking_address: HumanAddr,
    pub mars_token_address: HumanAddr,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Set emission per second for an asset
    /// ma_token_address
    SetAssetIncentive {
        /// Ma token address associated with the incentives
        ma_token_address: HumanAddr,
        /// How many Mars will be assigned per second to be distributed among all liquidity
        /// providers
        emission_per_second: Uint128,
    },

    /// Handle balance change updating user and asset rewards.
    /// Sent from an external contract, triggered on user balance changes
    /// Will return an empty response if no incentive is applied for the asset
    BalanceChange {
        user_address: HumanAddr,
        /// user balance up to the instant before the change
        user_balance_before: Uint128,
        /// total supply up to the instant before the change
        total_supply_before: Uint128,
    },

    /// Claim rewards. Mars rewards accrued by the user will be staked into xMars before
    /// being sent.
    ClaimRewards {},

    /// Update contract config (only callable by owner)
    UpdateConfig {
        owner: Option<HumanAddr>,
        mars_token_address: Option<HumanAddr>,
        staking_address: Option<HumanAddr>,
    },

    /// Execute Cosmos msg. Only callable by owner
    ExecuteCosmosMsg(CosmosMsg),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
}

/// Query response with config values
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: HumanAddr,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
