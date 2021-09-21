use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Uint128};

/// Protocol configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,

    /// Cooldown duration in seconds
    pub cooldown_duration: u64,

    /// Address provider address
    pub address_provider_address: Addr,
    /// Terraswap factory contract address
    pub astroport_factory_address: Addr,
    /// Terraswap max spread
    pub astroport_max_spread: Decimal,
}

/// Global State
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct GlobalState {
    /// Total amount of Mars belonging to open claims
    pub total_mars_for_claimers: Uint128,
}

/// Unstaking cooldown data
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Claim {
    /// Timestamp after which the claim is unlocked
    pub cooldown_end: u64,
    /// Amount of Mars that the user is allowed to claim
    pub amount: Uint128,
}

/// Event where funds are taken from the Mars pool to cover a shortfall. The loss is covered
/// proportionally by all owners of the Mars pool (xMars holders and users with an open claim)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct SlashEvent {
    /// Percentage of total Mars slashed
    pub slash_percentage: Decimal,
}
