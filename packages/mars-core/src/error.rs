use cosmwasm_std::StdError;
use thiserror::Error;

use crate::address_provider::MarsContract;

#[derive(Error, Debug, PartialEq)]
pub enum MarsError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("All params should be available during instantiation")]
    InstantiateParamsUnavailable {},

    #[error("Incorrect number of addresses, expected {expected:?}, got {actual:?}")]
    AddressesQueryWrongNumber { expected: u32, actual: u32 },

    #[error(
        "[{expected_params:?}] should be less or equal 1. Invalid params: [{invalid_params:?}]"
    )]
    ParamsNotLessOrEqualOne {
        expected_params: String,
        invalid_params: String,
    },

    #[error("One or more addresses are empty: {empty_addresses:?}")]
    EmptyAddresses { empty_addresses: Vec<MarsContract> },
}
