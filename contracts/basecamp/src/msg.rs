use crate::state::{ProposalExecuteCall, ProposalStatus, ProposalVoteOption};
use cosmwasm_std::{Binary, Decimal, HumanAddr, Uint128};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub cw20_code_id: u64,
    pub config: CreateOrUpdateConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct CreateOrUpdateConfig {
    pub xmars_token_address: Option<HumanAddr>,
    pub staking_contract_address: Option<HumanAddr>,
    pub insurance_fund_contract_address: Option<HumanAddr>,

    pub proposal_voting_period: Option<u64>,
    pub proposal_effective_delay: Option<u64>,
    pub proposal_expiration_period: Option<u64>,
    pub proposal_required_deposit: Option<Uint128>,
    pub proposal_required_quorum: Option<Decimal>,
    pub proposal_required_threshold: Option<Decimal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Implementation cw20 receive msg
    Receive(Cw20ReceiveMsg),

    /// Sets the xMars and Staking contract addresses
    SetContractAddresses {
        xmars_token_address: HumanAddr,
        staking_contract_address: HumanAddr,
        insurance_fund_contract_address: HumanAddr,
    },

    /// Callback to initialize Mars token
    InitTokenCallback {},

    /// Mint Mars tokens to receiver (Temp action for Testing)
    MintMars {
        recipient: HumanAddr,
        amount: Uint128,
    },

    /// Vote for a proposal
    CastVote {
        proposal_id: u64,
        vote: ProposalVoteOption,
    },

    /// End proposal after voting period has passed
    EndProposal { proposal_id: u64 },
    /// Execute a successful proposal
    ExecuteProposal { proposal_id: u64 },

    /// Update basecamp config
    UpdateConfig {
        mars_token_address: Option<HumanAddr>,
        config: CreateOrUpdateConfig,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
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
    Proposals {
        start: Option<u64>,
        limit: Option<u32>,
    },
    Proposal {
        proposal_id: u64,
    },
    LatestExecutedProposal {},
    ProposalVotes {
        proposal_id: u64,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub mars_token_address: HumanAddr,
    pub xmars_token_address: HumanAddr,
    pub staking_contract_address: HumanAddr,
    pub insurance_fund_contract_address: HumanAddr,

    pub proposal_required_deposit: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalsListResponse {
    pub proposal_count: u64,
    pub proposal_list: Vec<ProposalInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalInfo {
    pub proposal_id: u64,
    pub status: ProposalStatus,
    pub for_votes: Uint128,
    pub against_votes: Uint128,
    pub start_height: u64,
    pub end_height: u64,
    pub title: String,
    pub description: String,
    pub link: Option<String>,
    pub execute_calls: Option<Vec<ProposalExecuteCall>>,
    pub deposit_amount: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalVotesResponse {
    pub proposal_id: u64,
    pub votes: Vec<ProposalVoteResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalVoteResponse {
    pub voter_address: HumanAddr,
    pub option: ProposalVoteOption,
    pub power: Uint128,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
