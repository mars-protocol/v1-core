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
    /// Implementaton cw20 receive msg
    Receive(Cw20ReceiveMsg),

    /// Initialize an asset on the money market
    InitAsset {
        /// Symbol used in Terra (e.g: luna, usd)
        symbol: String,
    },
    /// Callback sent from maToken contract after instantiated
    InitAssetTokenCallback {
        /// Either the symbol for a terra native asset or address for a cw20 token
        id: String,
    },
    /// Deposit Terra native coins
    DepositNative {
        /// Symbol used in Terra (e.g: luna, usd)
        symbol: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMsg {
    /// Redeem the sent tokens for
    Redeem {
        /// Either the symbol for a terra native asset or address for a cw20 token
        // TODO: Maybe it's not neccesary to send this but it makes things more
        // straightforward for now. We can revisit when we figure how are we
        // going to index the state
        id: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    Config {},
    Reserve { symbol: String },
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
}
