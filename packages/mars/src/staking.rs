pub mod msg {
    use cosmwasm_std::{CosmosMsg, Decimal, HumanAddr, Uint128};

    use cw20::Cw20ReceiveMsg;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use terraswap::asset::AssetInfo;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitMsg {
        pub config: CreateOrUpdateConfig,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CreateOrUpdateConfig {
        pub owner: Option<HumanAddr>,
        pub address_provider_address: Option<HumanAddr>,
        pub terraswap_factory_address: Option<HumanAddr>,
        pub terraswap_max_spread: Option<Decimal>,
        pub cooldown_duration: Option<u64>,
        pub unstake_window: Option<u64>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HandleMsg {
        /// Update staking config
        UpdateConfig { config: CreateOrUpdateConfig },
        /// Implementation cw20 receive msg
        Receive(Cw20ReceiveMsg),
        /// Initialize or refresh cooldown
        Cooldown {},
        /// Execute Cosmos msg
        ExecuteCosmosMsg(CosmosMsg),
        /// Swap any asset on the contract to uusd
        SwapAssetToUusd {
            offer_asset_info: AssetInfo,
            amount: Option<Uint128>,
        },
        /// Swap uusd on the contract to Mars
        SwapUusdToMars { amount: Option<Uint128> },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReceiveMsg {
        /// Stake Mars and mint xMars in return
        /// - recipient: address to receive the xMars tokens. Set to sender if not specified
        Stake { recipient: Option<HumanAddr> },

        /// Unstake Mars and burn xMars
        /// - recipient: address to receive the Mars tokens. Set to sender if not specified
        Unstake { recipient: Option<HumanAddr> },
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
        pub address_provider_address: HumanAddr,
        pub terraswap_max_spread: Decimal,
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

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}
