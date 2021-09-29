use cosmwasm_std::{to_binary, Addr, BankMsg, Coin, CosmosMsg, Deps, StdResult, Uint128, WasmMsg};
use cw20::Cw20ExecuteMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tax::deduct_tax;

/// Represents either a native asset or a cw20. Meant to be used as part of a msg
/// in a contract call and not to be used internally
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Asset {
    Cw20 { contract_addr: String },
    Native { denom: String },
}

impl Asset {
    /// Get symbol (denom/addres), reference (bytes used as key for storage) and asset type
    pub fn get_attributes(&self) -> (String, Vec<u8>, AssetType) {
        match &self {
            Asset::Native { denom } => {
                let asset_reference = denom.as_bytes().to_vec();
                (denom.to_string(), asset_reference, AssetType::Native)
            }
            Asset::Cw20 { contract_addr } => {
                let asset_reference = contract_addr.as_bytes().to_vec();
                (contract_addr.to_string(), asset_reference, AssetType::Cw20)
            }
        }
    }

    /// Return bytes used as key for storage
    pub fn get_reference(&self) -> Vec<u8> {
        match &self {
            Asset::Native { denom } => denom.as_bytes().to_vec(),
            Asset::Cw20 { contract_addr } => contract_addr.as_bytes().to_vec(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Cw20,
    Native,
}

/// If the `AssetType` is `Native`, a "tax" is charged (see [`build_send_native_asset_with_tax_deduction_msg`] for details).
/// No tax is charged on `Cw20` asset transfers.
pub fn build_send_asset_with_tax_deduction_msg(
    deps: Deps,
    sender_address: Addr,
    recipient_address: Addr,
    asset: Asset,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    match asset {
        Asset::Native { denom } => Ok(build_send_native_asset_with_tax_deduction_msg(
            deps,
            sender_address,
            recipient_address,
            denom.as_str(),
            amount,
        )?),
        Asset::Cw20 { contract_addr } => {
            let contract_addr = deps.api.addr_validate(&contract_addr)?;
            build_send_cw20_token_msg(recipient_address, contract_addr, amount)
        }
    }
}

/// Prepare BankMsg::Send message.
/// When doing native transfers a "tax" is charged.
/// The actual amount taken from the contract is: amount + tax.
/// Instead of sending amount, send: amount - compute_tax(amount).
pub fn build_send_native_asset_with_tax_deduction_msg(
    deps: Deps,
    _sender: Addr,
    recipient: Addr,
    denom: &str,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Bank(BankMsg::Send {
        to_address: recipient.into(),
        amount: vec![deduct_tax(
            deps,
            Coin {
                denom: denom.to_string(),
                amount,
            },
        )?],
    }))
}

pub fn build_send_cw20_token_msg(
    recipient: Addr,
    token_contract_address: Addr,
    amount: Uint128,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: token_contract_address.into(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: recipient.into(),
            amount,
        })?,
        funds: vec![],
    }))
}
