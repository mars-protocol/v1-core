// Type definitions of relevant Astroport contracts. We have to define them here because Astroport
// has not uploaded their package to crates.io. Once they've uploaded, we can remove this
pub mod asset {
    use cosmwasm_std::{Addr, Uint128};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct Asset {
        pub info: AssetInfo,
        pub amount: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum AssetInfo {
        Token { contract_addr: Addr },
        NativeToken { denom: String },
    }
}

pub mod pair {
    use cosmwasm_std::Uint128;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::asset::Asset;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        Pool {},
        Simulation { offer_asset: Asset },
        CumulativePrices {},
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct PoolResponse {
        pub assets: [Asset; 2],
        pub total_share: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct SimulationResponse {
        pub return_amount: Uint128,
        pub spread_amount: Uint128,
        pub commission_amount: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CumulativePricesResponse {
        pub assets: [Asset; 2],
        pub total_share: Uint128,
        pub price0_cumulative_last: Uint128,
        pub price1_cumulative_last: Uint128,
    }
}
