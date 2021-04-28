use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use cosmwasm_std::{Binary, CanonicalAddr, Storage, Uint128};
use cosmwasm_storage::{
    bucket, bucket_read, singleton, singleton_read, Bucket, ReadonlyBucket, ReadonlySingleton,
    Singleton,
};

// keys (for singleton)
pub static CONFIG_KEY: &[u8] = b"config";
pub static BASECAMP_KEY: &[u8] = b"basecamp";

// namespaces (for buckets)
pub static COOLDOWNS_NAMESPACE: &[u8] = b"cooldowns";
pub static POLLS_NAMESPACE: &[u8] = b"polls";
pub static POLL_VOTES_NAMESPACE: &[u8] = b"poll_votes";

/// Basecamp global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: CanonicalAddr,
    /// Mars token address
    pub mars_token_address: CanonicalAddr,
    /// xMars token address
    pub xmars_token_address: CanonicalAddr,
    /// Cooldown duration in seconds
    pub cooldown_duration: u64,
    /// Time in seconds after the cooldown ends during which the unstaking of
    /// the associated amount is allowed
    pub unstake_window: u64,
    /// Blocks during which a proposal is active since being submitted
    pub voting_period: u64,
    /// Blocks that need to pass since a proposal succeeds in order for it to be available to be
    /// executed
    pub effective_delay: u64,
    /// Blocks after the effective_delay during which a successful proposal can be activated before it expires
    pub expiration_period: u64,
    /// Number of Mars needed to make a proposal. Will be returned if successful. Will be
    /// distributed between stakers if proposal is not executed.
    pub proposal_deposit: Uint128,
}

pub fn config_state<S: Storage>(storage: &mut S) -> Singleton<S, Config> {
    singleton(storage, CONFIG_KEY)
}

pub fn config_state_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, Config> {
    singleton_read(storage, CONFIG_KEY)
}

/// Basecamp global state
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Basecamp {
    /// Number of polls
    total_polls: u64,
}

pub fn basecamp_state<S: Storage>(storage: &mut S) -> Singleton<S, Basecamp> {
    singleton(storage, BASECAMP_KEY)
}

pub fn basecamp_state_read<S: Storage>(storage: &S) -> ReadonlySingleton<S, Basecamp> {
    singleton_read(storage, BASECAMP_KEY)
}

/// Unstaking cooldown data
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Cooldown {
    /// Timestamp where the cooldown was activated
    pub timestamp: u64,
    /// Amount that the user is allowed to unstake during the unstake window
    pub amount: Uint128,
}

pub fn cooldowns_state<S: Storage>(storage: &mut S) -> Bucket<S, Cooldown> {
    bucket(COOLDOWNS_NAMESPACE, storage)
}

pub fn cooldowns_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Cooldown> {
    bucket_read(COOLDOWNS_NAMESPACE, storage)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Poll {
    pub creator: CanonicalAddr,
    pub status: PollStatus,
    pub for_votes: Uint128,
    pub against_votes: Uint128,
    pub end_height: u64,
    pub title: String,
    pub description: String,
    pub link: Option<String>,
    pub execute_data: Option<Vec<ExecuteData>>,
    pub deposit_amount: Uint128,

    /// Amount used to compute voting quorum and threshold
    pub total_voting_power: Option<Uint128>,
}

pub fn polls_state<S: Storage>(storage: &mut S) -> Bucket<S, Poll> {
    bucket(POLLS_NAMESPACE, storage)
}

pub fn polls_state_read<S: Storage>(storage: &S) -> ReadonlyBucket<S, Poll> {
    bucket_read(POLLS_NAMESPACE, storage)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PollStatus {
    Active,
    Passed,
    Rejected,
    Executed,
    Expired,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ExecuteData {
    pub order: u64,
    pub contract: CanonicalAddr,
    pub msg: Binary,
}

impl Eq for ExecuteData {}

impl Ord for ExecuteData {
    fn cmp(&self, other: &Self) -> Ordering {
        self.order.cmp(&other.order)
    }
}

impl PartialOrd for ExecuteData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ExecuteData {
    fn eq(&self, other: &Self) -> bool {
        self.order == other.order
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PollVote {
    option: VoteOption,
    power: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoteOption {
    For,
    Against,
}

pub fn poll_votes_state<S: Storage>(storage: &mut S, poll_id: u64) -> Bucket<S, PollVote> {
    Bucket::multilevel(&[POLL_VOTES_NAMESPACE, &poll_id.to_be_bytes()], storage)
}

pub fn poll_votes_state_read<S: Storage>(storage: &S, poll_id: u64) -> ReadonlyBucket<S, PollVote> {
    ReadonlyBucket::multilevel(&[POLL_VOTES_NAMESPACE, &poll_id.to_be_bytes()], storage)
}
