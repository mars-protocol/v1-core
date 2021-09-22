use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, Uint128};

/// Protocol configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,

    /// Address provider address
    pub address_provider_address: Addr,

    /// Astroport factory contract address
    pub astroport_factory_address: Addr,
    /// Astroport max spread
    pub astroport_max_spread: Decimal,

    /// Cooldown duration in seconds
    pub cooldown_duration: u64,
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
    /// Block when the claim was created (Used to apply slash events when claiming)
    pub created_at_block: u64,
    /// Timestamp (in seconds) after which the claim is unlocked
    pub cooldown_end_timestamp: u64,
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
