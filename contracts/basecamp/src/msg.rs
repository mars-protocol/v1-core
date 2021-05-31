use crate::state::ProposalVoteOption;
use cosmwasm_std::{Binary, Decimal, HumanAddr, Uint128};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub cw20_code_id: u64,
    pub cooldown_duration: u64,
    pub unstake_window: u64,

    pub proposal_voting_period: u64,
    pub proposal_effective_delay: u64,
    pub proposal_expiration_period: u64,
    pub proposal_required_deposit: Uint128,
    pub proposal_required_quorum: Decimal,
    pub proposal_required_threshold: Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Implementation cw20 receive msg
    Receive(Cw20ReceiveMsg),
    /// Callback to initialize Mars and xMars tokens
    InitTokenCallback { token_id: u8 },

    /// Mint Mars tokens to receiver (Temp action for Testing)
    MintMars {
        recipient: HumanAddr,
        amount: Uint128,
    },

    /// Initialize or refresh cooldown
    Cooldown {},

    /// Vote for a proposal
    CastVote {
        proposal_id: u64,
        vote: ProposalVoteOption,
        voting_power: Uint128,
    },

    /// End proposal after voting period has passed
    EndProposal { proposal_id: u64 },
    /// Execute a successful proposal
    ExecuteProposal { proposal_id: u64 },

    /// Update basecamp config
    UpdateConfig {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Stake Mars and get minted xMars in return
    Stake,
    /// Unstake Mars and burn xMars
    Unstake,
    // TODO: Vote while sending tokens?
    SubmitProposal {
        title: String,
        description: String,
        link: Option<String>,
        execute_calls: Option<Vec<MsgExecuteCall>>,
    },
}

/// Execute call that will be done by the DAO if the proposal succeeds. As this is part of
/// the proposal creation call, the contract human address is sent (vs the canonical address when persisted)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MsgExecuteCall {
    pub execution_order: u64,
    pub target_contract_address: HumanAddr,
    pub msg: Binary,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub mars_token_address: HumanAddr,
    pub xmars_token_address: HumanAddr,
    pub cooldown_duration: u64,
    pub proposal_required_deposit: Uint128,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
