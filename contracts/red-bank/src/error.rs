use cosmwasm_std::{OverflowError, StdError};
use mars::error::MarsError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("Price not found for asset: {label:?}")]
    PriceNotFound { label: String },
}

impl ContractError {
    pub fn price_not_found<S: Into<String>>(label: S) -> ContractError {
        ContractError::PriceNotFound {
            label: label.into(),
        }
    }
}
