use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::HumanAddr;
use cw20::Cw20ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub ma_token_code_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    /// Implementation cw20 receive msg
    Receive(Cw20ReceiveMsg),

    /// Initialize an asset on the money market
    InitAsset {
        /// Asset related info
        asset_info: InitAssetInfo,
        /// Asset parameters
        asset_params: InitAssetParams,
    },
    /// Callback sent from maToken contract after instantiated
    InitAssetTokenCallback {
        /// Either the denom for a terra native asset or address for a cw20 token
        reference: Vec<u8>,
    },
    /// Deposit Terra native coins
    DepositNative {
        /// Denom used in Terra (e.g: uluna, uusd)
        denom: String,
    },
    /// Borrow Terra native coins
    BorrowNative {
        /// Denom used in Terra (e.g: uluna, uusd)
        denom: String,
        amount: Uint256,
    },
    /// Repay Terra native coins loan
    RepayNative {
        /// Denom used in Terra (e.g: uluna, uusd)
        denom: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Redeem the sent tokens for
    Redeem {
        /// Either the symbol for a terra native asset or address for a cw20 token
        // TODO: Maybe it's not necessary to send this but it makes things more
        // straightforward for now. We can revisit when we figure how are we
        // going to index the state
        id: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    Reserve { denom: String },
    ReservesList {},
    Debt { address: HumanAddr },
}

// We define a custom struct for each query response
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ConfigResponse {
    pub ma_token_code_id: u64,
    pub reserve_count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ReserveResponse {
    pub ma_token_address: HumanAddr,
    pub borrow_index: Decimal256,
    pub liquidity_index: Decimal256,
    pub borrow_rate: Decimal256,
    pub liquidity_rate: Decimal256,
    pub borrow_slope: Decimal256,
    pub loan_to_value: Decimal256,
    pub interests_last_updated: u64,
    pub debt_total_scaled: Uint256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ReservesListResponse {
    pub reserves_list: Vec<ReserveInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct ReserveInfo {
    pub denom: String,
    pub ma_token_address: HumanAddr,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DebtResponse {
    pub debts: Vec<DebtInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct DebtInfo {
    pub denom: String,
    pub amount: Uint256,
}

/// We currently take no arguments for migrations
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MigrateMsg {}
/// We currently take no arguments for migrations

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum InitAssetInfo {
    Cw20 { contract_addr: HumanAddr },
    Native { denom: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitAssetParams {
    /// Borrow slope to calculate borrow rate
    pub borrow_slope: Decimal256,
    /// Max percentage of collateral that can be borrowed
    pub loan_to_value: Decimal256,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Cw20,
    Native,
}

impl Default for AssetType {
    fn default() -> Self {
        AssetType::Native
    }
}
