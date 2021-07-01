pub mod msg {
    use cosmwasm_std::{HumanAddr};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitMsg {
        pub owner: HumanAddr,
    }
    
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HandleMsg {
        /// Update address provider config
        UpdateConfig {
            owner: Option<HumanAddr>,
            council_address: Option<HumanAddr>,
            incentives_address: Option<HumanAddr>,
            insurance_fund_address: Option<HumanAddr>,
            mars_token_address: Option<HumanAddr>,
            red_bank_address: Option<HumanAddr>,
            staking_address: Option<HumanAddr>,
            treasury_address: Option<HumanAddr>,
            xmars_token_address: Option<HumanAddr>,
        }
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub enum MarsAddress {
        Council,
        Incentives,
        InsuranceFund,
        MarsToken,
        RedBank,
        Staking,
        Treasury,
        XMarsToken,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Get config
        Config {},
        /// Get a single address
        Address { address: MarsAddress },
        /// Get a list of addresses
        Addresses { addresses: Vec<MarsAddress> },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: HumanAddr,
        pub council_address: HumanAddr,
        pub incentives_address: HumanAddr,
        pub insurance_fund_address: HumanAddr,
        pub mars_token_address: HumanAddr,
        pub red_bank_address: HumanAddr,
        pub staking_address: HumanAddr,
        pub treasury_address: HumanAddr,
        pub xmars_token_address: HumanAddr,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}
