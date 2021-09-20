use cosmwasm_std::{OverflowError, StdError};
use mars::error::MarsError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),

    #[error("Stake amount must be greater than 0")]
    StakeAmountZero {},

    #[error("Unstake amount must be greater than 0")]
    UnstakeAmountZero {},

    #[error("Cooldown has not ended")]
    ClaimCooldownNotEnded {},

    #[error("Cannot swap Mars")]
    MarsCannotSwap {},
}
