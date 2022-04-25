use cosmwasm_std::{OverflowError, StdError, Uint128};
use mars_core::error::MarsError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Proxy Reward Token not set")]
    ProxyRewardNotSet {},

    #[error("Incorrect CW20 hook message variant!")]
    IncorrectCw20HookMessageVariant {},

    #[error("{0}")]
    Mars(#[from] MarsError),

    #[error("{0}")]
    Overflow(#[from] OverflowError),
}
