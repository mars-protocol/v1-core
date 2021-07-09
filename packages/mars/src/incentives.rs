pub mod msg {
    use cosmwasm_std::{CosmosMsg, Addr, Uint128};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InstantiateMsg {
        /// Contract owner
        pub owner: String,
        /// Address provider returns addresses for all protocol contracts
        pub address_provider_address: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Set emission per second for an asset
        /// ma_token_address
        SetAssetIncentive {
            /// Ma token address associated with the incentives
            ma_token_address: String,
            /// How many Mars will be assigned per second to be distributed among all liquidity
            /// providers
            emission_per_second: Uint128,
        },

        /// Handle balance change updating user and asset rewards.
        /// Sent from an external contract, triggered on user balance changes
        /// Will return an empty response if no incentive is applied for the asset
        BalanceChange {
            user_address: String,
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
            owner: Option<String>,
            address_provider_address: Option<String>,
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
        pub owner: Addr,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}
