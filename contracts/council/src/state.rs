use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Binary, Addr, Decimal, StdResult, Uint128};
use cw_storage_plus::{Item, Map, U64Key};

use mars::helpers::all_conditions_valid;

pub const CONFIG: Item<Config> = Item::new("config");
pub const GLOBAL_STATE: Item<GlobalState> = Item::new("global_state");
pub const PROPOSALS: Map<U64Key, Proposal> =  Map::new("proposals");
pub const PROPOSAL_VOTES: Map<(U64Key, &Addr), ProposalVote> = Map::new("proposal_votes");

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

/// Global state
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GlobalState {
    /// Number of proposals
    pub proposal_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Proposal {
    pub submitter_address: Addr,
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
    pub target_contract_address: Addr,
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
