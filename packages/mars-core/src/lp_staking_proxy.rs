use cosmwasm_std::{Addr, Decimal, Uint128};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub redbank_addr: Addr,
    pub astro_generator_addr: Addr,
    pub token_addr: Addr,
    pub ma_token_addr: Option<Addr>,
    pub pool_addr: Addr,
    pub astro_token_addr: Addr,
    pub proxy_token_reward_addr: Option<Addr>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    DepositWithProxy {},
}
/// ## Description
/// This structure describes the execute messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Receives a message of type [`Cw20ReceiveMsg`]
    Receive(Cw20ReceiveMsg),
    /// Admin function to Update fees charged on rewards
    UpdateFeeConfig {
        astro_treasury_fee: Decimal,
        proxy_token_treasury_fee: Decimal,
    },
    /// Withdrawal pending rewards
    UpdateRewards {},
    /// Sends ASTRO rewards to the recipient
    SendAstroRewards { account: Addr, amount: Uint128 },
    /// Sends proxy token rewards to the recipient
    SendProxyRewards { account: Addr, amount: Uint128 },
    /// Withdrawal the rewards
    Withdraw {
        /// the recipient for withdrawal
        account: Addr,
        /// the amount of withdraw
        amount: Uint128,
    },
    /// Withdrawal the rewards
    EmergencyWithdraw {
        /// the recipient for withdrawal
        account: Addr,
        /// the amount of withdraw
        amount: Uint128,
    },
    /// the callback of type [`CallbackMsg`]
    Callback(CallbackMsg),
}

/// ## Description
/// This structure describes the callback messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallbackMsg {
    TransferTokensAfterWithdraw {
        /// the recipient
        account: Addr,
        /// the previous lp balance to calculate withdrawn amount
        prev_balance: Uint128,
    },
}

pub type ConfigResponse = InstantiateMsg;

/// ## Description
/// This structure describes the query messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Returns the contract's configuration struct
    Config {},
}

/// ## Description
/// This structure describes a migration message.
/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
