use cosmwasm_std::{OverflowError, StdError};
use thiserror::Error;

use mars_core::error::MarsError;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("Only Mars token can be deposited")]
    InvalidTokenDeposit {},

    #[error("Data already exists for account: {account}")]
    DataAlreadyExists { account: String },

    #[error("Cannot find attribute: {key}")]
    ReplyParseFailed { key: String },
}
