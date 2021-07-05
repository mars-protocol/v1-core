pub mod msg {
    use cosmwasm_std::HumanAddr;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Only owner can be set on initialization (the EOA doing all the deployments)
    /// as all other contracts are supposed to be initialized after this one with its address
    /// passed as a param.
    /// After initializing all contracts. An update config call should be done setting council as the
    /// owner and submiting all the contract addresses
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitMsg {
        pub owner: HumanAddr,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HandleMsg {
        /// Update address provider config
        UpdateConfig { config: ConfigParams },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, JsonSchema)]
    pub struct ConfigParams {
        pub owner: Option<HumanAddr>,
        pub council_address: Option<HumanAddr>,
        pub incentives_address: Option<HumanAddr>,
        pub insurance_fund_address: Option<HumanAddr>,
        pub mars_token_address: Option<HumanAddr>,
        pub red_bank_address: Option<HumanAddr>,
        pub staking_address: Option<HumanAddr>,
        pub treasury_address: Option<HumanAddr>,
        pub xmars_token_address: Option<HumanAddr>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    /// Contracts from mars protocol
    pub enum MarsContract {
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
        Address { contract: MarsContract },
        /// Get a list of addresses
        Addresses { contracts: Vec<MarsContract> },
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

pub mod helpers {
    use super::msg::{MarsContract, QueryMsg};
    use cosmwasm_std::{
        to_binary, Api, CanonicalAddr, Extern, HumanAddr, Querier, QueryRequest, StdResult,
        Storage, WasmQuery,
    };

    pub fn query_address<S: Storage, A: Api, Q: Querier>(
        deps: &Extern<S, A, Q>,
        address_provider_canonical_address: &CanonicalAddr,
        contract: MarsContract,
    ) -> StdResult<HumanAddr> {
        let address_provider_address =
            deps.api.human_address(address_provider_canonical_address)?;
        let query: HumanAddr = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: address_provider_address,
            msg: to_binary(&QueryMsg::Address { contract })?,
        }))?;

        Ok(query)
    }

    pub fn query_addresses<S: Storage, A: Api, Q: Querier>(
        deps: &Extern<S, A, Q>,
        address_provider_canonical_address: &CanonicalAddr,
        contracts: Vec<MarsContract>,
    ) -> StdResult<Vec<HumanAddr>> {
        let address_provider_address =
            deps.api.human_address(address_provider_canonical_address)?;
        let query: Vec<HumanAddr> = deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: address_provider_address,
            msg: to_binary(&QueryMsg::Addresses { contracts })?,
        }))?;

        Ok(query)
    }
}
