use cosmwasm_std::Uint128;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// T = String (unchecked) or Addr (checked)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config<T> {
    /// Account who can create new allocations
    pub owner: T,
    /// Account to receive the refund of unvested tokens if a user terminates allocation
    pub refund_recipient: T,
    /// Address of MARS token
    pub mars_token: T,
    /// Address of xMARS token
    pub xmars_token: T,
    /// Address of Mars staking contract
    pub mars_staking: T,
    /// By default, unlocking starts at Mars launch, with a cliff of 6 months and a duration of 36 months.
    /// If not specified, all allocations use this default schedule
    pub default_unlock_schedule: Schedule,
}

// Parameters describing a typical vesting/unlocking schedule
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Schedule {
    /// Timestamp of when vesting/unlocking is to be started (in seconds)
    pub start_time: u64,
    /// Number of seconds starting UST during which no token will be vested/unlocked
    pub cliff: u64,
    /// Number of seconds taken since UST for tokens to be fully vested/unlocked
    pub duration: u64,
}

// Record of a staking transaction: the amount of MARS put in, and the amount of xMARS minted.
// We need to keep a record of all staking transactions initiated by a user because this data is
// necessary in calculating how many xMARS can be withdrawn.
//
// Examples:
//
// 1) At 1 xMARS = 1.2 MARS, a user stakes 12 MARS to get 10 xMARS
// After some time, 1 xMARS should be worth more than 1.2 MARS. This user has 6 MARS unlocked
// and wants to withdraw. He should have 10 * (6 / 12) = 5 xMARS withdrawable, regardless of
// the current xMARS/MARS ratio.
//
// 2) At 1 xMARS = 1.2 MARS, a user stakes 12 MARS to get 10 xMARS
// Then, at 1 xMARS = 1.5 MARS, the user stakes another 12 MARS to get 8 xMARS
// Later, The user has 18 MARS unlocked and wishes to withdraw. He will get:
// - 12 MARS in the form of 10 xMARS at 1.2 MARS per xMARS
// - 6 MARS in the form of 8 * (6 / 12) = 4 xMARS at 1.5 MARS per xMARS
// Result, the user:
// - gets 14 xMARS, which is equivalent to 18 out of the 24 MARS that he put in
// - has 4 xMARS remaining, which can later be withdrawn at 1.5 MARS per xMARS ratio
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Stake {
    // Amount of MARS token staked in a staking transactions
    pub mars_staked: Uint128,
    // Amount of xMARS token minted in a staking transaction
    pub xmars_minted: Uint128,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AllocationParams {
    /// Total amount of MARS token allocated to this account
    pub amount: Uint128,
    /// Parameters controlling the vesting process
    pub vest_schedule: Schedule,
    /// Parameters controlling the unlocking process
    /// If not provided, use `config.default_unlock_schedule`
    pub unlock_schedule: Option<Schedule>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AllocationStatus {
    /// Amount of MARS already withdrawn in the form of MARS token
    pub mars_withdrawn_as_mars: Uint128,
    /// Amount of MARS already withdrawn in the form of xMARS token
    pub mars_withdrawn_as_xmars: Uint128,
    /// The amount of Mars staked
    pub mars_staked: Uint128,
    /// Stakes owned by the user: amount of xMARS, and their equivalent MARS amount
    pub stakes: Vec<Stake>,
}

impl AllocationStatus {
    pub const fn new() -> Self {
        Self {
            mars_withdrawn_as_mars: Uint128::zero(),
            mars_withdrawn_as_xmars: Uint128::zero(),
            mars_staked: Uint128::zero(),
            stakes: vec![],
        }
    }
}

pub mod msg {
    use cosmwasm_std::{Addr, Uint128};
    use cw20::Cw20ReceiveMsg;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::{AllocationParams, AllocationStatus, Config};

    pub type InstantiateMsg = Config<String>;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Implementation of cw20 receive msg
        Receive(Cw20ReceiveMsg),
        /// Stake all vested but not-yet-withdrawn MARS (locked and unlocked), receive xMARS
        Stake {},
        /// Claim withdrawable MARS and xMARS
        Withdraw {},
        /// Give up allocation, refund all unvested tokens to `config.fallback_recipient`
        Terminate {},
        /// Update addresses of owner and fallback_recipient
        TransferOwnership {
            new_owner: String,
            new_refund_recipient: String,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReceiveMsg {
        /// Create new allocations
        CreateAllocations {
            allocations: Vec<(String, AllocationParams)>,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        // Config of this contract
        Config {},
        // Parameters and current status of an allocation
        Allocation { account: String },
        // Simulate how many MARS and xMARS will be released if a withdrawal is attempted
        SimulateWithdraw { account: String },
        // Total amount of xMARS owned by an account that's under custody by this contract
        // Used by Martian Council to determine the account's vested voting power
        VotingPowerAt { account: String, block: u64 },
    }

    pub type ConfigResponse = Config<Addr>;
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct AllocationResponse {
        pub params: AllocationParams,
        pub status: AllocationStatus,
        pub voting_power_snapshots: Vec<(u64, Uint128)>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct SimulateWithdrawResponse {
        /// Amount of MARS to receive in the form of MARS token
        pub mars_to_withdraw: Uint128,
        /// Amount of MARS to receive in the form of xMARS token
        pub mars_to_withdraw_as_xmars: Uint128,
        /// Amount of xMARS token to receive, corresponding to `mars_to_withdraw_as_xmars` MARS
        pub xmars_to_withdraw: Uint128,
    }
}
