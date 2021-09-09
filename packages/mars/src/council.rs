pub mod msg {
    use cosmwasm_std::CosmosMsg;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// Execute call that will be executed by the DAO if the proposal succeeds
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ProposalExecuteCall {
        pub execution_order: u64,
        pub msg: CosmosMsg,
    }
}
