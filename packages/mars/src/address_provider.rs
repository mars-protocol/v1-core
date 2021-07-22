pub mod msg {
    use cosmwasm_std::Addr;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Only owner can be set on initialization (the EOA doing all the deployments)
    /// as all other contracts are supposed to be initialized after this one with its address
    /// passed as a param.
    /// After initializing all contracts. An update config call should be done setting council as the
    /// owner and submiting all the contract addresses
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InstantiateMsg {
        pub owner: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Update address provider config
        UpdateConfig { config: ConfigParams },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, JsonSchema)]
    pub struct ConfigParams {
        pub owner: Option<String>,
        pub council_address: Option<String>,
        pub incentives_address: Option<String>,
        pub insurance_fund_address: Option<String>,
        pub mars_token_address: Option<String>,
        pub red_bank_address: Option<String>,
        pub staking_address: Option<String>,
        pub treasury_address: Option<String>,
        pub xmars_token_address: Option<String>,
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
        pub owner: Addr,
        pub council_address: Addr,
        pub incentives_address: Addr,
        pub insurance_fund_address: Addr,
        pub mars_token_address: Addr,
        pub red_bank_address: Addr,
        pub staking_address: Addr,
        pub treasury_address: Addr,
        pub xmars_token_address: Addr,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}

pub mod helpers {
    use super::msg::{MarsContract, QueryMsg};
    use cosmwasm_std::{to_binary, Addr, QuerierWrapper, QueryRequest, StdResult, WasmQuery};
    use crate::error::{MarsError};

    pub fn query_address(
        querier: &QuerierWrapper,
        address_provider_address: Addr,
        contract: MarsContract,
    ) -> StdResult<Addr> {
        let query: Addr = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: address_provider_address.to_string(),
            msg: to_binary(&QueryMsg::Address { contract })?,
        }))?;

        Ok(query)
    }

    pub fn query_addresses(
        querier: &QuerierWrapper,
        address_provider_address: Addr,
        contracts: Vec<MarsContract>,
    ) -> Result<Vec<Addr>, MarsError> {
        let expected_len = contracts.len();

        let query: Vec<Addr> = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: address_provider_address.to_string(),
            msg: to_binary(&QueryMsg::Addresses { contracts })?,
        }))?;

        if query.len() != expected_len {
            return Err(MarsError::AddressesQueryWrongNumber{
                expected: expected_len as u32,
                actual: query.len() as u32,
            });
        }

        Ok(query)
    }
}
