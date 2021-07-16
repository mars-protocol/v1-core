use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Binary, Addr, Decimal, StdResult, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};
use mars::helpers::all_conditions_valid;

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";
pub static COUNCIL_KEY: &[u8] = b"council";

// namespaces (for buckets)
pub static PROPOSALS_NAMESPACE: &[u8] = b"proposals";
pub static PROPOSAL_VOTES_NAMESPACE: &[u8] = b"proposal_votes";

/// Council global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Address provider returns addresses for all protocol contracts
    pub address_provider_address: Addr,
    /// Blocks during which a proposal is active since being submitted
    pub proposal_voting_period: u64,
    /// Blocks that need to pass since a proposal succeeds in order for it to be available to be
    /// executed
    pub proposal_effective_delay: u64,
    /// Blocks after the effective_delay during which a successful proposal can be activated before it expires
    pub proposal_expiration_period: u64,
    /// Number of Mars needed to make a proposal. Will be returned if successful. Will be
    /// distributed between stakers if proposal is not executed.
    pub proposal_required_deposit: Uint128,
    /// % of total voting power required to participate in the proposal in order to consider it successfull
    pub proposal_required_quorum: Decimal,
    /// % of for votes required in order to consider the proposal successful
    pub proposal_required_threshold: Decimal,
}

impl Config {
    pub fn validate(&self) -> StdResult<()> {
        let conditions_and_names = vec![
            (
                Self::less_or_equal_one(&self.proposal_required_quorum),
                "proposal_required_quorum",
            ),
            (
                Self::less_or_equal_one(&self.proposal_required_threshold),
                "proposal_required_threshold",
            ),
        ];
        all_conditions_valid(conditions_and_names)
    }

    fn less_or_equal_one(value: &Decimal) -> bool {
        value.le(&Decimal::one())
    }
}

pub fn config(storage: &mut dyn Storage) -> Singleton<Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_read<S: Storage>(storage: &dyn Storage) -> ReadonlySingleton<Config> {
    singleton_read(storage, CONFIG_KEY)
}

/// Council global state
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Council {
    /// Number of proposals
    pub proposal_count: u64,
}

pub fn council(storage: &mut S) -> Singleton<S, Council> {
    singleton(storage, COUNCIL_KEY)
}

pub fn council_read(storage: &S) -> ReadonlySingleton<S, Council> {
    singleton_read(storage, COUNCIL_KEY)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Proposal {
    pub submitter_canonical_address: CanonicalAddr,
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

pub fn proposals(storage: &mut dyn S) -> Bucket<Proposal> {
    bucket(storage, PROPOSALS_NAMESPACE)
}

pub fn proposals_read(storage: &dyn S) -> ReadonlyBucket<Proposal> {
    bucket_read(storage, PROPOSALS_NAMESPACE)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Active,
    Passed,
    Rejected,
    Executed,
}

/// Execute call that will be done by the DAO if the proposal succeeds. As this is persisted,
/// the contract canonical address is stored (vs the human address when the proposal submit message is
/// sent)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalExecuteCall {
    pub execution_order: u64,
    pub target_contract_canonical_address: CanonicalAddr,
    pub msg: Binary,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ProposalVote {
    pub option: ProposalVoteOption,
    pub power: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalVoteOption {
    For,
    Against,
}

impl std::fmt::Display for ProposalVoteOption {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let display_str = match self {
            ProposalVoteOption::For => "for",
            ProposalVoteOption::Against => "against",
        };
        write!(f, "{}", display_str)
    }
}

pub fn proposal_votes(storage: &mut dyn S, proposal_id: u64) -> Bucket<ProposalVote> {
    Bucket::multilevel(
        storage,
        &[PROPOSAL_VOTES_NAMESPACE, &proposal_id.to_be_bytes()]
    )
}

pub fn proposal_votes_read<(
    storage: &dyn S,
    proposal_id: u64,
) -> ReadonlyBucket<ProposalVote> {
    ReadonlyBucket::multilevel(
        storage,
        &[PROPOSAL_VOTES_NAMESPACE, &proposal_id.to_be_bytes()]
    )
}
