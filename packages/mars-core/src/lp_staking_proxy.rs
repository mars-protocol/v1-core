use cosmwasm_std::{to_binary, Addr, CosmosMsg, Decimal, Env, StdResult, Uint128, WasmMsg};
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub redbank_addr: Addr,
    pub astro_generator_addr: Addr,
    pub redbank_treasury: Addr,
    pub token_addr: Addr,
    pub ma_token_addr: Option<Addr>,
    pub pool_addr: Addr,
    pub astro_token: Addr,
    pub proxy_token: Option<Addr>,
    pub is_collateral: bool,
    pub is_stakable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cw20HookMsg {
    DepositWithProxy {
        user_addr: Addr,
        ma_token_share: Uint128,
    },
}
/// ## Description
/// This structure describes the execute messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMsg {
    /// Receives a message of type [`Cw20ReceiveMsg`]
    Receive(Cw20ReceiveMsg),
    /// Unstake LP Tokens
    Withdraw {
        user_addr: Addr,
        ma_token_share: Uint128,
        lp_token_amount: Uint128,
        claim_rewards: bool,
    },
    /// Admin function to Update fees charged on rewards
    UpdateFee {
        astro_treasury_fee: Decimal,
        proxy_treasury_fee: Decimal,
    },
    EmergencyWithdraw {},

    // UpdateRewards {},
    /// Sends ASTRO rewards to the recipient
    // SendAstroRewards { account: Addr, amount: Uint128 },
    /// Sends proxy token rewards to the recipient
    // SendProxyRewards { account: Addr, amount: Uint128 },
    /// Withdrawal the rewards
    // EmergencyWithdraw {
    //     /// the recipient for withdrawal
    //     account: Addr,
    //     /// the amount of withdraw
    //     amount: Uint128,
    // },
    /// the callback of type [`CallbackMsg`]
    Callback(CallbackMsg),
}

/// ## Description
/// This structure describes the callback messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallbackMsg {
    UpdateIndexesAndExecute {
        execute_msg: ExecuteOnCallback,
    },
    TransferLpTokensToRedBank {
        prev_lp_balance: Uint128,
    },
    TransferTokensAfterWithdraw {
        /// the recipient
        account: Addr,
        /// the previous lp balance to calculate withdrawn amount
        prev_balance: Uint128,
    },
}

// Modified from https://github.com/CosmWasm/cosmwasm-plus/blob/v0.2.3/packages/cw20/src/receiver.rs#L15
impl CallbackMsg {
    pub fn to_cosmos_msg(self, env: &Env) -> StdResult<CosmosMsg> {
        Ok(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_binary(&ExecuteMsg::Callback(self))?,
            funds: vec![],
        }))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum ExecuteOnCallback {
    /// Stakes LP tokens with AstroGenerator
    Stake {
        user_addr: Addr,
        ma_token_share: Uint128,
        lp_token_amount: Uint128,
    },
    /// Unstakes LP tokens from AstroGenerator
    Unstake {
        user_addr: Addr,
        ma_token_share: Uint128,
        lp_token_amount: Uint128,
        claim_rewards: bool,
    },
    UpdateFee {
        astro_treasury_fee: Decimal,
        proxy_treasury_fee: Decimal,
    },
    EmergencyWithdraw {},
}

pub type ConfigResponse = InstantiateMsg;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct UserInfoResponse {
    pub ma_tokens_staked: Uint128,
    pub underlying_tokens_staked: Uint128,
    pub claimable_astro: Uint128,
    pub claimable_proxy: Uint128,
    pub is_collateral: bool,
}

/// ## Description
/// This structure describes the query messages of the contract.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    /// Returns the contract's configuration struct
    Config {},
    UserInfo {
        user_address: Addr,
    },
}

/// ## Description
/// This structure describes a migration message.
/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
