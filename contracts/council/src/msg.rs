use crate::state::{ProposalExecuteCall, ProposalStatus, ProposalVoteOption};
use cosmwasm_std::{Binary, Decimal, HumanAddr, Uint128};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub config: CreateOrUpdateConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct CreateOrUpdateConfig {
    pub address_provider_address: Option<HumanAddr>,

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

    /// Vote for a proposal
    CastVote {
        proposal_id: u64,
        vote: ProposalVoteOption,
    },

    /// End proposal after voting period has passed
    EndProposal { proposal_id: u64 },
    /// Execute a successful proposal
    ExecuteProposal { proposal_id: u64 },

    /// Update config
    UpdateConfig { config: CreateOrUpdateConfig },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Submit a proposal to be voted
    /// Requires a Mars deposit equal or greater than the proposal_required_deposit
    SubmitProposal {
        title: String,
        description: String,
        link: Option<String>,
        execute_calls: Option<Vec<MsgExecuteCall>>,
    },
}

/// Execute call that will be done by the DAO if the proposal succeeds.
/// Contains a the msg and contract_addr attributes of a WasmMsg::Execute call
/// As this is part of the proposal creation call, the contract human address is sent
/// (vs the canonical address when persisted)
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
        start: Option<u64>,
        limit: Option<u32>,
        sort: Option<ProposalVotesSort>,
        filter: Option<ProposalVotesFilter>,
    },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub address_provider_address: HumanAddr,

    pub proposal_voting_period: u64,
    pub proposal_effective_delay: u64,
    pub proposal_expiration_period: u64,
    pub proposal_required_deposit: Uint128,
    pub proposal_required_quorum: Decimal,
    pub proposal_required_threshold: Decimal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalsListResponse {
    pub proposal_count: u64,
    pub proposal_list: Vec<ProposalInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalInfo {
    pub proposal_id: u64,
    pub submitter_address: HumanAddr,
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
#[serde(rename_all = "snake_case")]
pub enum ProposalVotesSort {
    Ascending,
    Descending,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
pub struct ProposalVotesFilter {
    pub voter_address: Option<HumanAddr>,
    pub vote_option: Option<ProposalVoteOption>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalVotesResponse {
    pub proposal_id: u64,
    pub votes: Vec<ProposalVoteResponse>,
    pub proposal_votes_count: u64,
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
