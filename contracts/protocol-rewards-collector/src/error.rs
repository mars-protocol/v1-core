use cosmwasm_std::{OverflowError, StdError, Uint128};
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

    #[error("Asset is not enabled for distribution: {label:?}")]
    AssetNotEnabled { label: String },

    #[error("Amount to distribute {amount} is larger than available balance {balance}")]
    AmountToDistributeTooLarge { amount: Uint128, balance: Uint128 },

    #[error("Invalid fee share amounts. Sum of insurance and treasury fee shares exceeds one")]
    InvalidFeeShareAmounts {},
}
