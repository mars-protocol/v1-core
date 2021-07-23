use cosmwasm_std::StdError;
use mars::error::MarsError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Stake amount must be greater than 0")]
    StakeAmountZero {},

    #[error("Unstake amount must be greater than 0")]
    UnstakeAmountZero {},

    #[error("Address must have a valid cooldown to unstake")]
    UnstakeNoCooldown {},

    #[error("Cooldown has expired")]
    UnstakeCooldownExpired {},

    #[error("Cooldown has not finished")]
    UnstakeCooldownNotFinished {},

    #[error("Unstake amount must not be greater than cooldown amount")]
    UnstakeAmountTooLarge {},

    #[error("Cannot swap Mars")]
    MarsCannotSwap {},

    #[error("xMars token address not found")]
    XMarsAddressNotFound {},

    #[error("Mars token address not found")]
    MarsAddressNotFound {},
}
