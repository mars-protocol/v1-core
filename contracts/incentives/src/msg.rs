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
    /// Set emission per second for a list of assets
    /// set_asset_incentives: list of asset incentives to initilize/update
    SetAssetIncentives {
        set_asset_incentives: Vec<SetAssetIncentive>,
    },

    /// Handle a balance change. Sent on an external contract,
    /// triggered on user balance changes
    /// user_address: address of the user
    /// user_balance: user balance up to the instant before the change
    /// total_supply: total supply up to the instant before the change
    HandleBalanceChange {
        user_address: HumanAddr,
        user_balance_before: Uint128,
        total_supply_before: Uint128,
    },

    /// Claim rewards. Mars rewards accrued by the user will be staked into xMars before
    /// being sent.
    ClaimRewards,

    /// Execute Cosmos msg. Only callable by owner
    ExecuteCosmosMsg(CosmosMsg),
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct SetAssetIncentive {
    /// Ma token address associated with the incentives
    pub ma_token_address: HumanAddr,
    /// How many Mars will be assigned per second to be distributed among all liquidity
    /// providers
    pub emission_per_second: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub owner: HumanAddr,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
