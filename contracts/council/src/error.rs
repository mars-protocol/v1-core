use cosmwasm_std::StdError;
use mars::error::MarsError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("Invalid Proposal: {error:?}")]
    InvalidProposal { error: String },

    #[error("Proposal is not active")]
    ProposalNotActive{},

    #[error("Cannot vote on an expired proposal")]
    VoteProposalExpired{},

    #[error("User has already voted on this proposal")]
    VoteUserAlreadyVoted{},

    #[error("User has no voting power at block: {block:?}")]
    VoteNoVotingPower{ block: u64 },

    #[error("Voting period has not ended")]
    EndVotingPeriodNotEnded{},

    #[error("Proposal must end it's delay period in order to be executed")]
    ExecuteProposalDelayNotEnded{},
}

impl ContractError {
    pub fn invalid_proposal<S: Into<String>>(error: S) -> ContractError {
        ContractError::InvalidProposal {
            error: error.into(),
        }
    }
}
