use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum MarsError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Incorrect number of addresses, expected {expected:?}, got {actual:?}")]
    AddressesQueryWrongNumber { expected: u32, actual: u32 },
}
