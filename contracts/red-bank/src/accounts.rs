use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{Addr, Decimal, Deps, DepsMut, StdError, StdResult};

use mars::asset::AssetType;
use mars::helpers::cw20_get_balance;

use crate::contract::{
    get_bit, get_updated_borrow_index, get_updated_liquidity_index, market_get_from_index,
};
use crate::error::ContractError;
use crate::state::{Debt, User, DEBTS};

/// User global position
pub struct UserPosition {
    /// NOTE: Not used yet
    pub _total_collateral_in_uusd: Uint256,
    pub total_debt_in_uusd: Uint256,
    pub total_collateralized_debt_in_uusd: Uint256,
    pub max_debt_in_uusd: Uint256,
    pub weighted_maintenance_margin_in_uusd: Uint256,
    pub health_status: UserHealthStatus,
    pub asset_positions: Vec<UserAssetPosition>,
}

impl UserPosition {
    /// Gets asset price used to build the position for a given reference
    pub fn get_asset_price(
        &self,
        asset_reference: &[u8],
        asset_label: &str,
    ) -> Result<Decimal256, ContractError> {
        let asset_position = self
            .asset_positions
            .iter()
            .find(|ap| ap.asset_reference.as_slice() == asset_reference);

        match asset_position {
            Some(position) => Ok(Decimal256::from(position.asset_price)),
            None => Err(ContractError::price_not_found(asset_label)),
        }
    }
}

/// User asset settlement
pub struct UserAssetPosition {
    pub asset_label: String,
    pub asset_type: AssetType,
    pub asset_reference: Vec<u8>,
    pub collateral_amount: Uint256,
    pub debt_amount: Uint256,
    pub uncollateralized_debt: bool,
    pub max_ltv: Decimal256,
    pub maintenance_margin: Decimal256,
    pub asset_price: Decimal,
}

pub enum UserHealthStatus {
    NotBorrowing,
    Borrowing(Decimal256),
}

/// Calculates the user data across the markets.
/// This includes the total debt/collateral balances in uusd,
/// the max debt in uusd, the average Liquidation threshold, and the Health factor.
pub fn get_user_position(
    deps: &DepsMut,
    block_time: u64,
    user_address: &Addr,
    oracle_address: Addr,
    user: &User,
    market_count: u32,
) -> StdResult<UserPosition> {
    let user_asset_positions = get_user_asset_positions(
        deps.as_ref(),
        market_count,
        user,
        user_address,
        oracle_address,
        block_time,
    )?;

    let mut total_collateral_in_uusd = Uint256::zero();
    let mut total_debt_in_uusd = Uint256::zero();
    let mut total_collateralized_debt_in_uusd = Uint256::zero();
    let mut max_debt_in_uusd = Uint256::zero();
    let mut weighted_maintenance_margin_in_uusd = Uint256::zero();

    for user_asset_position in &user_asset_positions {
        let asset_price = Decimal256::from(user_asset_position.asset_price);
        let collateral_in_uusd = user_asset_position.collateral_amount * asset_price;
        total_collateral_in_uusd += collateral_in_uusd;

        max_debt_in_uusd += collateral_in_uusd * user_asset_position.max_ltv;
        weighted_maintenance_margin_in_uusd +=
            collateral_in_uusd * user_asset_position.maintenance_margin;

        let debt_in_uusd = user_asset_position.debt_amount * asset_price;
        total_debt_in_uusd += debt_in_uusd;

        if !user_asset_position.uncollateralized_debt {
            total_collateralized_debt_in_uusd += debt_in_uusd;
        }
    }

    // When computing health factor we should not take debt into account that has been given
    // an uncollateralized loan limit
    let health_status = if total_collateralized_debt_in_uusd.is_zero() {
        UserHealthStatus::NotBorrowing
    } else {
        let health_factor = Decimal256::from_uint256(weighted_maintenance_margin_in_uusd)
            / Decimal256::from_uint256(total_collateralized_debt_in_uusd);
        UserHealthStatus::Borrowing(health_factor)
    };

    let user_position = UserPosition {
        _total_collateral_in_uusd: total_collateral_in_uusd,
        total_debt_in_uusd,
        total_collateralized_debt_in_uusd,
        max_debt_in_uusd,
        weighted_maintenance_margin_in_uusd,
        health_status,
        asset_positions: user_asset_positions,
    };

    Ok(user_position)
}

/// Goes through assets user has a position in and returns a vec containing the scaled debt
/// (denominated in the asset), a result from a specified computation for the current collateral
/// (denominated in asset) and some metadata to be used by the caller.
fn get_user_asset_positions(
    deps: Deps,
    market_count: u32,
    user: &User,
    user_address: &Addr,
    oracle_address: Addr,
    block_time: u64,
) -> StdResult<Vec<UserAssetPosition>> {
    let mut ret: Vec<UserAssetPosition> = vec![];

    for i in 0_u32..market_count {
        let user_is_using_as_collateral = get_bit(user.collateral_assets, i)?;
        let user_is_borrowing = get_bit(user.borrowed_assets, i)?;
        if !(user_is_using_as_collateral || user_is_borrowing) {
            continue;
        }

        let (asset_reference_vec, market) = market_get_from_index(&deps, i)?;

        let (collateral_amount, max_ltv, maintenance_margin) = if user_is_using_as_collateral {
            // query asset balance (ma_token contract gives back a scaled value)
            let asset_balance = cw20_get_balance(
                &deps.querier,
                market.ma_token_address.clone(),
                user_address.clone(),
            )?;

            let liquidity_index = get_updated_liquidity_index(&market, block_time);
            let collateral_amount = Uint256::from(asset_balance) * liquidity_index;

            (
                collateral_amount,
                market.max_loan_to_value,
                market.maintenance_margin,
            )
        } else {
            (Uint256::zero(), Decimal256::zero(), Decimal256::zero())
        };

        let (debt_amount, uncollateralized_debt) = if user_is_borrowing {
            // query debt
            let user_debt: Debt =
                DEBTS.load(deps.storage, (asset_reference_vec.as_slice(), user_address))?;

            let borrow_index = get_updated_borrow_index(&market, block_time);

            (
                user_debt.amount_scaled * borrow_index,
                user_debt.uncollateralized,
            )
        } else {
            (Uint256::zero(), false)
        };

        let asset_label = match market.asset_type {
            AssetType::Native => match String::from_utf8(asset_reference_vec.clone()) {
                Ok(res) => res,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            },
            AssetType::Cw20 => match String::from_utf8(asset_reference_vec.clone()) {
                Ok(res) => res,
                Err(_) => {
                    return Err(StdError::generic_err(
                        "failed to encode Cw20 address into string",
                    ))
                }
            },
        };

        let asset_price = mars::oracle::helpers::query_price(
            deps.querier,
            oracle_address.clone(),
            &asset_label,
            asset_reference_vec.clone(),
            market.asset_type.clone(),
        )?;

        let user_asset_position = UserAssetPosition {
            asset_label,
            asset_type: market.asset_type,
            asset_reference: asset_reference_vec,
            collateral_amount,
            debt_amount,
            uncollateralized_debt,
            max_ltv,
            maintenance_margin,
            asset_price,
        };
        ret.push(user_asset_position);
    }

    Ok(ret)
}
