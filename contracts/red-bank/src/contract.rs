use std::str;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, entry_point, from_binary, to_binary, Addr, Attribute, BankMsg, Binary, Coin, CosmosMsg,
    Deps, DepsMut, Env, MessageInfo, Order, QuerierWrapper, Response, StdError, StdResult, SubMsg,
    Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use terra_cosmwasm::TerraQuerier;

use mars::address_provider;
use mars::address_provider::msg::MarsContract;
use mars::helpers::{cw20_get_balance, cw20_get_symbol, option_string_to_addr, zero_address};
use mars::asset::{Asset, AssetType, asset_get_attributes};
use mars::ma_token;
use mars::red_bank::msg::{
    CollateralInfo, CollateralResponse, ConfigResponse, CreateOrUpdateConfig,
    DebtInfo, DebtResponse, ExecuteMsg, InitOrUpdateAssetParams, InstantiateMsg, MarketInfo,
    MarketResponse, MarketsListResponse, MigrateMsg, QueryMsg, ReceiveMsg,
    UncollateralizedLoanLimitResponse,
};

use crate::error::ContractError;
use crate::state::{
    Config, Debt, Market, MarketReferences, RedBank, User, CONFIG, DEBTS, MARKETS,
    MARKET_MA_TOKENS, MARKET_REFERENCES, RED_BANK, UNCOLLATERALIZED_LOAN_LIMITS, USERS,
};
use cw_storage_plus::U32Key;
use mars::error::MarsError;
use mars::tax::deduct_tax;

// CONSTANTS

const SECONDS_PER_YEAR: u64 = 31536000u64;

// INIT

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        owner,
        address_provider_address,
        insurance_fund_fee_share,
        treasury_fee_share,
        ma_token_code_id,
        close_factor,
    } = msg.config;

    // All fields should be available
    let available = owner.is_some()
        && address_provider_address.is_some()
        && insurance_fund_fee_share.is_some()
        && treasury_fee_share.is_some()
        && ma_token_code_id.is_some()
        && close_factor.is_some();

    if !available {
        return Err(StdError::generic_err(
            "All params should be available during initialization",
        ));
    };

    let config = Config {
        owner: option_string_to_addr(deps.api, owner, zero_address())?,
        address_provider_address: option_string_to_addr(
            deps.api,
            address_provider_address,
            zero_address(),
        )?,
        ma_token_code_id: ma_token_code_id.unwrap(),
        close_factor: close_factor.unwrap(),
        insurance_fund_fee_share: insurance_fund_fee_share.unwrap(),
        treasury_fee_share: treasury_fee_share.unwrap(),
    };
    config.validate()?;

    CONFIG.save(deps.storage, &config)?;

    RED_BANK.save(deps.storage, &RedBank { market_count: 0 })?;

    Ok(Response::default())
}

// HANDLERS

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::UpdateConfig { config } => execute_update_config(deps, env, info, config),

        ExecuteMsg::Receive(cw20_msg) => execute_receive_cw20(deps, env, info, cw20_msg),

        ExecuteMsg::InitAsset {
            asset,
            asset_params,
        } => execute_init_asset(deps, env, info, asset, asset_params),

        ExecuteMsg::UpdateAsset {
            asset,
            asset_params,
        } => execute_update_asset(deps, env, info, asset, asset_params),

        ExecuteMsg::InitAssetTokenCallback { reference } => {
            init_asset_token_callback(deps, env, info, reference)
        }

        ExecuteMsg::DepositNative { denom } => {
            let deposit_amount = get_denom_amount_from_coins(&info.funds, &denom);
            let depositor_address = info.sender.clone();
            execute_deposit(
                deps,
                env,
                info,
                depositor_address,
                denom.as_bytes(),
                denom.as_str(),
                deposit_amount,
            )
        }

        ExecuteMsg::Borrow { asset, amount } => execute_borrow(deps, env, info, asset, amount),

        ExecuteMsg::RepayNative { denom } => {
            let repayer_address = info.sender.clone();
            let repay_amount = get_denom_amount_from_coins(&info.funds, &denom);
            execute_repay(
                deps,
                env,
                info,
                repayer_address,
                denom.as_bytes(),
                denom.as_str(),
                repay_amount,
                AssetType::Native,
            )
        }

        ExecuteMsg::LiquidateNative {
            collateral_asset,
            debt_asset,
            user_address,
            receive_ma_token,
        } => {
            let sender = info.sender.clone();
            let user_addr = deps.api.addr_validate(&user_address)?;
            let sent_debt_asset_amount = get_denom_amount_from_coins(&info.funds, &debt_asset);
            execute_liquidate(
                deps,
                env,
                info,
                sender,
                collateral_asset,
                Asset::Native { denom: debt_asset },
                user_addr,
                sent_debt_asset_amount,
                receive_ma_token,
            )
        }

        ExecuteMsg::FinalizeLiquidityTokenTransfer {
            sender_address,
            recipient_address,
            sender_previous_balance,
            recipient_previous_balance,
            amount,
        } => {
            let sender_addr = deps.api.addr_validate(&sender_address)?;
            let recipient_addr = deps.api.addr_validate(&recipient_address)?;
            execute_finalize_liquidity_token_transfer(
                deps,
                env,
                info,
                sender_addr,
                recipient_addr,
                sender_previous_balance,
                recipient_previous_balance,
                amount,
            )
        }

        ExecuteMsg::UpdateUncollateralizedLoanLimit {
            user_address,
            asset,
            new_limit,
        } => {
            let user_addr = deps.api.addr_validate(&user_address)?;
            execute_update_uncollateralized_loan_limit(deps, env, info, user_addr, asset, new_limit)
        }
        ExecuteMsg::UpdateUserCollateralAssetStatus { asset, enable } => {
            execute_update_user_collateral_asset_status(deps, env, info, asset, enable)
        }

        ExecuteMsg::DistributeProtocolIncome { asset, amount } => {
            execute_distribute_protocol_income(deps, env, info, asset, amount)
        }

        ExecuteMsg::Withdraw { asset, amount } => execute_withdraw(deps, env, info, asset, amount),
    }
}

/// Update config
pub fn execute_update_config(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    new_config: CreateOrUpdateConfig,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {}.into());
    }

    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        owner,
        address_provider_address,
        insurance_fund_fee_share,
        treasury_fee_share,
        ma_token_code_id,
        close_factor,
    } = new_config;

    // Update config
    config.owner = option_string_to_addr(deps.api, owner, config.owner)?;
    config.address_provider_address = option_string_to_addr(
        deps.api,
        address_provider_address,
        config.address_provider_address,
    )?;
    config.ma_token_code_id = ma_token_code_id.unwrap_or(config.ma_token_code_id);
    config.close_factor = close_factor.unwrap_or(config.close_factor);
    config.insurance_fund_fee_share =
        insurance_fund_fee_share.unwrap_or(config.insurance_fund_fee_share);
    config.treasury_fee_share = treasury_fee_share.unwrap_or(config.treasury_fee_share);

    // Validate config
    config.validate()?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

/// cw20 receive implementation
pub fn execute_receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    match from_binary(&cw20_msg.msg)? {
        ReceiveMsg::DepositCw20 {} => {
            let depositor_addr = deps.api.addr_validate(&cw20_msg.sender)?;
            let token_contract_address = info.sender.clone();
            execute_deposit(
                deps,
                env,
                info,
                depositor_addr,
                token_contract_address.as_bytes(),
                token_contract_address.as_str(),
                Uint256::from(cw20_msg.amount),
            )
        }
        ReceiveMsg::RepayCw20 {} => {
            let repayer_addr = deps.api.addr_validate(&cw20_msg.sender)?;
            let token_contract_address = info.sender.clone();
            execute_repay(
                deps,
                env,
                info,
                repayer_addr,
                token_contract_address.as_bytes(),
                token_contract_address.as_str(),
                Uint256::from(cw20_msg.amount),
                AssetType::Cw20,
            )
        }
        ReceiveMsg::LiquidateCw20 {
            collateral_asset,
            debt_asset_address,
            user_address,
            receive_ma_token,
        } => {
            let debt_asset_addr = deps.api.addr_validate(&debt_asset_address)?;
            if info.sender != debt_asset_addr {
                return Err(StdError::generic_err(format!(
                    "Incorrect asset, must send {} in order to liquidate",
                    debt_asset_address
                ))
                .into());
            }
            let liquidator_addr = deps.api.addr_validate(&cw20_msg.sender)?;
            let user_addr = deps.api.addr_validate(&user_address)?;
            let sent_debt_asset_amount = Uint256::from(cw20_msg.amount);
            execute_liquidate(
                deps,
                env,
                info,
                liquidator_addr,
                collateral_asset,
                Asset::Cw20 {
                    contract_addr: debt_asset_address,
                },
                user_addr,
                sent_debt_asset_amount,
                receive_ma_token,
            )
        }
    }
}

/// Burns sent maAsset in exchange of underlying asset
pub fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset: Asset,
    amount: Option<Uint256>,
) -> Result<Response, ContractError> {
    let withdrawer_addr = info.sender;

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&asset)?;
    let mut market = MARKETS.load(deps.storage, asset_reference.as_slice())?;

    let asset_ma_addr = market.ma_token_address.clone();
    let withdrawer_balance_scaled = Uint256::from(cw20_get_balance(
        &deps.querier,
        asset_ma_addr,
        withdrawer_addr.clone(),
    )?);

    if withdrawer_balance_scaled.is_zero() {
        return Err(StdError::generic_err(
            format!("User has no balance (asset: {})", asset_label,),
        )
        .into());
    }

    // Check user has sufficient balance to send back
    let (withdraw_amount, withdraw_amount_scaled) = match amount {
        Some(amount) => {
            let amount_scaled =
                amount / get_updated_liquidity_index(&market, env.block.time.seconds());
            if amount_scaled.is_zero() || amount_scaled > withdrawer_balance_scaled {
                return Err(StdError::generic_err(format!(
                    "Withdraw amount must be greater than 0 and less or equal user balance (asset: {})",
                    asset_label,
                )).into());
            };
            (amount, amount_scaled)
        }
        None => {
            // NOTE: We prefer to just do one multiplication equation instead of two: division and multiplication.
            // This helps to avoid rounding errors if we want to be sure in burning total balance.
            let withdrawer_balance = withdrawer_balance_scaled
                * get_updated_liquidity_index(&market, env.block.time.seconds());
            (withdrawer_balance, withdrawer_balance_scaled)
        }
    };

    let mut withdrawer = USERS.load(deps.storage, &withdrawer_addr)?;
    let asset_as_collateral = get_bit(withdrawer.collateral_assets, market.index)?;
    let user_is_borrowing = !withdrawer.borrowed_assets.is_zero();

    // if asset is used as collateral and user is borrowing we need to validate health factor after withdraw,
    // otherwise no reasons to block the withdraw
    if asset_as_collateral && user_is_borrowing {
        let money_market = RED_BANK.load(deps.storage)?;

        let (user_account_settlement, native_asset_prices) = prepare_user_account_settlement(
            &deps,
            env.block.time.seconds(),
            &withdrawer_addr,
            &withdrawer,
            &money_market,
            &mut vec![],
        )?;

        let withdraw_asset_price =
            asset_get_price(asset_label.as_str(), &native_asset_prices, &asset_type)?;
        let withdraw_amount_in_uusd = withdraw_amount * withdraw_asset_price;

        let health_factor_after_withdraw =
            Decimal256::from_uint256(
                user_account_settlement.weighted_maintenance_margin_in_uusd
                    - (withdraw_amount_in_uusd * market.maintenance_margin),
            ) / Decimal256::from_uint256(user_account_settlement.total_collateralized_debt_in_uusd);
        if health_factor_after_withdraw < Decimal256::one() {
            return Err(StdError::generic_err(
                "User's health factor can't be less than 1 after withdraw",
            )
            .into());
        }
    }

    // if max collateral to withdraw equals the user's balance then unset collateral bit
    if asset_as_collateral && withdraw_amount_scaled == withdrawer_balance_scaled {
        unset_bit(&mut withdrawer.collateral_assets, market.index)?;
        USERS.save(deps.storage, &withdrawer_addr, &withdrawer)?;
    }

    market_apply_accumulated_interests(&env, &mut market);
    market_update_interest_rates(
        &deps,
        &env,
        asset_reference.as_slice(),
        &mut market,
        withdraw_amount,
    )?;
    MARKETS.save(deps.storage, asset_reference.as_slice(), &market)?;

    let burn_ma_tokens_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: market.ma_token_address.to_string(),
        msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
            user: withdrawer_addr.to_string(),
            amount: withdraw_amount_scaled.into(),
        })?,
        funds: vec![],
    });
    let burn_ma_tokens_msg = SubMsg::new(burn_ma_tokens_msg);

    let send_underlying_asset_msg = build_send_asset_msg(
        deps.as_ref(),
        env.contract.address,
        withdrawer_addr.clone(),
        asset,
        withdraw_amount,
    )?;
    let send_underlying_asset_msg = SubMsg::new(send_underlying_asset_msg);

    let messages = vec![burn_ma_tokens_msg, send_underlying_asset_msg];

    let mut attributes = vec![
        attr("action", "withdraw"),
        attr("market", asset_label.as_str()),
        attr("user", withdrawer_addr.as_str()),
        attr("burn_amount", withdraw_amount_scaled),
        attr("withdraw_amount", withdraw_amount),
    ];

    append_indices_and_rates_to_logs(&mut attributes, &market);

    Ok(Response {
        messages,
        attributes,
        events: vec![],
        data: None,
    })
}

/// Initialize asset if not exist.
/// Initialization requires that all params are provided and there is no asset in state.
pub fn execute_init_asset(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset: Asset,
    asset_params: InitOrUpdateAssetParams,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {}.into());
    }

    let mut money_market = RED_BANK.load(deps.storage)?;

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&asset)?;
    let market_option = MARKETS.may_load(deps.storage, asset_reference.as_slice())?;
    match market_option {
        None => {
            let market_idx = money_market.market_count;
            let new_market = Market::create(env.block.time, market_idx, asset_type, asset_params)?;

            // Save new market
            MARKETS.save(deps.storage, asset_reference.as_slice(), &new_market)?;

            // Save index to reference mapping
            MARKET_REFERENCES.save(
                deps.storage,
                U32Key::new(market_idx),
                &MarketReferences {
                    reference: asset_reference.to_vec(),
                },
            )?;

            // Increment market count
            money_market.market_count += 1;
            RED_BANK.save(deps.storage, &money_market)?;

            let symbol = match asset {
                Asset::Native { denom } => denom,
                Asset::Cw20 { contract_addr } => {
                    let contract_addr = deps.api.addr_validate(&contract_addr)?;
                    cw20_get_symbol(&deps.querier, contract_addr)?
                }
            };

            // Prepare response, should instantiate an maToken
            // and use the Register hook.
            // A new maToken should be created which callbacks this contract in order to be registered.
            let incentives_address = address_provider::helpers::query_address(
                &deps.querier,
                config.address_provider_address,
                MarsContract::Incentives,
            )?;

            Ok(Response {
                attributes: vec![attr("action", "init_asset"), attr("asset", asset_label)],
                events: vec![],
                data: None,
                messages: vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Instantiate {
                    admin: None,
                    code_id: config.ma_token_code_id,
                    msg: to_binary(&ma_token::msg::InstantiateMsg {
                        name: format!("mars {} liquidity token", symbol),
                        symbol: format!("ma{}", symbol),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: env.contract.address.to_string(),
                            cap: None,
                        }),
                        red_bank_address: env.contract.address.to_string(),
                        incentives_address: incentives_address.into(),
                    })?,
                    funds: vec![],
                    label: String::from(""),
                }))],
            })
        }
        Some(_) => Err(StdError::generic_err("Asset already initialized").into()),
    }
}

/// Update asset with new params.
pub fn execute_update_asset(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset: Asset,
    asset_params: InitOrUpdateAssetParams,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {}.into());
    }

    let (asset_label, asset_reference, _asset_type) = asset_get_attributes(&asset)?;
    let market_option = MARKETS.may_load(deps.storage, asset_reference.as_slice())?;
    match market_option {
        Some(market) => {
            let updated_market = market.update_with(asset_params)?;

            // Save updated market
            MARKETS.save(deps.storage, asset_reference.as_slice(), &updated_market)?;

            Ok(Response {
                attributes: vec![attr("action", "update_asset"), attr("asset", asset_label)],
                events: vec![],
                data: None,
                messages: vec![],
            })
        }
        None => Err(StdError::generic_err("Asset not initialized").into()),
    }
}

pub fn init_asset_token_callback(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    reference: Vec<u8>,
) -> Result<Response, ContractError> {
    let mut market = MARKETS.load(deps.storage, reference.as_slice())?;

    if market.ma_token_address == zero_address() {
        let ma_contract_addr = info.sender;

        market.ma_token_address = ma_contract_addr.clone();
        MARKETS.save(deps.storage, reference.as_slice(), &market)?;

        // save ma token contract to reference mapping
        MARKET_MA_TOKENS.save(deps.storage, &ma_contract_addr, &reference)?;

        Ok(Response::default())
    } else {
        // Can do this only once
        Err(MarsError::Unauthorized {}.into())
    }
}

/// Execute deposits and mint corresponding ma_tokens
pub fn execute_deposit(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    depositor_address: Addr,
    asset_reference: &[u8],
    asset_label: &str,
    deposit_amount: Uint256,
) -> Result<Response, ContractError> {
    let mut market = MARKETS.load(deps.storage, asset_reference)?;

    // Cannot deposit zero amount
    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            asset_label,
        ))
        .into());
    }

    let mut user = USERS
        .may_load(deps.storage, &depositor_address)?
        .unwrap_or_default();

    let has_deposited_asset = get_bit(user.collateral_assets, market.index)?;
    if !has_deposited_asset {
        set_bit(&mut user.collateral_assets, market.index)?;
        USERS.save(deps.storage, &depositor_address, &user)?;
    }

    market_apply_accumulated_interests(&env, &mut market);
    market_update_interest_rates(&deps, &env, asset_reference, &mut market, Uint256::zero())?;
    MARKETS.save(deps.storage, asset_reference, &market)?;

    if market.liquidity_index.is_zero() {
        return Err(StdError::generic_err("Cannot have 0 as liquidity index").into());
    }
    let mint_amount =
        deposit_amount / get_updated_liquidity_index(&market, env.block.time.seconds());

    let mut attributes = vec![
        attr("action", "deposit"),
        attr("market", asset_label),
        attr("user", depositor_address.as_str()),
        attr("amount", deposit_amount),
    ];

    append_indices_and_rates_to_logs(&mut attributes, &market);

    Ok(Response {
        data: None,
        attributes,
        messages: vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: market.ma_token_address.into(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: depositor_address.into(),
                amount: mint_amount.into(),
            })?,
            funds: vec![],
        }))],
        events: vec![],
    })
}

/// Add debt for the borrower and send the borrowed funds
pub fn execute_borrow(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset: Asset,
    borrow_amount: Uint256,
) -> Result<Response, ContractError> {
    let borrower_address = info.sender;

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&asset)?;

    // Cannot borrow zero amount
    if borrow_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Borrow amount must be greater than 0 {}",
            asset_label,
        ))
        .into());
    }

    let money_market = RED_BANK.load(deps.storage)?;
    let mut borrow_market = match MARKETS.load(deps.storage, asset_reference.as_slice()) {
        Ok(borrow_market) => borrow_market,
        Err(_) => {
            return Err(StdError::generic_err(format!(
                "no borrow market exists with asset reference: {}",
                String::from_utf8(asset_reference).expect("Found invalid UTF-8")
            ))
            .into());
        }
    };

    let uncollateralized_loan_limit = UNCOLLATERALIZED_LOAN_LIMITS
        .may_load(
            deps.storage,
            (asset_reference.as_slice(), &borrower_address),
        )?
        .unwrap_or_else(Uint128::zero);

    let mut user: User = match USERS.may_load(deps.storage, &borrower_address)? {
        Some(user) => user,
        None => {
            if uncollateralized_loan_limit.is_zero() {
                return Err(StdError::generic_err("address has no collateral deposited").into());
            }
            // If User has some uncollateralized_loan_limit, then we don't require an existing debt position and initialize a new one.
            User::default()
        }
    };

    // TODO: Check the contract has enough funds to safely lend them

    let mut uncollateralized_debt = false;
    if uncollateralized_loan_limit.is_zero() {
        // Collateralized loan: validate user has enough collateral if they have no uncollateralized loan limit
        let mut native_asset_prices_to_query: Vec<String> = match asset {
            Asset::Native { .. } if asset_label != "uusd" => vec![asset_label.clone()],
            _ => vec![],
        };

        let (user_account_settlement, native_asset_prices) = prepare_user_account_settlement(
            &deps,
            env.block.time.seconds(),
            &borrower_address,
            &user,
            &money_market,
            &mut native_asset_prices_to_query,
        )?;

        let borrow_asset_price =
            asset_get_price(asset_label.as_str(), &native_asset_prices, &asset_type)?;
        let borrow_amount_in_uusd = borrow_amount * borrow_asset_price;

        if user_account_settlement.total_debt_in_uusd + borrow_amount_in_uusd
            > user_account_settlement.max_debt_in_uusd
        {
            return Err(StdError::generic_err(
                "borrow amount exceeds maximum allowed given current collateral value",
            )
            .into());
        }
    } else {
        // Uncollateralized loan: check borrow amount plus debt does not exceed uncollateralized loan limit
        uncollateralized_debt = true;

        let borrower_debt = DEBTS
            .may_load(
                deps.storage,
                (asset_reference.as_slice(), &borrower_address),
            )?
            .unwrap_or(Debt {
                amount_scaled: Uint256::zero(),
                uncollateralized: uncollateralized_debt,
            });

        let asset_market = MARKETS.load(deps.storage, asset_reference.as_slice())?;
        let debt_amount = borrower_debt.amount_scaled
            * get_updated_borrow_index(&asset_market, env.block.time.seconds());
        if borrow_amount + debt_amount > Uint256::from(uncollateralized_loan_limit) {
            return Err(StdError::generic_err(
                "borrow amount exceeds uncollateralized loan limit given existing debt",
            )
            .into());
        }
    }

    market_apply_accumulated_interests(&env, &mut borrow_market);

    // Set borrowing asset for user
    let is_borrowing_asset = get_bit(user.borrowed_assets, borrow_market.index)?;
    if !is_borrowing_asset {
        set_bit(&mut user.borrowed_assets, borrow_market.index)?;
        USERS.save(deps.storage, &borrower_address, &user)?;
    }

    // Set new debt
    let mut debt = DEBTS
        .may_load(
            deps.storage,
            (asset_reference.as_slice(), &borrower_address),
        )?
        .unwrap_or(Debt {
            amount_scaled: Uint256::zero(),
            uncollateralized: uncollateralized_debt,
        });
    let borrow_amount_scaled =
        borrow_amount / get_updated_borrow_index(&borrow_market, env.block.time.seconds());
    debt.amount_scaled += borrow_amount_scaled;
    DEBTS.save(
        deps.storage,
        (asset_reference.as_slice(), &borrower_address),
        &debt,
    )?;

    borrow_market.debt_total_scaled += borrow_amount_scaled;

    market_update_interest_rates(
        &deps,
        &env,
        asset_reference.as_slice(),
        &mut borrow_market,
        borrow_amount,
    )?;
    MARKETS.save(deps.storage, asset_reference.as_slice(), &borrow_market)?;

    let mut attributes = vec![
        attr("action", "borrow"),
        attr("market", asset_label.as_str()),
        attr("user", borrower_address.as_str()),
        attr("amount", borrow_amount),
    ];

    append_indices_and_rates_to_logs(&mut attributes, &borrow_market);

    // Send borrow amount to borrower
    let send_msg = build_send_asset_msg(
        deps.as_ref(),
        env.contract.address,
        borrower_address,
        asset,
        borrow_amount,
    )?;

    Ok(Response {
        data: None,
        attributes,
        messages: vec![SubMsg::new(send_msg)],
        events: vec![],
    })
}

/// Handle the repay of native tokens. Refund extra funds if they exist
pub fn execute_repay(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    repayer_address: Addr,
    asset_reference: &[u8],
    asset_label: &str,
    repay_amount: Uint256,
    asset_type: AssetType,
) -> Result<Response, ContractError> {
    // TODO: assumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    let mut market = MARKETS.load(deps.storage, asset_reference)?;

    // Get repay amount
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    // Cannot repay zero amount
    if repay_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Repay amount must be greater than 0 {}",
            asset_label,
        ))
        .into());
    }

    // Check new debt
    let mut debt = DEBTS.load(deps.storage, (asset_reference, &repayer_address))?;

    if debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("Cannot repay 0 debt").into());
    }

    market_apply_accumulated_interests(&env, &mut market);

    let mut repay_amount_scaled =
        repay_amount / get_updated_borrow_index(&market, env.block.time.seconds());

    let mut messages = vec![];
    let mut refund_amount = Uint256::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        // TODO: Should we log this?
        refund_amount = (repay_amount_scaled - debt.amount_scaled)
            * get_updated_borrow_index(&market, env.block.time.seconds());
        let refund_msg = match asset_type {
            AssetType::Native => build_send_native_asset_msg(
                deps.as_ref(),
                env.contract.address.clone(),
                repayer_address.clone(),
                asset_label,
                refund_amount,
            )?,
            AssetType::Cw20 => {
                let token_contract_addr = deps.api.addr_validate(asset_label)?;
                build_send_cw20_token_msg(
                    repayer_address.clone(),
                    token_contract_addr,
                    refund_amount,
                )?
            }
        };
        messages.push(SubMsg::new(refund_msg));
        repay_amount_scaled = debt.amount_scaled;
    }

    debt.amount_scaled = debt.amount_scaled - repay_amount_scaled;
    DEBTS.save(deps.storage, (asset_reference, &repayer_address), &debt)?;

    if repay_amount_scaled > market.debt_total_scaled {
        return Err(StdError::generic_err("Amount to repay is greater than total debt").into());
    }
    market.debt_total_scaled = market.debt_total_scaled - repay_amount_scaled;
    market_update_interest_rates(&deps, &env, asset_reference, &mut market, Uint256::zero())?;
    MARKETS.save(deps.storage, asset_reference, &market)?;

    if debt.amount_scaled == Uint256::zero() {
        // Remove asset from borrowed assets
        let mut user = USERS.load(deps.storage, &repayer_address)?;
        unset_bit(&mut user.borrowed_assets, market.index)?;
        USERS.save(deps.storage, &repayer_address, &user)?;
    }

    let mut attributes = vec![
        attr("action", "repay"),
        attr("market", asset_label),
        attr("user", repayer_address),
        attr("amount", repay_amount - refund_amount),
    ];

    append_indices_and_rates_to_logs(&mut attributes, &market);

    Ok(Response {
        data: None,
        messages,
        attributes,
        events: vec![],
    })
}

/// Execute loan liquidations on under-collateralized loans
pub fn execute_liquidate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    liquidator_address: Addr,
    collateral_asset: Asset,
    debt_asset: Asset,
    user_address: Addr,
    sent_debt_asset_amount: Uint256,
    receive_ma_token: bool,
) -> Result<Response, ContractError> {
    let block_time = env.block.time.seconds();

    let (debt_asset_label, debt_asset_reference, _) = asset_get_attributes(&debt_asset)?;
    // 1. Validate liquidation
    // If user (contract) has a positive uncollateralized limit then the user
    // cannot be liquidated
    let uncollateralized_loan_limit = match UNCOLLATERALIZED_LOAN_LIMITS.may_load(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
    ) {
        Ok(Some(limit)) => limit,
        Ok(None) => Uint128::zero(),
        Err(error) => return Err(error.into()),
    };
    if uncollateralized_loan_limit > Uint128::zero() {
        return Err(StdError::generic_err(
            "user has a positive uncollateralized loan limit and thus cannot be liquidated",
        )
        .into());
    }

    // liquidator must send positive amount of funds in the debt asset
    if sent_debt_asset_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Must send more than 0 {} in order to liquidate",
            debt_asset_label,
        ))
        .into());
    }

    let (collateral_asset_label, collateral_asset_reference, _) =
        asset_get_attributes(&collateral_asset)?;

    let mut collateral_market =
        MARKETS.load(deps.storage, collateral_asset_reference.as_slice())?;

    // check if user has available collateral in specified collateral asset to be liquidated
    let user_collateral_balance = get_updated_liquidity_index(&collateral_market, block_time)
        * Uint256::from(cw20_get_balance(
            &deps.querier,
            collateral_market.ma_token_address.clone(),
            user_address.clone(),
        )?);
    if user_collateral_balance == Uint256::zero() {
        return Err(StdError::generic_err(
            "user has no balance in specified collateral asset to be liquidated",
        )
        .into());
    }

    // check if user has outstanding debt in the deposited asset that needs to be repayed
    let user_debt = DEBTS.load(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
    )?;
    if user_debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("User has no outstanding debt in the specified debt asset and thus cannot be liquidated").into());
    }

    // 2. Compute health factor
    let config = CONFIG.load(deps.storage)?;
    let money_market = RED_BANK.load(deps.storage)?;
    let user = USERS.load(deps.storage, &user_address)?;
    let (user_account_settlement, native_asset_prices) = prepare_user_account_settlement(
        &deps,
        block_time,
        &user_address,
        &user,
        &money_market,
        &mut vec![],
    )?;

    let health_factor = match user_account_settlement.health_status {
        // NOTE: Should not get in practice as it would fail on the debt asset check
        UserHealthStatus::NotBorrowing => {
            return Err(StdError::generic_err(
                "User has no outstanding debt and thus cannot be liquidated",
            )
            .into())
        }
        UserHealthStatus::Borrowing(hf) => hf,
    };

    // if health factor is not less than one the user cannot be liquidated
    if health_factor >= Decimal256::one() {
        return Err(StdError::generic_err(
            "User's health factor is not less than 1 and thus cannot be liquidated",
        )
        .into());
    }

    let mut debt_market = MARKETS.load(deps.storage, debt_asset_reference.as_slice())?;

    // 3. Compute debt to repay and collateral to liquidate
    let collateral_price = asset_get_price(
        collateral_asset_label.as_str(),
        &native_asset_prices,
        &collateral_market.asset_type,
    )?;
    let debt_price = asset_get_price(
        debt_asset_label.as_str(),
        &native_asset_prices,
        &debt_market.asset_type,
    )?;

    market_apply_accumulated_interests(&env, &mut debt_market);

    let user_debt_asset_total_debt =
        user_debt.amount_scaled * get_updated_borrow_index(&debt_market, block_time);

    let (debt_amount_to_repay, collateral_amount_to_liquidate, refund_amount) =
        liquidation_compute_amounts(
            collateral_price,
            debt_price,
            config.close_factor,
            user_collateral_balance,
            collateral_market.liquidation_bonus,
            user_debt_asset_total_debt,
            sent_debt_asset_amount,
        );

    let mut messages = vec![];
    // 4. Update collateral positions and market depending on whether the liquidator elects to
    // receive ma_tokens or the underlying asset
    if receive_ma_token {
        // Transfer ma tokens from user to liquidator
        let mut liquidator = USERS
            .may_load(deps.storage, &liquidator_address)?
            .unwrap_or_default();

        // set liquidator's deposited bit to true if not already true
        // NOTE: previous checks should ensure this amount is not zero
        let liquidator_is_using_as_collateral =
            get_bit(liquidator.collateral_assets, collateral_market.index)?;
        if !liquidator_is_using_as_collateral {
            set_bit(&mut liquidator.collateral_assets, collateral_market.index)?;
            USERS.save(deps.storage, &liquidator_address, &liquidator)?;
        }

        let collateral_amount_to_liquidate_scaled = collateral_amount_to_liquidate
            / get_updated_liquidity_index(&collateral_market, block_time);

        messages.push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: collateral_market.ma_token_address.to_string(),
            msg: to_binary(&mars::ma_token::msg::ExecuteMsg::TransferOnLiquidation {
                sender: user_address.to_string(),
                recipient: liquidator_address.to_string(),
                amount: collateral_amount_to_liquidate_scaled.into(),
            })?,
            funds: vec![],
        })));
    } else {
        // Burn ma_tokens from user and send underlying asset to liquidator

        // Ensure contract has enough collateral to send back underlying asset
        let contract_collateral_balance = match collateral_asset.clone() {
            Asset::Native { denom } => Uint256::from(
                deps.querier
                    .query_balance(env.contract.address.clone(), denom.as_str())?
                    .amount,
            ),
            Asset::Cw20 {
                contract_addr: token_addr,
            } => {
                let token_addr = deps.api.addr_validate(&token_addr)?;
                Uint256::from(cw20_get_balance(
                    &deps.querier,
                    token_addr,
                    env.contract.address.clone(),
                )?)
            }
        };
        let contract_collateral_balance = contract_collateral_balance
            * get_updated_liquidity_index(&collateral_market, block_time);
        if contract_collateral_balance < collateral_amount_to_liquidate {
            return Err(StdError::generic_err(
                "contract does not have enough collateral liquidity to send back underlying asset",
            )
            .into());
        }

        // apply update collateral interest as liquidity is reduced
        market_apply_accumulated_interests(&env, &mut collateral_market);
        market_update_interest_rates(
            &deps,
            &env,
            collateral_asset_reference.as_slice(),
            &mut collateral_market,
            collateral_amount_to_liquidate,
        )?;

        let collateral_amount_to_liquidate_scaled = collateral_amount_to_liquidate
            / get_updated_liquidity_index(&collateral_market, block_time);

        let burn_ma_tokens_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: collateral_market.ma_token_address.to_string(),
            msg: to_binary(&mars::ma_token::msg::ExecuteMsg::Burn {
                user: user_address.to_string(),
                amount: collateral_amount_to_liquidate_scaled.into(),
            })?,
            funds: vec![],
        });

        let send_underlying_asset_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address.clone(),
            liquidator_address.clone(),
            collateral_asset,
            collateral_amount_to_liquidate,
        )?;
        messages.push(SubMsg::new(burn_ma_tokens_msg));
        messages.push(SubMsg::new(send_underlying_asset_msg));
    }

    // if max collateral to liquidate equals the user's balance then unset collateral bit
    if collateral_amount_to_liquidate == user_collateral_balance {
        let mut user = USERS.load(deps.storage, &user_address)?;
        unset_bit(&mut user.collateral_assets, collateral_market.index)?;
        USERS.save(deps.storage, &user_address, &user)?;
    }

    // 5. Update debt market and positions

    let debt_amount_to_repay_scaled =
        debt_amount_to_repay / get_updated_borrow_index(&debt_market, block_time);

    // update user and market debt
    let mut debt = DEBTS.load(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
    )?;
    // NOTE: Should be > 0 as amount to repay is capped by the close factor
    debt.amount_scaled = debt.amount_scaled - debt_amount_to_repay_scaled;
    DEBTS.save(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
        &debt,
    )?;
    debt_market.debt_total_scaled = debt_market.debt_total_scaled - debt_amount_to_repay_scaled;

    market_update_interest_rates(
        &deps,
        &env,
        debt_asset_reference.as_slice(),
        &mut debt_market,
        refund_amount,
    )?;

    // save markets
    MARKETS.save(deps.storage, debt_asset_reference.as_slice(), &debt_market)?;
    MARKETS.save(
        deps.storage,
        collateral_asset_reference.as_slice(),
        &collateral_market,
    )?;

    // 6. Build response
    // refund sent amount in excess of actual debt amount to liquidate
    if refund_amount > Uint256::zero() {
        let refund_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address,
            liquidator_address.clone(),
            debt_asset,
            refund_amount,
        )?;
        messages.push(SubMsg::new(refund_msg));
    }

    let mut attributes = vec![
        attr("action", "liquidate"),
        attr("collateral_market", collateral_asset_label),
        attr("debt_market", debt_asset_label),
        attr("user", user_address.as_str()),
        attr("liquidator", liquidator_address.as_str()),
        attr(
            "collateral_amount_liquidated",
            collateral_amount_to_liquidate,
        ),
        attr("debt_amount_repaid", debt_amount_to_repay),
        attr("refund_amount", refund_amount),
    ];

    // TODO: we should distinguish between collateral and market values in some way
    append_indices_and_rates_to_logs(&mut attributes, &debt_market);
    if !receive_ma_token {
        append_indices_and_rates_to_logs(&mut attributes, &collateral_market);
    }

    Ok(Response {
        data: None,
        attributes,
        messages,
        events: vec![],
    })
}

/// Computes debt to repay (in debt asset),
/// collateral to liquidate (in collateral asset) and
/// amount to refund the liquidator (in debt asset)
fn liquidation_compute_amounts(
    collateral_price: Decimal256,
    debt_price: Decimal256,
    close_factor: Decimal256,
    user_collateral_balance: Uint256,
    liquidation_bonus: Decimal256,
    user_debt_asset_total_debt: Uint256,
    sent_debt_asset_amount: Uint256,
) -> (Uint256, Uint256, Uint256) {
    // Debt: Only up to a fraction of the total debt (determined by the close factor) can be
    // repayed.
    let max_repayable_debt = close_factor * user_debt_asset_total_debt;

    let mut debt_amount_to_repay = if sent_debt_asset_amount > max_repayable_debt {
        max_repayable_debt
    } else {
        sent_debt_asset_amount
    };

    // Collateral: debt to repay in uusd times the liquidation
    // bonus
    let debt_amount_to_repay_in_uusd = debt_amount_to_repay * debt_price;
    let collateral_amount_to_liquidate_in_uusd =
        debt_amount_to_repay_in_uusd * (Decimal256::one() + liquidation_bonus);
    let mut collateral_amount_to_liquidate =
        collateral_amount_to_liquidate_in_uusd / collateral_price;

    // If collateral amount to liquidate is higher than user_collateral_balance,
    // liquidate the full balance and adjust the debt amount to repay accordingly
    if collateral_amount_to_liquidate > user_collateral_balance {
        collateral_amount_to_liquidate = user_collateral_balance;
        debt_amount_to_repay = collateral_price * collateral_amount_to_liquidate
            / debt_price
            / (Decimal256::one() + liquidation_bonus);
    }

    let refund_amount = sent_debt_asset_amount - debt_amount_to_repay;

    (
        debt_amount_to_repay,
        collateral_amount_to_liquidate,
        refund_amount,
    )
}

/// Update uncollateralized loan limit by a given amount in uusd
pub fn execute_finalize_liquidity_token_transfer(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    from_address: Addr,
    to_address: Addr,
    from_previous_balance: Uint128,
    to_previous_balance: Uint128,
    amount: Uint128,
) -> Result<Response, ContractError> {
    // Get liquidity token market
    let market_reference = MARKET_MA_TOKENS.load(deps.storage, &info.sender)?;
    let market = MARKETS.load(deps.storage, market_reference.as_slice())?;

    // Check user health factor is above 1
    // TODO: this assumes new balances are already in state as this call will be made
    // after a transfer call on an ma_asset. Double check this is the case when doing
    // integration tests. If it's not we would need to pass the updated balances to
    // the health factor somehow
    let money_market = RED_BANK.load(deps.storage)?;
    let mut from_user = USERS.load(deps.storage, &from_address)?;
    let (user_account_settlement, _) = prepare_user_account_settlement(
        &deps,
        env.block.time.seconds(),
        &from_address,
        &from_user,
        &money_market,
        &mut vec![],
    )?;
    if let UserHealthStatus::Borrowing(health_factor) = user_account_settlement.health_status {
        if health_factor < Decimal256::one() {
            return Err(StdError::generic_err("Cannot make token transfer if it results in a health factor lower than 1 for the sender").into());
        }
    }

    // Update users's positions
    // TODO: Should this and all collateral positions changes be logged? how?
    if from_address != to_address {
        if from_previous_balance.checked_sub(amount)? == Uint128::zero() {
            unset_bit(&mut from_user.collateral_assets, market.index)?;
            USERS.save(deps.storage, &from_address, &from_user)?;
        }

        if (to_previous_balance == Uint128::zero()) && (amount != Uint128::zero()) {
            let mut to_user = USERS
                .may_load(deps.storage, &to_address)?
                .unwrap_or_default();
            set_bit(&mut to_user.collateral_assets, market.index)?;
            USERS.save(deps.storage, &to_address, &to_user)?;
        }
    }

    Ok(Response::default())
}

/// Update uncollateralized loan limit by a given amount in uusd
pub fn execute_update_uncollateralized_loan_limit(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    user_address: Addr,
    asset: Asset,
    new_limit: Uint128,
) -> Result<Response, ContractError> {
    // Get config
    let config = CONFIG.load(deps.storage)?;

    // Only owner can do this
    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {}.into());
    }

    let (asset_label, asset_reference, _) = asset_get_attributes(&asset)?;

    UNCOLLATERALIZED_LOAN_LIMITS.save(
        deps.storage,
        (asset_reference.as_slice(), &user_address),
        &new_limit,
    )?;

    DEBTS.update(
        deps.storage,
        (asset_reference.as_slice(), &user_address),
        |debt_opt: Option<Debt>| -> StdResult<_> {
            let mut debt = debt_opt.unwrap_or(Debt {
                amount_scaled: Uint256::zero(),
                uncollateralized: false,
            });
            // if limit == 0 then uncollateralized = false, otherwise uncollateralized = true
            debt.uncollateralized = !new_limit.is_zero();
            Ok(debt)
        },
    )?;

    Ok(Response {
        messages: vec![],
        attributes: vec![
            attr("action", "update_uncollateralized_loan_limit"),
            attr("user", user_address.as_str()),
            attr("asset", asset_label),
            attr("new_allowance", new_limit),
        ],
        events: vec![],
        data: None,
    })
}

/// Update (enable / disable) collateral asset for specific user
pub fn execute_update_user_collateral_asset_status(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    asset: Asset,
    enable: bool,
) -> Result<Response, ContractError> {
    let user_address = info.sender;
    let mut user = USERS
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    let (collateral_asset_label, collateral_asset_reference, _) = asset_get_attributes(&asset)?;
    let collateral_market = MARKETS.load(deps.storage, collateral_asset_reference.as_slice())?;
    let has_collateral_asset = get_bit(user.collateral_assets, collateral_market.index)?;
    if !has_collateral_asset && enable {
        let collateral_ma_address = collateral_market.ma_token_address;
        let user_collateral_balance =
            cw20_get_balance(&deps.querier, collateral_ma_address, user_address.clone())?;
        if user_collateral_balance > Uint128::zero() {
            // enable collateral asset
            set_bit(&mut user.collateral_assets, collateral_market.index)?;
            USERS.save(deps.storage, &user_address, &user)?;
        } else {
            return Err(StdError::generic_err(format!(
                "User address {} has no balance in specified collateral asset {}",
                user_address.as_str(),
                collateral_asset_label
            ))
            .into());
        }
    } else if has_collateral_asset && !enable {
        // disable collateral asset
        unset_bit(&mut user.collateral_assets, collateral_market.index)?;
        USERS.save(deps.storage, &user_address, &user)?;
    }

    Ok(Response {
        messages: vec![],
        attributes: vec![
            attr("action", "update_user_collateral_asset_status"),
            attr("user", user_address.as_str()),
            attr("asset", collateral_asset_label),
            attr("has_collateral", has_collateral_asset),
            attr("enable", enable),
        ],
        events: vec![],
        data: None,
    })
}

/// Send accumulated asset income to protocol contracts
pub fn execute_distribute_protocol_income(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    asset: Asset,
    amount: Option<Uint256>,
) -> Result<Response, ContractError> {
    // Get config
    let config = CONFIG.load(deps.storage)?;

    let (asset_label, asset_reference, _) = asset_get_attributes(&asset)?;
    let mut market = MARKETS.load(deps.storage, asset_reference.as_slice())?;

    let amount_to_distribute = match amount {
        Some(amount) => amount,
        None => market.protocol_income_to_distribute,
    };

    if amount_to_distribute > market.protocol_income_to_distribute {
        return Err(StdError::generic_err(
            "amount specified exceeds market's income to be distributed",
        )
        .into());
    }

    market.protocol_income_to_distribute =
        market.protocol_income_to_distribute - amount_to_distribute;
    MARKETS.save(deps.storage, asset_reference.as_slice(), &market)?;

    let mut messages = vec![];

    let mars_contracts = vec![
        MarsContract::InsuranceFund,
        MarsContract::Staking,
        MarsContract::Treasury,
    ];
    let expected_len = mars_contracts.len();
    let mut addresses_query = address_provider::helpers::query_addresses(
        &deps.querier,
        config.address_provider_address,
        mars_contracts,
    )?;
    if addresses_query.len() != expected_len {
        return Err(StdError::generic_err(format!(
            "Incorrect number of addresses, expected {} got {}",
            expected_len,
            addresses_query.len()
        ))
        .into());
    }
    let treasury_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting addresses from provider"))?;
    let staking_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting addresses from provider"))?;
    let insurance_fund_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting addresses from provider"))?;

    let insurance_fund_amount = amount_to_distribute * config.insurance_fund_fee_share;
    let treasury_amount = amount_to_distribute * config.treasury_fee_share;
    let amount_to_distribute_before_staking_rewards = insurance_fund_amount + treasury_amount;
    if amount_to_distribute_before_staking_rewards > amount_to_distribute {
        return Err(StdError::generic_err(format!(
            "Decimal256 Underflow: will subtract {} from {} ",
            amount_to_distribute_before_staking_rewards, amount_to_distribute
        ))
        .into());
    }
    let staking_amount = amount_to_distribute - (amount_to_distribute_before_staking_rewards);

    // only build and add send message if fee is non-zero
    if !insurance_fund_amount.is_zero() {
        let insurance_fund_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address.clone(),
            insurance_fund_address,
            asset.clone(),
            insurance_fund_amount,
        )?;
        messages.push(SubMsg::new(insurance_fund_msg));
    }

    if !treasury_amount.is_zero() {
        let scaled_mint_amount =
            treasury_amount / get_updated_liquidity_index(&market, env.block.time.seconds());
        let treasury_fund_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: market.ma_token_address.into(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: treasury_address.into(),
                amount: scaled_mint_amount.into(),
            })?,
            funds: vec![],
        });
        messages.push(SubMsg::new(treasury_fund_msg));
    }

    if !staking_amount.is_zero() {
        let staking_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address,
            staking_address,
            asset,
            staking_amount,
        )?;
        messages.push(SubMsg::new(staking_msg));
    }

    Ok(Response {
        messages,
        attributes: vec![
            attr("action", "distribute_protocol_income"),
            attr("asset", asset_label),
            attr("amount", amount_to_distribute),
        ],
        data: None,
        events: vec![],
    })
}

// QUERIES

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Market { asset } => to_binary(&query_market(deps, asset)?),
        QueryMsg::MarketsList {} => to_binary(&query_markets_list(deps)?),
        QueryMsg::Debt { address } => {
            let address = deps.api.addr_validate(&address)?;
            to_binary(&query_debt(deps, address)?)
        }
        QueryMsg::Collateral { address } => {
            let address = deps.api.addr_validate(&address)?;
            to_binary(&query_collateral(deps, address)?)
        }
        QueryMsg::UncollateralizedLoanLimit {
            user_address,
            asset,
        } => {
            let user_address = deps.api.addr_validate(&user_address)?;
            to_binary(&query_uncollateralized_loan_limit(
                deps,
                user_address,
                asset,
            )?)
        }
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    let money_market = RED_BANK.load(deps.storage)?;

    Ok(ConfigResponse {
        owner: config.owner,
        address_provider_address: config.address_provider_address,
        insurance_fund_fee_share: config.insurance_fund_fee_share,
        treasury_fee_share: config.treasury_fee_share,
        ma_token_code_id: config.ma_token_code_id,
        market_count: money_market.market_count,
        close_factor: config.close_factor,
    })
}

fn query_market(deps: Deps, asset: Asset) -> StdResult<MarketResponse> {
    let market = match asset {
        Asset::Native { denom } => match MARKETS.load(deps.storage, denom.as_bytes()) {
            Ok(market) => market,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "failed to load market for: {}",
                    denom
                )))
            }
        },
        Asset::Cw20 { contract_addr } => {
            let contract_addr = deps.api.addr_validate(&contract_addr)?;
            match MARKETS.load(deps.storage, contract_addr.as_bytes()) {
                Ok(market) => market,
                Err(_) => {
                    return Err(StdError::generic_err(format!(
                        "failed to load market for: {}",
                        contract_addr
                    )))
                }
            }
        }
    };

    Ok(MarketResponse {
        ma_token_address: market.ma_token_address,
        borrow_index: market.borrow_index,
        liquidity_index: market.liquidity_index,
        borrow_rate: market.borrow_rate,
        liquidity_rate: market.liquidity_rate,
        max_loan_to_value: market.max_loan_to_value,
        interests_last_updated: market.interests_last_updated,
        debt_total_scaled: market.debt_total_scaled,
        asset_type: market.asset_type,
        maintenance_margin: market.maintenance_margin,
        liquidation_bonus: market.liquidation_bonus,
    })
}

fn query_markets_list(deps: Deps) -> StdResult<MarketsListResponse> {
    let markets_list: StdResult<Vec<_>> = MARKETS
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = get_market_denom(deps, k, v.asset_type)?;

            Ok(MarketInfo {
                denom,
                ma_token_address: v.ma_token_address,
            })
        })
        .collect();

    Ok(MarketsListResponse {
        markets_list: markets_list?,
    })
}

fn query_debt(deps: Deps, address: Addr) -> StdResult<DebtResponse> {
    let user = USERS.may_load(deps.storage, &address)?.unwrap_or_default();

    let debts: StdResult<Vec<_>> = MARKETS
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = get_market_denom(deps, k.clone(), v.asset_type)?;

            let is_borrowing_asset = get_bit(user.borrowed_assets, v.index)?;
            if is_borrowing_asset {
                let debt = DEBTS.load(deps.storage, (k.as_slice(), &address))?;
                Ok(DebtInfo {
                    denom,
                    amount: debt.amount_scaled,
                })
            } else {
                Ok(DebtInfo {
                    denom,
                    amount: Uint256::zero(),
                })
            }
        })
        .collect();

    Ok(DebtResponse { debts: debts? })
}

fn query_collateral(deps: Deps, address: Addr) -> StdResult<CollateralResponse> {
    let user = USERS.may_load(deps.storage, &address)?.unwrap_or_default();

    let collateral: StdResult<Vec<_>> = MARKETS
        .range(deps.storage, None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = get_market_denom(deps, k, v.asset_type)?;

            Ok(CollateralInfo {
                denom,
                enabled: get_bit(user.collateral_assets, v.index)?,
            })
        })
        .collect();

    Ok(CollateralResponse {
        collateral: collateral?,
    })
}

fn query_uncollateralized_loan_limit(
    deps: Deps,
    user_address: Addr,
    asset: Asset,
) -> StdResult<UncollateralizedLoanLimitResponse> {
    let (asset_label, asset_reference, _) = asset_get_attributes(&asset)?;
    let uncollateralized_loan_limit = UNCOLLATERALIZED_LOAN_LIMITS
        .load(deps.storage, (asset_reference.as_slice(), &user_address));

    match uncollateralized_loan_limit {
        Ok(limit) => Ok(UncollateralizedLoanLimitResponse { limit }),
        Err(_) => Err(StdError::not_found(format!(
            "No uncollateralized loan approved for user_address: {} on asset: {}",
            user_address, asset_label
        ))),
    }
}

#[entry_point]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}

// INTEREST

/// Updates market indices and protocol_income by applying current interest rates on the time between
/// last interest update and current block.
/// Note it does not save the market to the store (that is left to the caller)
pub fn market_apply_accumulated_interests(env: &Env, market: &mut Market) {
    let current_timestamp = env.block.time.seconds();
    // Since interest is updated on every change on scale debt, multiplying the scaled debt for each
    // of the indices and subtracting them returns the accrued borrow interest for the period since
    // when the indices were last updated and the current point in time.
    let previous_borrow_index = market.borrow_index;

    if market.interests_last_updated < current_timestamp {
        let time_elapsed = current_timestamp - market.interests_last_updated;

        if market.borrow_rate > Decimal256::zero() {
            market.borrow_index = calculate_applied_linear_interest_rate(
                market.borrow_index,
                market.borrow_rate,
                time_elapsed,
            );
        }
        if market.liquidity_rate > Decimal256::zero() {
            market.liquidity_index = calculate_applied_linear_interest_rate(
                market.liquidity_index,
                market.liquidity_rate,
                time_elapsed,
            );
        }
        market.interests_last_updated = current_timestamp;
    }

    let previous_debt_total = market.debt_total_scaled * previous_borrow_index;
    let new_debt_total = market.debt_total_scaled * market.borrow_index;

    let interest_accrued = if new_debt_total > previous_debt_total {
        new_debt_total - previous_debt_total
    } else {
        Uint256::zero()
    };

    let new_protocol_income_to_distribute = interest_accrued * market.reserve_factor;
    market.protocol_income_to_distribute += new_protocol_income_to_distribute;
}

/// Return applied interest rate for borrow index according to passed blocks
/// NOTE: Calling this function when interests for the market are up to date with the current block
/// and index is not, will use the wrong interest rate to update the index.
fn get_updated_borrow_index(market: &Market, block_time: u64) -> Decimal256 {
    if market.interests_last_updated < block_time {
        let time_elapsed = block_time - market.interests_last_updated;

        if market.borrow_rate > Decimal256::zero() {
            let applied_interest_rate = calculate_applied_linear_interest_rate(
                market.borrow_index,
                market.borrow_rate,
                time_elapsed,
            );
            return applied_interest_rate;
        }
    }

    market.borrow_index
}

/// Return applied interest rate for liquidity index according to passed blocks
/// NOTE: Calling this function when interests for the market are up to date with the current block
/// and index is not, will use the wrong interest rate to update the index.
fn get_updated_liquidity_index(market: &Market, block_time: u64) -> Decimal256 {
    if market.interests_last_updated < block_time {
        let time_elapsed = block_time - market.interests_last_updated;

        if market.liquidity_rate > Decimal256::zero() {
            let applied_interest_rate = calculate_applied_linear_interest_rate(
                market.liquidity_index,
                market.liquidity_rate,
                time_elapsed,
            );
            return applied_interest_rate;
        }
    }

    market.liquidity_index
}

fn calculate_applied_linear_interest_rate(
    index: Decimal256,
    rate: Decimal256,
    time_elapsed: u64,
) -> Decimal256 {
    let rate_factor =
        rate * Decimal256::from_uint256(time_elapsed) / Decimal256::from_uint256(SECONDS_PER_YEAR);
    index * (Decimal256::one() + rate_factor)
}

/// Update interest rates for current liquidity and debt levels
/// Note it does not save the market to the store (that is left to the caller)
pub fn market_update_interest_rates(
    deps: &DepsMut,
    env: &Env,
    reference: &[u8],
    market: &mut Market,
    liquidity_taken: Uint256,
) -> StdResult<()> {
    let contract_balance_amount = match market.asset_type {
        AssetType::Native => {
            let denom = str::from_utf8(reference);
            let denom = match denom {
                Ok(denom) => denom,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            };
            deps.querier
                .query_balance(env.contract.address.clone(), denom)?
                .amount
        }
        AssetType::Cw20 => {
            let cw20_addr = str::from_utf8(reference);
            let cw20_addr = match cw20_addr {
                Ok(cw20_addr) => cw20_addr,
                Err(_) => {
                    return Err(StdError::generic_err(
                        "failed to encode Cw20 address into string",
                    ))
                }
            };
            let cw20_addr = deps.api.addr_validate(cw20_addr)?;
            cw20_get_balance(&deps.querier, cw20_addr, env.contract.address.clone())?
        }
    };

    // TODO: Verify on integration tests that this balance includes the
    // amount sent by the user on deposits and repays(both for cw20 and native).
    // If it doesn't, we should include them on the available_liquidity
    let contract_current_balance = Uint256::from(contract_balance_amount);

    // Get protocol income to be deducted from liquidity (doesn't belong to the money market
    // anymore)
    let config = CONFIG.load(deps.storage)?;
    // NOTE: No check for underflow because this is done on config validations
    let protocol_income_minus_treasury_amount =
        (Decimal256::one() - config.treasury_fee_share) * market.protocol_income_to_distribute;
    let liquidity_to_deduct_from_current_balance =
        liquidity_taken + protocol_income_minus_treasury_amount;

    if contract_current_balance < liquidity_to_deduct_from_current_balance {
        return Err(StdError::generic_err(
            "Protocol income to be distributed and liquidity taken cannot be greater than available liquidity",
        ));
    }

    let available_liquidity = Decimal256::from_uint256(
        contract_current_balance - liquidity_to_deduct_from_current_balance,
    );
    let total_debt = Decimal256::from_uint256(market.debt_total_scaled)
        * get_updated_borrow_index(market, env.block.time.seconds());
    let current_utilization_rate = if total_debt > Decimal256::zero() {
        total_debt / (available_liquidity + total_debt)
    } else {
        Decimal256::zero()
    };

    let (new_borrow_rate, new_liquidity_rate) =
        get_updated_interest_rates(market, current_utilization_rate);
    market.borrow_rate = new_borrow_rate;
    market.liquidity_rate = new_liquidity_rate;

    Ok(())
}

/// Updates borrow and liquidity rates based on PID parameters
fn get_updated_interest_rates(
    market: &Market,
    current_utilization_rate: Decimal256,
) -> (Decimal256, Decimal256) {
    // Use PID params for calculating borrow interest rate
    let pid_params = market.pid_parameters.clone();

    // error_value should be represented as integer number so we do this with help from boolean flag
    let (error_value, error_positive) =
        if pid_params.optimal_utilization_rate > current_utilization_rate {
            (
                pid_params.optimal_utilization_rate - current_utilization_rate,
                true,
            )
        } else {
            (
                current_utilization_rate - pid_params.optimal_utilization_rate,
                false,
            )
        };

    let kp = if error_value >= pid_params.kp_augmentation_threshold {
        pid_params.kp_2
    } else {
        pid_params.kp_1
    };

    let p = kp * error_value;
    let mut new_borrow_rate = if error_positive {
        // error_positive = true (u_optimal > u) means we want utilization rate to go up
        // we lower interest rate so more people borrow
        if market.borrow_rate > p {
            market.borrow_rate - p
        } else {
            Decimal256::zero()
        }
    } else {
        // error_positive = false (u_optimal < u) means we want utilization rate to go down
        // we increase interest rate so less people borrow
        market.borrow_rate + p
    };

    // Check borrow rate conditions
    if new_borrow_rate < market.min_borrow_rate {
        new_borrow_rate = market.min_borrow_rate
    } else if new_borrow_rate > market.max_borrow_rate {
        new_borrow_rate = market.max_borrow_rate;
    };

    // This operation should not underflow as reserve_factor is checked to be <= 1
    let new_liquidity_rate =
        new_borrow_rate * current_utilization_rate * (Decimal256::one() - market.reserve_factor);

    (new_borrow_rate, new_liquidity_rate)
}

fn append_indices_and_rates_to_logs(logs: &mut Vec<Attribute>, market: &Market) {
    let mut interest_logs = vec![
        attr("borrow_index", market.borrow_index),
        attr("liquidity_index", market.liquidity_index),
        attr("borrow_rate", market.borrow_rate),
        attr("liquidity_rate", market.liquidity_rate),
    ];
    logs.append(&mut interest_logs);
}

/// User asset settlement
struct UserAssetSettlement {
    asset_label: String,
    asset_type: AssetType,
    collateral_amount: Uint256,
    debt_amount: Uint256,
    uncollateralized_debt: bool,
    ltv: Decimal256,
    maintenance_margin: Decimal256,
}

/// Goes through assets user has a position in and returns a vec containing the scaled debt
/// (denominated in the asset), a result from a specified computation for the current collateral
/// (denominated in asset) and some metadata to be used by the caller.
/// Also adds the price to native_assets_prices_to_query in case the prices in uusd need to
/// be retrieved by the caller later
fn user_get_balances(
    deps: Deps,
    money_market: &RedBank,
    user: &User,
    user_address: &Addr,
    native_asset_prices_to_query: &mut Vec<String>,
    block_time: u64,
) -> StdResult<Vec<UserAssetSettlement>> {
    let mut ret: Vec<UserAssetSettlement> = vec![];

    for i in 0_u32..money_market.market_count {
        let user_is_using_as_collateral = get_bit(user.collateral_assets, i)?;
        let user_is_borrowing = get_bit(user.borrowed_assets, i)?;
        if !(user_is_using_as_collateral || user_is_borrowing) {
            continue;
        }

        let (asset_reference_vec, market) = market_get_from_index(&deps, i)?;

        let (collateral_amount, ltv, maintenance_margin) = if user_is_using_as_collateral {
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

        // get asset label
        let asset_label = match market.asset_type {
            AssetType::Native => match String::from_utf8(asset_reference_vec) {
                Ok(res) => res,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            },
            AssetType::Cw20 => match String::from_utf8(asset_reference_vec) {
                Ok(res) => res,
                Err(_) => {
                    return Err(StdError::generic_err(
                        "failed to encode Cw20 address into string",
                    ))
                }
            },
        };

        // Add price to query list
        if market.asset_type == AssetType::Native && asset_label != "uusd" {
            native_asset_prices_to_query.push(asset_label.clone());
        }

        let user_asset_settlement = UserAssetSettlement {
            asset_label,
            asset_type: market.asset_type,
            collateral_amount,
            debt_amount,
            uncollateralized_debt,
            ltv,
            maintenance_margin,
        };
        ret.push(user_asset_settlement);
    }

    Ok(ret)
}

/// User account settlement
struct UserAccountSettlement {
    /// NOTE: Not used yet
    _total_collateral_in_uusd: Uint256,
    total_debt_in_uusd: Uint256,
    total_collateralized_debt_in_uusd: Uint256,
    max_debt_in_uusd: Uint256,
    weighted_maintenance_margin_in_uusd: Uint256,
    health_status: UserHealthStatus,
}

enum UserHealthStatus {
    NotBorrowing,
    Borrowing(Decimal256),
}

/// Calculates the user data across the markets.
/// This includes the total debt/collateral balances in uusd,
/// the average LTV, the average Liquidation threshold, and the Health factor.
/// Moreover returns the list of asset prices that were used during the computation.
fn prepare_user_account_settlement(
    deps: &DepsMut,
    block_time: u64,
    user_address: &Addr,
    user: &User,
    money_market: &RedBank,
    native_asset_prices_to_query: &mut Vec<String>,
) -> StdResult<(UserAccountSettlement, Vec<(String, Decimal256)>)> {
    let user_balances = user_get_balances(
        deps.as_ref(),
        money_market,
        user,
        user_address,
        native_asset_prices_to_query,
        block_time,
    )?;
    let native_asset_prices = get_native_asset_prices(&deps.querier, native_asset_prices_to_query)?;

    let mut total_collateral_in_uusd = Uint256::zero();
    let mut total_debt_in_uusd = Uint256::zero();
    let mut total_collateralized_debt_in_uusd = Uint256::zero();
    let mut weighted_ltv_in_uusd = Uint256::zero();
    let mut weighted_maintenance_margin_in_uusd = Uint256::zero();

    for user_asset_settlement in user_balances {
        let asset_price = asset_get_price(
            user_asset_settlement.asset_label.as_str(),
            &native_asset_prices,
            &user_asset_settlement.asset_type,
        )?;

        let collateral_in_uusd = user_asset_settlement.collateral_amount * asset_price;
        total_collateral_in_uusd += collateral_in_uusd;

        weighted_ltv_in_uusd += collateral_in_uusd * user_asset_settlement.ltv;
        weighted_maintenance_margin_in_uusd +=
            collateral_in_uusd * user_asset_settlement.maintenance_margin;

        let debt_in_uusd = user_asset_settlement.debt_amount * asset_price;
        total_debt_in_uusd += debt_in_uusd;

        if !user_asset_settlement.uncollateralized_debt {
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

    let use_account_settlement = UserAccountSettlement {
        _total_collateral_in_uusd: total_collateral_in_uusd,
        total_debt_in_uusd,
        total_collateralized_debt_in_uusd,
        max_debt_in_uusd: weighted_ltv_in_uusd,
        weighted_maintenance_margin_in_uusd,
        health_status,
    };

    Ok((use_account_settlement, native_asset_prices))
}

// HELPERS

// native coins
fn get_denom_amount_from_coins(coins: &[Coin], denom: &str) -> Uint256 {
    coins
        .iter()
        .find(|c| c.denom == denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero)
}

fn get_market_denom(deps: Deps, market_id: Vec<u8>, asset_type: AssetType) -> StdResult<String> {
    match asset_type {
        AssetType::Native => match String::from_utf8(market_id) {
            Ok(denom) => Ok(denom),
            Err(_) => Err(StdError::generic_err("failed to encode key into string")),
        },
        AssetType::Cw20 => {
            let cw20_contract_address = match String::from_utf8(market_id) {
                Ok(cw20_contract_address) => cw20_contract_address,
                Err(_) => {
                    return Err(StdError::generic_err(
                        "failed to encode key into contract address",
                    ))
                }
            };
            let cw20_contract_address = deps.api.addr_validate(&cw20_contract_address)?;
            match cw20_get_symbol(&deps.querier, cw20_contract_address.clone()) {
                Ok(symbol) => Ok(symbol),
                Err(_) => {
                    return Err(StdError::generic_err(format!(
                        "failed to get symbol from cw20 contract address: {}",
                        cw20_contract_address
                    )));
                }
            }
        }
    }
}

// bitwise operations
/// Gets bit: true: 1, false: 0
fn get_bit(bitmap: Uint128, index: u32) -> StdResult<bool> {
    if index >= 128 {
        return Err(StdError::generic_err("index out of range"));
    }
    Ok(((bitmap.u128() >> index) & 1) == 1)
}

/// Sets bit to 1
fn set_bit(bitmap: &mut Uint128, index: u32) -> StdResult<()> {
    if index >= 128 {
        return Err(StdError::generic_err("index out of range"));
    }
    *bitmap = Uint128::from(bitmap.u128() | (1 << index));
    Ok(())
}

/// Sets bit to 0
fn unset_bit(bitmap: &mut Uint128, index: u32) -> StdResult<()> {
    if index >= 128 {
        return Err(StdError::generic_err("index out of range"));
    }
    *bitmap = Uint128::from(bitmap.u128() & !(1 << index));
    Ok(())
}

fn build_send_asset_msg(
    deps: Deps,
    sender_address: Addr,
    recipient_address: Addr,
    asset: Asset,
    amount: Uint256,
) -> StdResult<CosmosMsg> {
    match asset {
        Asset::Native { denom } => Ok(build_send_native_asset_msg(
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
fn build_send_native_asset_msg(
    deps: Deps,
    _sender: Addr,
    recipient: Addr,
    denom: &str,
    amount: Uint256,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Bank(BankMsg::Send {
        to_address: recipient.into(),
        amount: vec![deduct_tax(
            deps,
            Coin {
                denom: denom.to_string(),
                amount: amount.into(),
            },
        )?],
    }))
}

fn build_send_cw20_token_msg(
    recipient: Addr,
    token_contract_address: Addr,
    amount: Uint256,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: token_contract_address.into(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: recipient.into(),
            amount: amount.into(),
        })?,
        funds: vec![],
    }))
}

fn get_native_asset_prices(
    querier: &QuerierWrapper,
    assets_to_query: &[String],
) -> StdResult<Vec<(String, Decimal256)>> {
    let mut asset_prices: Vec<(String, Decimal256)> = vec![];

    if !assets_to_query.is_empty() {
        let assets_to_query: Vec<&str> = assets_to_query.iter().map(AsRef::as_ref).collect(); // type conversion
        let querier = TerraQuerier::new(querier);
        let asset_prices_in_uusd = querier
            .query_exchange_rates("uusd", assets_to_query)?
            .exchange_rates;
        for rate in asset_prices_in_uusd {
            asset_prices.push((rate.quote_denom, Decimal256::from(rate.exchange_rate)));
        }
    }

    Ok(asset_prices)
}

fn asset_get_price(
    asset_label: &str,
    asset_prices: &[(String, Decimal256)],
    asset_type: &AssetType,
) -> StdResult<Decimal256> {
    if asset_label == "uusd" || *asset_type == AssetType::Cw20 {
        return Ok(Decimal256::one());
    }

    let asset_price = match asset_prices
        .iter()
        .find(|asset| asset.0 == asset_label)
        .map(|correct_asset| correct_asset.1)
    {
        Some(price) => price,
        None => {
            return Err(StdError::generic_err(format!(
                "asset price for {} not found",
                asset_label
            )))
        }
    };

    Ok(asset_price)
}


fn market_get_from_index(deps: &Deps, index: u32) -> StdResult<(Vec<u8>, Market)> {
    let asset_reference_vec = match MARKET_REFERENCES.load(deps.storage, U32Key::new(index)) {
        Ok(asset_reference_vec) => asset_reference_vec,
        Err(_) => {
            return Err(StdError::generic_err(format!(
                "no market reference exists with index: {}",
                index
            )))
        }
    }
    .reference;
    match MARKETS.load(deps.storage, asset_reference_vec.as_slice()) {
        Ok(asset_market) => Ok((asset_reference_vec, asset_market)),
        Err(_) => Err(StdError::generic_err(format!(
            "no asset market exists with asset reference: {}",
            String::from_utf8(asset_reference_vec).expect("Found invalid UTF-8")
        ))),
    }
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PidParameters;
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{coin, from_binary, Decimal, OwnedDeps};
    use mars::red_bank::msg::ExecuteMsg::UpdateConfig;
    use mars::testing::{
        assert_generic_error_message, mock_dependencies, mock_env, mock_env_at_block_time,
        mock_info, MarsMockQuerier, MockEnvParams,
    };

    #[test]
    fn test_accumulated_index_calculation() {
        let index = Decimal256::from_ratio(1, 10);
        let rate = Decimal256::from_ratio(2, 10);
        let time_elapsed = 15768000; // half a year
        let accumulated = calculate_applied_linear_interest_rate(index, rate, time_elapsed);

        assert_eq!(accumulated, Decimal256::from_ratio(11, 100));
    }

    #[test]
    fn test_pid_interest_rates_calculation() {
        let market = Market {
            // Params used for new rates calculations
            borrow_rate: Decimal256::from_ratio(5, 100),
            min_borrow_rate: Decimal256::from_ratio(1, 100),
            max_borrow_rate: Decimal256::from_ratio(90, 100),
            pid_parameters: PidParameters {
                kp_1: Decimal256::from_ratio(2, 1),
                optimal_utilization_rate: Decimal256::from_ratio(60, 100),
                kp_augmentation_threshold: Decimal256::from_ratio(10, 100),
                kp_2: Decimal256::from_ratio(3, 1),
            },

            // Rest params are not used
            index: 0,
            ma_token_address: zero_address(),
            borrow_index: Default::default(),
            liquidity_index: Default::default(),
            liquidity_rate: Default::default(),
            max_loan_to_value: Default::default(),
            reserve_factor: Default::default(),
            interests_last_updated: 0,
            debt_total_scaled: Default::default(),
            asset_type: AssetType::Cw20,
            maintenance_margin: Default::default(),
            liquidation_bonus: Default::default(),
            protocol_income_to_distribute: Default::default(),
        };

        // *
        // current utilization rate > optimal utilization rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(61, 100);
        let (new_borrow_rate, new_liquidity_rate) =
            get_updated_interest_rates(&market, current_utilization_rate);

        let expected_error =
            current_utilization_rate - market.pid_parameters.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate =
            market.borrow_rate + (market.pid_parameters.kp_1 * expected_error);
        let expected_liquidity_rate = expected_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(59, 100);
        let (new_borrow_rate, new_liquidity_rate) =
            get_updated_interest_rates(&market, current_utilization_rate);

        let expected_error =
            market.pid_parameters.optimal_utilization_rate - current_utilization_rate;
        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate =
            market.borrow_rate - (market.pid_parameters.kp_1 * expected_error);
        let expected_liquidity_rate = expected_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, increment KP by a multiplier if error goes beyond threshold
        // *
        let current_utilization_rate = Decimal256::from_ratio(72, 100);
        let (new_borrow_rate, new_liquidity_rate) =
            get_updated_interest_rates(&market, current_utilization_rate);

        let expected_error =
            current_utilization_rate - market.pid_parameters.optimal_utilization_rate;
        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate =
            market.borrow_rate + (market.pid_parameters.kp_2 * expected_error);
        let expected_liquidity_rate = expected_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate < optimal utilization rate, borrow rate can't be less than min borrow rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(10, 100);
        let (new_borrow_rate, new_liquidity_rate) =
            get_updated_interest_rates(&market, current_utilization_rate);

        // we want to decrease borrow rate to increase utilization rate
        let expected_borrow_rate = market.min_borrow_rate;
        let expected_liquidity_rate = expected_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);

        // *
        // current utilization rate > optimal utilization rate, borrow rate can't be less than max borrow rate
        // *
        let current_utilization_rate = Decimal256::from_ratio(90, 100);
        let (new_borrow_rate, new_liquidity_rate) =
            get_updated_interest_rates(&market, current_utilization_rate);

        // we want to increase borrow rate to decrease utilization rate
        let expected_borrow_rate = market.max_borrow_rate;
        let expected_liquidity_rate = expected_borrow_rate
            * current_utilization_rate
            * (Decimal256::one() - market.reserve_factor);

        assert_eq!(new_borrow_rate, expected_borrow_rate);
        assert_eq!(new_liquidity_rate, expected_liquidity_rate);
    }

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        // Config with base params valid (just update the rest)
        let base_config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            ma_token_code_id: Some(10u64),
            insurance_fund_fee_share: None,
            treasury_fee_share: None,
            close_factor: None,
        };

        // *
        // init config with empty params
        // *
        let empty_config = CreateOrUpdateConfig {
            owner: None,
            address_provider_address: None,
            insurance_fund_fee_share: None,
            treasury_fee_share: None,
            ma_token_code_id: None,
            close_factor: None,
        };
        let msg = InstantiateMsg {
            config: empty_config,
        };
        let info = mock_info("owner");
        let response = instantiate(deps.as_mut(), env.clone(), info, msg);
        assert_generic_error_message(
            response,
            "All params should be available during initialization",
        );

        // *
        // init config with close_factor, insurance_fund_fee_share, treasury_fee_share greater than 1
        // *
        let mut insurance_fund_fee_share = Decimal256::from_ratio(11, 10);
        let mut treasury_fee_share = Decimal256::from_ratio(12, 10);
        let mut close_factor = Decimal256::from_ratio(13, 10);
        let config = CreateOrUpdateConfig {
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
            ..base_config.clone()
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        let response = instantiate(deps.as_mut(), env.clone(), info, msg);
        assert_generic_error_message(response, "[close_factor, insurance_fund_fee_share, treasury_fee_share] should be less or equal 1. \
                Invalid params: [close_factor, insurance_fund_fee_share, treasury_fee_share]");

        // *
        // init config with invalid fee share amounts
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(7, 10);
        treasury_fee_share = Decimal256::from_ratio(4, 10);
        close_factor = Decimal256::from_ratio(1, 2);
        let config = CreateOrUpdateConfig {
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
            ..base_config.clone()
        };
        let exceeding_fees_msg = InstantiateMsg { config };
        let info = mock_info("owner");
        let response = instantiate(deps.as_mut(), env.clone(), info, exceeding_fees_msg);
        assert_generic_error_message(
            response,
            "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one",
        );

        // *
        // init config with valid params
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(5, 10);
        treasury_fee_share = Decimal256::from_ratio(3, 10);
        close_factor = Decimal256::from_ratio(1, 2);
        let config = CreateOrUpdateConfig {
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
            ..base_config
        };
        let msg = InstantiateMsg { config };

        // we can just call .unwrap() to assert this was a success
        let info = mock_info("owner");
        let res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(10, value.ma_token_code_id);
        assert_eq!(0, value.market_count);
        assert_eq!(insurance_fund_fee_share, value.insurance_fund_fee_share);
        assert_eq!(treasury_fee_share, value.treasury_fee_share);
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        // *
        // init config with valid params
        // *
        let mut insurance_fund_fee_share = Decimal256::from_ratio(1, 10);
        let mut treasury_fee_share = Decimal256::from_ratio(3, 10);
        let mut close_factor = Decimal256::from_ratio(1, 4);
        let init_config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            ma_token_code_id: Some(20u64),
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
        };
        let msg = InstantiateMsg {
            config: init_config.clone(),
        };
        // we can just call .unwrap() to assert this was a success
        let info = mock_info("owner");
        let _res = instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            config: init_config.clone(),
        };
        let info = mock_info("somebody");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());

        // *
        // update config with close_factor, insurance_fund_fee_share, treasury_fee_share greater than 1
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(11, 10);
        treasury_fee_share = Decimal256::from_ratio(12, 10);
        close_factor = Decimal256::from_ratio(13, 10);
        let config = CreateOrUpdateConfig {
            owner: None,
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
            ..init_config.clone()
        };
        let msg = UpdateConfig { config };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("[close_factor, insurance_fund_fee_share, treasury_fee_share] should be less or equal 1. \
                Invalid params: [close_factor, insurance_fund_fee_share, treasury_fee_share]").into());

        // *
        // update config with invalid fee share amounts
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(10, 10);
        let config = CreateOrUpdateConfig {
            owner: None,
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: None,
            ..init_config
        };
        let exceeding_fees_msg = UpdateConfig { config };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, exceeding_fees_msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one"
            )
            .into()
        );

        // *
        // update config with all new params
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(5, 100);
        treasury_fee_share = Decimal256::from_ratio(3, 100);
        close_factor = Decimal256::from_ratio(1, 20);
        let config = CreateOrUpdateConfig {
            owner: Some("new_owner".to_string()),
            address_provider_address: Some("new_address_provider".to_string()),
            ma_token_code_id: Some(40u64),
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
        };
        let msg = UpdateConfig {
            config: config.clone(),
        };

        // we can just call .unwrap() to assert this was a success
        let info = mock_info("owner");
        let res = execute(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = CONFIG.load(&deps.storage).unwrap();

        assert_eq!(new_config.owner, Addr::unchecked("new_owner"));
        assert_eq!(
            new_config.address_provider_address,
            Addr::unchecked(config.address_provider_address.unwrap())
        );
        assert_eq!(
            new_config.ma_token_code_id,
            config.ma_token_code_id.unwrap()
        );
        assert_eq!(
            new_config.insurance_fund_fee_share,
            config.insurance_fund_fee_share.unwrap()
        );
        assert_eq!(
            new_config.treasury_fee_share,
            config.treasury_fee_share.unwrap()
        );
        assert_eq!(new_config.close_factor, config.close_factor.unwrap());
    }

    #[test]
    fn test_init_asset() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal256::from_ratio(20, 100)),
            min_borrow_rate: Some(Decimal256::from_ratio(5, 100)),
            max_borrow_rate: Some(Decimal256::from_ratio(50, 100)),
            max_loan_to_value: Some(Decimal256::from_ratio(8, 10)),
            reserve_factor: Some(Decimal256::from_ratio(1, 100)),
            maintenance_margin: Some(Decimal256::one()),
            liquidation_bonus: Some(Decimal256::zero()),
            kp_1: Some(Decimal256::from_ratio(3, 1)),
            optimal_utilization_rate: Some(Decimal256::from_ratio(80, 100)),
            kp_augmentation_threshold: Some(Decimal256::from_ratio(2000, 1)),
            kp_2: Some(Decimal256::from_ratio(2, 1)),
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("somebody");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());

        // *
        // init asset with empty params
        // *
        let empty_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: None,
            maintenance_margin: None,
            liquidation_bonus: None,
            ..asset_params
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("All params should be available during initialization").into()
        );

        // *
        // init asset with some params greater than 1
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(110, 10)),
            reserve_factor: Some(Decimal256::from_ratio(120, 100)),
            ..asset_params
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("[max_loan_to_value, reserve_factor, maintenance_margin, liquidation_bonus] should be less or equal 1. \
                Invalid params: [max_loan_to_value, reserve_factor]").into());

        // *
        // init asset where LTV >= liquidity threshold
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(5, 10)),
            maintenance_margin: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "maintenance_margin should be greater than max_loan_to_value. \
                    maintenance_margin: 0.5, \
                    max_loan_to_value: 0.5",
            )
            .into()
        );

        // *
        // init asset where min borrow rate >= max borrow rate
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            min_borrow_rate: Some(Decimal256::from_ratio(5, 10)),
            max_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("max_borrow_rate should be greater than min_borrow_rate. max_borrow_rate: 0.4, min_borrow_rate: 0.5").into());

        // *
        // init asset where optimal utilization rate > 1
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            optimal_utilization_rate: Some(Decimal256::from_ratio(11, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Optimal utilization rate can't be greater than one").into()
        );

        // *
        // owner is authorized
        // *
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("owner");
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // should have asset market with Canonical default address
        let market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(zero_address(), market.ma_token_address);
        // should have 0 index
        assert_eq!(0, market.index);
        // should have asset_type Native
        assert_eq!(AssetType::Native, market.asset_type);

        // should store reference in market index
        let market_reference = MARKET_REFERENCES
            .load(&deps.storage, U32Key::new(0))
            .unwrap();
        assert_eq!(b"someasset", market_reference.reference.as_slice());

        // Should have market count of 1
        let money_market = RED_BANK.load(&deps.storage).unwrap();
        assert_eq!(money_market.market_count, 1);

        // should instantiate a liquidity token
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Instantiate {
                admin: None,
                code_id: 5u64,
                msg: to_binary(&ma_token::msg::InstantiateMsg {
                    name: String::from("mars someasset liquidity token"),
                    symbol: String::from("masomeasset"),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: MOCK_CONTRACT_ADDR.to_string(),
                        cap: None,
                    }),
                    red_bank_address: MOCK_CONTRACT_ADDR.to_string(),
                    incentives_address: "incentives".to_string(),
                })
                .unwrap(),
                funds: vec![],
                label: "".to_string()
            })),]
        );

        assert_eq!(
            res.attributes,
            vec![attr("action", "init_asset"), attr("asset", "someasset"),],
        );

        // *
        // can't init more than once
        // *
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Asset already initialized").into()
        );

        // *
        // callback comes back with created token
        // *
        let msg = ExecuteMsg::InitAssetTokenCallback {
            reference: "someasset".into(),
        };
        let info = mock_info("mtokencontract");
        execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // should have asset market with contract address
        let market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(Addr::unchecked("mtokencontract"), market.ma_token_address);
        assert_eq!(Decimal256::one(), market.liquidity_index);

        // *
        // calling this again should not be allowed
        // *
        let msg = ExecuteMsg::InitAssetTokenCallback {
            reference: "someasset".into(),
        };
        let info = mock_info("mtokencontract");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());

        // *
        // Initialize a cw20 asset
        // *
        let cw20_addr = Addr::unchecked("otherasset");
        deps.querier
            .set_cw20_symbol(cw20_addr.clone(), "otherasset".to_string());
        let info = mock_info("owner");

        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Cw20 {
                contract_addr: cw20_addr.to_string(),
            },
            asset_params,
        };
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        let market = MARKETS.load(&deps.storage, cw20_addr.as_bytes()).unwrap();
        // should have asset market with Canonical default address
        assert_eq!(zero_address(), market.ma_token_address);
        // should have index 1
        assert_eq!(1, market.index);
        // should have asset_type Cw20
        assert_eq!(AssetType::Cw20, market.asset_type);

        // should store reference in market index
        let market_reference = MARKET_REFERENCES
            .load(&deps.storage, U32Key::new(1))
            .unwrap();
        assert_eq!(cw20_addr.as_bytes(), market_reference.reference.as_slice());

        // should have an asset_type of cw20
        assert_eq!(AssetType::Cw20, market.asset_type);

        // Should have market count of 2
        let money_market = RED_BANK.load(&deps.storage).unwrap();
        assert_eq!(2, money_market.market_count);

        assert_eq!(
            res.attributes,
            vec![
                attr("action", "init_asset"),
                attr("asset", cw20_addr.clone())
            ],
        );
        // *
        // cw20 callback comes back with created token
        // *
        let msg = ExecuteMsg::InitAssetTokenCallback {
            reference: Vec::from(cw20_addr.as_bytes()),
        };
        let info = mock_info("mtokencontract");
        execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // should have asset market with contract address
        let market = MARKETS.load(&deps.storage, cw20_addr.as_bytes()).unwrap();
        assert_eq!(Addr::unchecked("mtokencontract"), market.ma_token_address);
        assert_eq!(Decimal256::one(), market.liquidity_index);

        // *
        // calling this again should not be allowed
        // *
        let msg = ExecuteMsg::InitAssetTokenCallback {
            reference: Vec::from(cw20_addr.as_bytes()),
        };
        let info = mock_info("mtokencontract");
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());
    }

    #[test]
    fn test_update_asset() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal256::from_ratio(20, 100)),
            min_borrow_rate: Some(Decimal256::from_ratio(5, 100)),
            max_borrow_rate: Some(Decimal256::from_ratio(50, 100)),
            max_loan_to_value: Some(Decimal256::from_ratio(50, 100)),
            reserve_factor: Some(Decimal256::from_ratio(1, 100)),
            maintenance_margin: Some(Decimal256::from_ratio(80, 100)),
            liquidation_bonus: Some(Decimal256::from_ratio(10, 100)),
            kp_1: Some(Decimal256::from_ratio(3, 1)),
            optimal_utilization_rate: Some(Decimal256::from_ratio(80, 100)),
            kp_augmentation_threshold: Some(Decimal256::from_ratio(2000, 1)),
            kp_2: Some(Decimal256::from_ratio(2, 1)),
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("somebody");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());

        // *
        // owner is authorized but can't update asset if not initialize firstly
        // *
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Asset not initialized").into()
        );

        // *
        // initialize asset
        // *
        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("owner");
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // update asset with some params greater than 1
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            maintenance_margin: Some(Decimal256::from_ratio(110, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("[max_loan_to_value, reserve_factor, maintenance_margin, liquidation_bonus] should be less or equal 1. \
                Invalid params: [maintenance_margin]").into());

        // *
        // update asset where LTV >= liquidity threshold
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(6, 10)),
            maintenance_margin: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "maintenance_margin should be greater than max_loan_to_value. \
                    maintenance_margin: 0.5, \
                    max_loan_to_value: 0.6"
            )
            .into()
        );

        // *
        // init asset where min borrow rate >= max borrow rate
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            min_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            max_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("max_borrow_rate should be greater than min_borrow_rate. max_borrow_rate: 0.4, min_borrow_rate: 0.4").into());

        // *
        // init asset where optimal utilization rate > 1
        // *
        let invalid_asset_params = InitOrUpdateAssetParams {
            optimal_utilization_rate: Some(Decimal256::from_ratio(11, 10)),
            ..asset_params
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let info = mock_info("owner");
        let error_res = execute(deps.as_mut(), env.clone(), info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Optimal utilization rate can't be greater than one").into()
        );

        // *
        // update asset with new params
        // *
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal256::from_ratio(20, 100)),
            min_borrow_rate: Some(Decimal256::from_ratio(5, 100)),
            max_borrow_rate: Some(Decimal256::from_ratio(50, 100)),
            max_loan_to_value: Some(Decimal256::from_ratio(60, 100)),
            reserve_factor: Some(Decimal256::from_ratio(10, 100)),
            maintenance_margin: Some(Decimal256::from_ratio(90, 100)),
            liquidation_bonus: Some(Decimal256::from_ratio(12, 100)),
            kp_1: Some(Decimal256::from_ratio(3, 1)),
            optimal_utilization_rate: Some(Decimal256::from_ratio(80, 100)),
            kp_augmentation_threshold: Some(Decimal256::from_ratio(2000, 1)),
            kp_2: Some(Decimal256::from_ratio(2, 1)),
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let info = mock_info("owner");
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        let new_market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(0, new_market.index);
        assert_eq!(
            asset_params.max_loan_to_value.unwrap(),
            new_market.max_loan_to_value
        );
        assert_eq!(
            asset_params.reserve_factor.unwrap(),
            new_market.reserve_factor
        );
        assert_eq!(
            asset_params.maintenance_margin.unwrap(),
            new_market.maintenance_margin
        );
        assert_eq!(
            asset_params.liquidation_bonus.unwrap(),
            new_market.liquidation_bonus
        );

        let new_market_reference = MARKET_REFERENCES
            .load(&deps.storage, U32Key::new(0))
            .unwrap();
        assert_eq!(b"someasset", new_market_reference.reference.as_slice());

        let new_money_market = RED_BANK.load(&deps.storage).unwrap();
        assert_eq!(new_money_market.market_count, 1);

        assert_eq!(res.messages, vec![],);

        assert_eq!(
            res.attributes,
            vec![attr("action", "update_asset"), attr("asset", "someasset"),],
        );

        // *
        // update asset with empty params
        // *
        let empty_asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: None,
            min_borrow_rate: None,
            max_borrow_rate: None,
            max_loan_to_value: None,
            reserve_factor: None,
            maintenance_margin: None,
            liquidation_bonus: None,
            kp_1: None,
            optimal_utilization_rate: None,
            kp_augmentation_threshold: None,
            kp_2: None,
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let info = mock_info("owner");
        let _res = execute(deps.as_mut(), env, info, msg).unwrap();

        let new_market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(0, new_market.index);
        // should keep old params
        assert_eq!(
            asset_params.initial_borrow_rate.unwrap(),
            new_market.borrow_rate
        );
        assert_eq!(
            asset_params.min_borrow_rate.unwrap(),
            new_market.min_borrow_rate
        );
        assert_eq!(
            asset_params.max_borrow_rate.unwrap(),
            new_market.max_borrow_rate
        );
        assert_eq!(
            asset_params.max_loan_to_value.unwrap(),
            new_market.max_loan_to_value
        );
        assert_eq!(
            asset_params.reserve_factor.unwrap(),
            new_market.reserve_factor
        );
        assert_eq!(
            asset_params.maintenance_margin.unwrap(),
            new_market.maintenance_margin
        );
        assert_eq!(
            asset_params.liquidation_bonus.unwrap(),
            new_market.liquidation_bonus
        );
        assert_eq!(asset_params.kp_1.unwrap(), new_market.pid_parameters.kp_1);
        assert_eq!(
            asset_params.kp_augmentation_threshold.unwrap(),
            new_market.pid_parameters.kp_augmentation_threshold
        );
        assert_eq!(asset_params.kp_2.unwrap(), new_market.pid_parameters.kp_2);
    }

    #[test]
    fn test_init_asset_callback_cannot_be_called_on_its_own() {
        let mut deps = th_setup(&[]);

        let env = mock_env(MockEnvParams::default());
        let info = mock_info("mtokencontract");
        let msg = ExecuteMsg::InitAssetTokenCallback {
            reference: "uluna".into(),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::not_found("red_bank::state::Market").into()
        );
    }

    #[test]
    fn test_deposit_native_asset() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "somecoin")]);
        let reserve_factor = Decimal256::from_ratio(1, 10);

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal256::from_ratio(11, 10),
            max_loan_to_value: Decimal256::one(),
            borrow_index: Decimal256::from_ratio(1, 1),
            borrow_rate: Decimal256::from_ratio(10, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor,
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            ..Default::default()
        };
        let market = th_init_market(deps.as_mut(), b"somecoin", &mock_market);

        let deposit_amount = 110000;
        let env = mock_env_at_block_time(10000100);
        let info =
            cosmwasm_std::testing::mock_info("depositor", &[coin(deposit_amount, "somecoin")]);
        let msg = ExecuteMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market,
            env.block.time.seconds(),
            initial_liquidity,
            Default::default(),
        );

        let expected_mint_amount =
            (Uint256::from(deposit_amount) / expected_params.liquidity_index).into();

        // mints coin_amount/liquidity_index
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "matoken".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: "depositor".to_string(),
                    amount: expected_mint_amount,
                })
                .unwrap(),
                funds: vec![]
            }))]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "deposit"),
                attr("market", "somecoin"),
                attr("user", "depositor"),
                attr("amount", deposit_amount),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        let market = MARKETS.load(&deps.storage, b"somecoin").unwrap();
        assert_eq!(market.borrow_rate, expected_params.borrow_rate);
        assert_eq!(market.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(market.liquidity_index, expected_params.liquidity_index);
        assert_eq!(market.borrow_index, expected_params.borrow_index);
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let info = mock_info("depositor");
        let msg = ExecuteMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Deposit amount must be greater than 0 somecoin").into()
        );
    }

    #[test]
    fn test_deposit_cw20() {
        let initial_liquidity = 10_000_000;
        let mut deps = th_setup(&[]);

        let cw20_addr = Addr::unchecked("somecontract");

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal256::from_ratio(11, 10),
            max_loan_to_value: Decimal256::one(),
            borrow_index: Decimal256::from_ratio(1, 1),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(4, 100),
            debt_total_scaled: Uint256::from(10_000_000u128),
            interests_last_updated: 10_000_000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let market = th_init_market(deps.as_mut(), cw20_addr.as_bytes(), &mock_market);

        // set initial balance on cw20 contract
        deps.querier.set_cw20_balances(
            cw20_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                initial_liquidity.into(),
            )],
        );
        // set symbol for cw20 contract
        deps.querier
            .set_cw20_symbol(cw20_addr.clone(), "somecoin".to_string());

        let deposit_amount = 110000u128;
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::DepositCw20 {}).unwrap(),
            sender: "depositor".to_string(),
            amount: Uint128::new(deposit_amount),
        });
        let env = mock_env_at_block_time(10000100);
        let info =
            cosmwasm_std::testing::mock_info("somecontract", &[coin(deposit_amount, "somecoin")]);

        let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market,
            env.block.time.seconds(),
            initial_liquidity,
            Default::default(),
        );

        let expected_mint_amount: Uint256 =
            Uint256::from(deposit_amount) / expected_params.liquidity_index;

        let market = MARKETS.load(&deps.storage, cw20_addr.as_bytes()).unwrap();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "matoken".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: "depositor".to_string(),
                    amount: expected_mint_amount.into(),
                })
                .unwrap(),
                funds: vec![]
            }))]
        );

        assert_eq!(
            res.attributes,
            vec![
                attr("action", "deposit"),
                attr("market", cw20_addr),
                attr("user", "depositor"),
                attr("amount", deposit_amount),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let info = mock_info("depositor");
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::DepositCw20 {}).unwrap(),
            sender: "depositor".to_string(),
            amount: Uint128::new(deposit_amount),
        });
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::not_found("red_bank::state::Market").into()
        );
    }

    #[test]
    fn test_cannot_deposit_if_no_market() {
        let mut deps = th_setup(&[]);
        let env = mock_env(MockEnvParams::default());

        let info = cosmwasm_std::testing::mock_info("depositer", &[coin(110000, "somecoin")]);
        let msg = ExecuteMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::not_found("red_bank::state::Market").into()
        );
    }

    #[test]
    fn test_withdraw_native() {
        // Withdraw native token
        let initial_available_liquidity = 12000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128::new(100u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal256::from_ratio(2, 1),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let withdraw_amount = Uint256::from(20000u128);
        let seconds_elapsed = 2000u64;

        deps.querier.set_cw20_balances(
            Addr::unchecked("matoken"),
            &[(Addr::unchecked("withdrawer"), Uint128::new(2000000u128))],
        );

        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        MARKET_MA_TOKENS
            .save(
                deps.as_mut().storage,
                &Addr::unchecked("matoken"),
                &(b"somecoin".to_vec()),
            )
            .unwrap();

        let withdrawer_addr = Addr::unchecked("withdrawer");
        let user = User::default();
        USERS
            .save(deps.as_mut().storage, &withdrawer_addr, &user)
            .unwrap();

        let msg = ExecuteMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(withdraw_amount),
        };

        let env = mock_env_at_block_time(mock_market.interests_last_updated + seconds_elapsed);
        let info = mock_info("withdrawer");
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let market = MARKETS.load(&deps.storage, b"somecoin").unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_initial,
            mock_market.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: withdraw_amount.into(),
                ..Default::default()
            },
        );

        let withdraw_amount_scaled = withdraw_amount / expected_params.liquidity_index;

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "matoken".to_string(),
                    msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
                        user: withdrawer_addr.to_string(),
                        amount: withdraw_amount_scaled.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: withdrawer_addr.to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: String::from("somecoin"),
                            amount: withdraw_amount.into(),
                        }
                    )
                    .unwrap()],
                })),
            ]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "withdraw"),
                attr("market", "somecoin"),
                attr("user", "withdrawer"),
                attr("burn_amount", withdraw_amount_scaled),
                attr("withdraw_amount", withdraw_amount),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        assert_eq!(market.borrow_rate, expected_params.borrow_rate);
        assert_eq!(market.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(market.liquidity_index, expected_params.liquidity_index);
        assert_eq!(market.borrow_index, expected_params.borrow_index);
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn test_withdraw_cw20() {
        // Withdraw cw20 token
        let mut deps = th_setup(&[]);
        let cw20_contract_addr = Addr::unchecked("somecontract");
        let initial_available_liquidity = 12000000u128;

        let ma_token_addr = Addr::unchecked("matoken");

        deps.querier.set_cw20_balances(
            cw20_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(initial_available_liquidity),
            )],
        );
        deps.querier.set_cw20_balances(
            ma_token_addr.clone(),
            &[(Addr::unchecked("withdrawer"), Uint128::new(2000000u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal256::from_ratio(2, 1),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(2, 100),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let withdraw_amount = Uint256::from(20000u128);
        let seconds_elapsed = 2000u64;

        let market_initial =
            th_init_market(deps.as_mut(), cw20_contract_addr.as_bytes(), &mock_market);
        MARKET_MA_TOKENS
            .save(
                deps.as_mut().storage,
                &ma_token_addr,
                &cw20_contract_addr.as_bytes().to_vec(),
            )
            .unwrap();

        let withdrawer_addr = Addr::unchecked("withdrawer");

        let user = User::default();
        USERS
            .save(deps.as_mut().storage, &withdrawer_addr, &user)
            .unwrap();

        let msg = ExecuteMsg::Withdraw {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.to_string(),
            },
            amount: Some(withdraw_amount),
        };

        let env = mock_env_at_block_time(mock_market.interests_last_updated + seconds_elapsed);
        let info = mock_info("withdrawer");
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let market = MARKETS
            .load(&deps.storage, cw20_contract_addr.as_bytes())
            .unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_initial,
            mock_market.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: withdraw_amount.into(),
                ..Default::default()
            },
        );

        let withdraw_amount_scaled = withdraw_amount / expected_params.liquidity_index;

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: ma_token_addr.to_string(),
                    msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
                        user: withdrawer_addr.to_string(),
                        amount: withdraw_amount_scaled.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: cw20_contract_addr.to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: withdrawer_addr.to_string(),
                        amount: withdraw_amount.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
            ]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "withdraw"),
                attr("market", "somecontract"),
                attr("user", "withdrawer"),
                attr("burn_amount", withdraw_amount_scaled),
                attr("withdraw_amount", withdraw_amount),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        assert_eq!(market.borrow_rate, expected_params.borrow_rate);
        assert_eq!(market.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(market.liquidity_index, expected_params.liquidity_index);
        assert_eq!(market.borrow_index, expected_params.borrow_index);
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn test_withdraw_cannot_exceed_balance() {
        let mut deps = th_setup(&[]);
        let env = mock_env(MockEnvParams::default());

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal256::from_ratio(15, 10),
            ..Default::default()
        };

        deps.querier.set_cw20_balances(
            Addr::unchecked("matoken"),
            &[(Addr::unchecked("withdrawer"), Uint128::new(200u128))],
        );

        th_init_market(deps.as_mut(), b"somecoin", &mock_market);

        let msg = ExecuteMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(Uint256::from(2000u128)),
        };

        let info = mock_info("withdrawer");
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(error_res, StdError::generic_err("Withdraw amount must be greater than 0 and less or equal user balance (asset: somecoin)").into());
    }

    #[test]
    fn test_withdraw_if_health_factor_not_met() {
        let initial_available_liquidity = 10000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "token3")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("token3"), Uint128::new(100u128))],
        );

        let withdrawer_addr = Addr::unchecked("withdrawer");

        // Initialize markets
        let ma_token_1_addr = Addr::unchecked("matoken1");
        let market_1 = Market {
            ma_token_address: ma_token_1_addr.clone(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(40, 100),
            maintenance_margin: Decimal256::from_ratio(60, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_2_addr = Addr::unchecked("matoken2");
        let market_2 = Market {
            ma_token_address: ma_token_2_addr,
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(50, 100),
            maintenance_margin: Decimal256::from_ratio(80, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_3_addr = Addr::unchecked("matoken3");
        let market_3 = Market {
            ma_token_address: ma_token_3_addr.clone(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(20, 100),
            maintenance_margin: Decimal256::from_ratio(40, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let market_1_initial = th_init_market(deps.as_mut(), b"token1", &market_1);
        let market_2_initial = th_init_market(deps.as_mut(), b"token2", &market_2);
        let market_3_initial = th_init_market(deps.as_mut(), b"token3", &market_3);

        // Initialize user with market_1 and market_3 as collaterals
        // User borrows market_2
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_3_initial.index).unwrap();
        set_bit(&mut user.borrowed_assets, market_2_initial.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &withdrawer_addr, &user)
            .unwrap();

        // Set the querier to return collateral balances (ma_token_1 and ma_token_3)
        let ma_token_1_balance_scaled = Uint128::new(100000);
        deps.querier.set_cw20_balances(
            ma_token_1_addr,
            &[(withdrawer_addr.clone(), ma_token_1_balance_scaled)],
        );
        let ma_token_3_balance_scaled = Uint128::new(600000);
        deps.querier.set_cw20_balances(
            ma_token_3_addr,
            &[(withdrawer_addr.clone(), ma_token_3_balance_scaled)],
        );

        // Set user to have positive debt amount in debt asset
        // Uncollateralized debt shouldn't count for health factor
        let token_2_debt_scaled = Uint256::from(200000u128);
        let debt = Debt {
            amount_scaled: token_2_debt_scaled,
            uncollateralized: false,
        };
        let uncollateralized_debt = Debt {
            amount_scaled: Uint256::from(200000u128),
            uncollateralized: true,
        };
        DEBTS
            .save(deps.as_mut().storage, (b"token2", &withdrawer_addr), &debt)
            .unwrap();
        DEBTS
            .save(
                deps.as_mut().storage,
                (b"token3", &withdrawer_addr),
                &uncollateralized_debt,
            )
            .unwrap();

        // Set the querier to return native exchange rates
        let token_1_exchange_rate = Decimal::from_ratio(3u128, 1u128);
        let token_2_exchange_rate = Decimal::from_ratio(2u128, 1u128);
        let token_3_exchange_rate = Decimal::from_ratio(1u128, 1u128);
        let exchange_rates = [
            (String::from("token1"), token_1_exchange_rate),
            (String::from("token2"), token_2_exchange_rate),
            (String::from("token3"), token_3_exchange_rate),
        ];
        deps.querier
            .set_native_exchange_rates(String::from("uusd"), &exchange_rates[..]);

        let env = mock_env(MockEnvParams::default());
        let info = mock_info("withdrawer");

        // Calculate how much to withdraw to have health factor equal to one
        let how_much_to_withdraw = {
            let token_1_weighted_lt_in_uusd = Uint256::from(ma_token_1_balance_scaled)
                * get_updated_liquidity_index(&market_1_initial, env.block.time.seconds())
                * market_1_initial.maintenance_margin
                * Decimal256::from(token_1_exchange_rate);
            let token_3_weighted_lt_in_uusd = Uint256::from(ma_token_3_balance_scaled)
                * get_updated_liquidity_index(&market_3_initial, env.block.time.seconds())
                * market_3_initial.maintenance_margin
                * Decimal256::from(token_3_exchange_rate);
            let weighted_maintenance_margin_in_uusd =
                token_1_weighted_lt_in_uusd + token_3_weighted_lt_in_uusd;

            let total_collateralized_debt_in_uusd = token_2_debt_scaled
                * get_updated_borrow_index(&market_2_initial, env.block.time.seconds())
                * Decimal256::from(token_2_exchange_rate);

            // How much to withdraw in uusd to have health factor equal to one
            let how_much_to_withdraw_in_uusd = (weighted_maintenance_margin_in_uusd
                - total_collateralized_debt_in_uusd)
                / market_3_initial.maintenance_margin;
            how_much_to_withdraw_in_uusd / Decimal256::from(token_3_exchange_rate)
        };

        // Withdraw token3 with failure
        // The withdraw amount needs to be a little bit greater to have health factor less than one
        {
            let withdraw_amount = how_much_to_withdraw + Uint256::from(10u128);
            let msg = ExecuteMsg::Withdraw {
                asset: Asset::Native {
                    denom: "token3".to_string(),
                },
                amount: Some(withdraw_amount),
            };
            let error_res = execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap_err();
            assert_eq!(
                error_res,
                StdError::generic_err("User's health factor can't be less than 1 after withdraw")
                    .into()
            );
        }

        // Withdraw token3 with success
        // The withdraw amount needs to be a little bit smaller to have health factor greater than one
        {
            let withdraw_amount = how_much_to_withdraw - Uint256::from(10u128);
            let msg = ExecuteMsg::Withdraw {
                asset: Asset::Native {
                    denom: "token3".to_string(),
                },
                amount: Some(withdraw_amount),
            };
            let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

            let withdraw_amount_scaled = withdraw_amount
                / get_updated_liquidity_index(&market_3_initial, env.block.time.seconds());

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: "matoken3".to_string(),
                        msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
                            user: withdrawer_addr.to_string(),
                            amount: withdraw_amount_scaled.into(),
                        })
                        .unwrap(),
                        funds: vec![]
                    })),
                    SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                        to_address: withdrawer_addr.to_string(),
                        amount: vec![deduct_tax(
                            deps.as_ref(),
                            Coin {
                                denom: String::from("token3"),
                                amount: withdraw_amount.into(),
                            }
                        )
                        .unwrap()],
                    })),
                ]
            );
        }
    }

    #[test]
    fn test_withdraw_total_balance() {
        // Withdraw native token
        let initial_available_liquidity = 12000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128::new(100u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal256::from_ratio(2, 1),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let withdrawer_balance_scaled = Uint256::from(123456u128);
        let seconds_elapsed = 2000u64;

        deps.querier.set_cw20_balances(
            Addr::unchecked("matoken"),
            &[(
                Addr::unchecked("withdrawer"),
                withdrawer_balance_scaled.into(),
            )],
        );

        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        MARKET_MA_TOKENS
            .save(
                deps.as_mut().storage,
                &Addr::unchecked("matoken"),
                &(b"somecoin".to_vec()),
            )
            .unwrap();

        // Mark the market as collateral for the user
        let withdrawer_addr = Addr::unchecked("withdrawer");
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_initial.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &withdrawer_addr, &user)
            .unwrap();
        // Check if user has set bit for collateral
        assert!(get_bit(user.collateral_assets, market_initial.index).unwrap());

        let msg = ExecuteMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: None,
        };

        let env = mock_env_at_block_time(mock_market.interests_last_updated + seconds_elapsed);
        let info = mock_info("withdrawer");
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let market = MARKETS.load(&deps.storage, b"somecoin").unwrap();

        let withdrawer_balance = withdrawer_balance_scaled
            * get_updated_liquidity_index(
                &market_initial,
                market_initial.interests_last_updated + seconds_elapsed,
            );

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_initial,
            mock_market.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: withdrawer_balance.into(),
                ..Default::default()
            },
        );

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "matoken".to_string(),
                    msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
                        user: withdrawer_addr.to_string(),
                        amount: withdrawer_balance_scaled.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: withdrawer_addr.to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: String::from("somecoin"),
                            amount: withdrawer_balance.into(),
                        }
                    )
                    .unwrap()],
                })),
            ]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "withdraw"),
                attr("market", "somecoin"),
                attr("user", "withdrawer"),
                attr("burn_amount", withdrawer_balance_scaled),
                attr("withdraw_amount", withdrawer_balance),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        assert_eq!(market.borrow_rate, expected_params.borrow_rate);
        assert_eq!(market.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(market.liquidity_index, expected_params.liquidity_index);
        assert_eq!(market.borrow_index, expected_params.borrow_index);
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // User should have unset bit for collateral after full withdraw
        let user = USERS.load(&deps.storage, &withdrawer_addr).unwrap();
        assert!(!get_bit(user.collateral_assets, market_initial.index).unwrap());
    }

    #[test]
    fn test_borrow_and_repay() {
        // NOTE: available liquidity stays fixed as the test environment does not get changes in
        // contract balances on subsequent calls. They would change from call to call in practice
        let available_liquidity_cw20 = 1000000000u128; // cw20
        let available_liquidity_native = 2000000000u128; // native
        let mut deps = th_setup(&[coin(available_liquidity_native, "borrowedcoinnative")]);

        let cw20_contract_addr = Addr::unchecked("borrowedcoincw20");
        deps.querier.set_cw20_balances(
            cw20_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(available_liquidity_cw20),
            )],
        );

        let exchange_rates = [
            (String::from("borrowedcoinnative"), Decimal::one()),
            (String::from("depositedcoin"), Decimal::one()),
        ];
        deps.querier
            .set_native_exchange_rates(String::from("uusd"), &exchange_rates[..]);
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("borrowedcoinnative"), Uint128::new(100u128))],
        );

        let mock_market_1 = Market {
            ma_token_address: Addr::unchecked("matoken1"),
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: Addr::unchecked("matoken2"),
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: Addr::unchecked("matoken3"),
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::from_ratio(11, 10),
            max_loan_to_value: Decimal256::from_ratio(7, 10),
            borrow_rate: Decimal256::from_ratio(30, 100),
            reserve_factor: Decimal256::from_ratio(3, 100),
            liquidity_rate: Decimal256::from_ratio(20, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let market_1_initial =
            th_init_market(deps.as_mut(), cw20_contract_addr.as_bytes(), &mock_market_1);
        // should get index 1
        let market_2_initial = th_init_market(deps.as_mut(), b"borrowedcoinnative", &mock_market_2);
        // should get index 2
        let market_collateral = th_init_market(deps.as_mut(), b"depositedcoin", &mock_market_3);

        let borrower_addr = Addr::unchecked("borrower");

        // Set user as having the market_collateral deposited
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, market_collateral.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &borrower_addr, &user)
            .unwrap();

        // Set the querier to return a certain collateral balance
        let deposit_coin_address = Addr::unchecked("matoken3");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(borrower_addr.clone(), Uint128::new(10000))],
        );

        // TODO: probably some variables (ie: borrow_amount, expected_params) that are repeated
        // in all calls could be enclosed in local scopes somehow)
        // *
        // Borrow cw20 token
        // *
        let block_time = mock_market_1.interests_last_updated + 10000u64;
        let borrow_amount = 2400u128;

        let msg = ExecuteMsg::Borrow {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.to_string(),
            },
            amount: Uint256::from(borrow_amount),
        };

        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");

        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_1_initial,
            block_time,
            available_liquidity_cw20,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );

        // check correct messages and logging
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_addr.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: borrower_addr.to_string(),
                    amount: borrow_amount.into(),
                })
                .unwrap(),
                funds: vec![]
            }))]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "borrow"),
                attr("market", "borrowedcoincw20"),
                attr("user", "borrower"),
                attr("amount", borrow_amount),
                attr("borrow_index", expected_params_cw20.borrow_index),
                attr("liquidity_index", expected_params_cw20.liquidity_index),
                attr("borrow_rate", expected_params_cw20.borrow_rate),
                attr("liquidity_rate", expected_params_cw20.liquidity_rate),
            ]
        );

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt = DEBTS
            .load(
                &deps.storage,
                (cw20_contract_addr.as_bytes(), &borrower_addr),
            )
            .unwrap();
        let expected_debt_scaled_1_after_borrow =
            Uint256::from(borrow_amount) / expected_params_cw20.borrow_index;

        let market_1_after_borrow = MARKETS
            .load(&deps.storage, cw20_contract_addr.as_bytes())
            .unwrap();

        assert_eq!(expected_debt_scaled_1_after_borrow, debt.amount_scaled);
        assert_eq!(
            expected_debt_scaled_1_after_borrow,
            market_1_after_borrow.debt_total_scaled
        );
        assert_eq!(
            expected_params_cw20.borrow_rate,
            market_1_after_borrow.borrow_rate
        );
        assert_eq!(
            expected_params_cw20.liquidity_rate,
            market_1_after_borrow.liquidity_rate
        );

        // *
        // Borrow cw20 token (again)
        // *
        let borrow_amount = 1200u128;
        let block_time = market_1_after_borrow.interests_last_updated + 20000u64;

        let msg = ExecuteMsg::Borrow {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.to_string(),
            },
            amount: Uint256::from(borrow_amount),
        };

        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");

        execute(deps.as_mut(), env, info, msg).unwrap();

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_1_after_borrow,
            block_time,
            available_liquidity_cw20,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );
        let debt = DEBTS
            .load(
                &deps.storage,
                (cw20_contract_addr.as_bytes(), &borrower_addr),
            )
            .unwrap();
        let market_1_after_borrow_again = MARKETS
            .load(&deps.storage, cw20_contract_addr.as_bytes())
            .unwrap();

        let expected_debt_scaled_1_after_borrow_again = expected_debt_scaled_1_after_borrow
            + Uint256::from(borrow_amount) / expected_params_cw20.borrow_index;
        assert_eq!(
            expected_debt_scaled_1_after_borrow_again,
            debt.amount_scaled
        );
        assert_eq!(
            expected_debt_scaled_1_after_borrow_again,
            market_1_after_borrow_again.debt_total_scaled
        );
        assert_eq!(
            expected_params_cw20.borrow_rate,
            market_1_after_borrow_again.borrow_rate
        );
        assert_eq!(
            expected_params_cw20.liquidity_rate,
            market_1_after_borrow_again.liquidity_rate
        );

        // *
        // Borrow native coin
        // *

        let borrow_amount = 4000u128;
        let block_time = market_1_after_borrow_again.interests_last_updated + 3000u64;
        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        let msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: String::from("borrowedcoinnative"),
            },
            amount: Uint256::from(borrow_amount),
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_native = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_2_initial,
            block_time,
            available_liquidity_native,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );

        // check correct messages and logging
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: "borrower".to_string(),
                amount: vec![deduct_tax(
                    deps.as_ref(),
                    Coin {
                        denom: String::from("borrowedcoinnative"),
                        amount: borrow_amount.into(),
                    }
                )
                .unwrap()],
            }))]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "borrow"),
                attr("market", "borrowedcoinnative"),
                attr("user", "borrower"),
                attr("amount", borrow_amount),
                attr("borrow_index", expected_params_native.borrow_index),
                attr("liquidity_index", expected_params_native.liquidity_index),
                attr("borrow_rate", expected_params_native.borrow_rate),
                attr("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let debt2 = DEBTS
            .load(&deps.storage, (b"borrowedcoinnative", &borrower_addr))
            .unwrap();
        let market_2_after_borrow_2 = MARKETS.load(&deps.storage, b"borrowedcoinnative").unwrap();

        let expected_debt_scaled_2_after_borrow_2 =
            Uint256::from(borrow_amount) / expected_params_native.borrow_index;
        assert_eq!(expected_debt_scaled_2_after_borrow_2, debt2.amount_scaled);
        assert_eq!(
            expected_debt_scaled_2_after_borrow_2,
            market_2_after_borrow_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_native.borrow_rate,
            market_2_after_borrow_2.borrow_rate
        );
        assert_eq!(
            expected_params_native.liquidity_rate,
            market_2_after_borrow_2.liquidity_rate
        );

        // *
        // Borrow native coin again (should fail due to insufficient collateral)
        // *

        let env = mock_env(MockEnvParams::default());
        let info = mock_info("borrower");
        let msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: String::from("borrowedcoinnative"),
            },
            amount: Uint256::from(83968_u128),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "borrow amount exceeds maximum allowed given current collateral value"
            )
            .into()
        );

        // *
        // Repay zero native debt(should fail)
        // *
        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        let msg = ExecuteMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Repay amount must be greater than 0 borrowedcoinnative").into()
        );

        // *
        // Repay some native debt
        // *
        let repay_amount = 2000u128;
        let block_time = market_2_after_borrow_2.interests_last_updated + 8000u64;
        let env = mock_env_at_block_time(block_time);
        let info = cosmwasm_std::testing::mock_info(
            "borrower",
            &[coin(repay_amount, "borrowedcoinnative")],
        );
        let msg = ExecuteMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let expected_params_native = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_2_after_borrow_2,
            block_time,
            available_liquidity_native,
            TestUtilizationDeltas {
                less_debt: repay_amount,
                ..Default::default()
            },
        );

        assert_eq!(res.messages, vec![]);
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "repay"),
                attr("market", "borrowedcoinnative"),
                attr("user", "borrower"),
                attr("amount", repay_amount),
                attr("borrow_index", expected_params_native.borrow_index),
                attr("liquidity_index", expected_params_native.liquidity_index),
                attr("borrow_rate", expected_params_native.borrow_rate),
                attr("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = DEBTS
            .load(&deps.storage, (b"borrowedcoinnative", &borrower_addr))
            .unwrap();
        let market_2_after_repay_some_2 =
            MARKETS.load(&deps.storage, b"borrowedcoinnative").unwrap();

        let expected_debt_scaled_2_after_repay_some_2 = expected_debt_scaled_2_after_borrow_2
            - Uint256::from(repay_amount) / expected_params_native.borrow_index;
        assert_eq!(
            expected_debt_scaled_2_after_repay_some_2,
            debt2.amount_scaled
        );
        assert_eq!(
            expected_debt_scaled_2_after_repay_some_2,
            market_2_after_repay_some_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_native.borrow_rate,
            market_2_after_repay_some_2.borrow_rate
        );
        assert_eq!(
            expected_params_native.liquidity_rate,
            market_2_after_repay_some_2.liquidity_rate
        );

        // *
        // Repay all native debt
        // *
        let block_time = market_2_after_repay_some_2.interests_last_updated + 10000u64;
        // need this to compute the repay amount
        let expected_params_native = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_2_after_repay_some_2,
            block_time,
            available_liquidity_native,
            TestUtilizationDeltas {
                less_debt: 9999999999999, // hack: Just do a big number to repay all debt,
                ..Default::default()
            },
        );
        // TODO: There's a rounding error when multiplying a dividing by a Decimal256
        // probably because intermediate result is cast to Uint256. doing everything in Decimal256
        // eliminates this but need to then find a way to cast it back to an integer
        let repay_amount: u128 = (expected_debt_scaled_2_after_repay_some_2
            * expected_params_native.borrow_index)
            .into();

        let env = mock_env_at_block_time(block_time);
        let info = cosmwasm_std::testing::mock_info(
            "borrower",
            &[coin(repay_amount, "borrowedcoinnative")],
        );
        let msg = ExecuteMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        assert_eq!(res.messages, vec![]);
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "repay"),
                attr("market", "borrowedcoinnative"),
                attr("user", "borrower"),
                attr("amount", repay_amount),
                attr("borrow_index", expected_params_native.borrow_index),
                attr("liquidity_index", expected_params_native.liquidity_index),
                attr("borrow_rate", expected_params_native.borrow_rate),
                attr("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = DEBTS
            .load(&deps.storage, (b"borrowedcoinnative", &borrower_addr))
            .unwrap();
        let market_2_after_repay_all_2 =
            MARKETS.load(&deps.storage, b"borrowedcoinnative").unwrap();

        assert_eq!(Uint256::zero(), debt2.amount_scaled);
        assert_eq!(
            Uint256::zero(),
            market_2_after_repay_all_2.debt_total_scaled
        );

        // *
        // Repay more native debt (should fail)
        // *
        let env = mock_env(MockEnvParams::default());
        let info =
            cosmwasm_std::testing::mock_info("borrower", &[coin(2000, "borrowedcoinnative")]);
        let msg = ExecuteMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("Cannot repay 0 debt").into()
        );

        // *
        // Repay all cw20 debt (and then some)
        // *
        let block_time = market_2_after_repay_all_2.interests_last_updated + 5000u64;
        let repay_amount = 4800u128;

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_1_after_borrow_again,
            block_time,
            available_liquidity_cw20,
            TestUtilizationDeltas {
                less_debt: repay_amount,
                ..Default::default()
            },
        );

        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrowedcoincw20");

        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::RepayCw20 {}).unwrap(),
            sender: borrower_addr.to_string(),
            amount: Uint128::new(repay_amount),
        });

        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        let expected_repay_amount_scaled =
            Uint256::from(repay_amount) / expected_params_cw20.borrow_index;
        let expected_refund_amount: u128 = ((expected_repay_amount_scaled
            - expected_debt_scaled_1_after_borrow_again)
            * expected_params_cw20.borrow_index)
            .into();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_addr.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: borrower_addr.to_string(),
                    amount: expected_refund_amount.into(),
                })
                .unwrap(),
                funds: vec![]
            }))]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "repay"),
                attr("market", "borrowedcoincw20"),
                attr("user", "borrower"),
                attr(
                    "amount",
                    Uint128::new(repay_amount - expected_refund_amount)
                ),
                attr("borrow_index", expected_params_cw20.borrow_index),
                attr("liquidity_index", expected_params_cw20.liquidity_index),
                attr("borrow_rate", expected_params_cw20.borrow_rate),
                attr("liquidity_rate", expected_params_cw20.liquidity_rate),
            ]
        );
        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(!get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt1 = DEBTS
            .load(
                &deps.storage,
                (cw20_contract_addr.as_bytes(), &borrower_addr),
            )
            .unwrap();
        let market_1_after_repay_1 = MARKETS
            .load(&deps.storage, cw20_contract_addr.as_bytes())
            .unwrap();
        assert_eq!(Uint256::from(0_u128), debt1.amount_scaled);
        assert_eq!(
            Uint256::from(0_u128),
            market_1_after_repay_1.debt_total_scaled
        );
    }

    #[test]
    fn test_borrow_uusd() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "uusd")]);
        let block_time = 1;

        let borrower_addr = Addr::unchecked("borrower");
        let ltv = Decimal256::from_ratio(7, 10);

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal256::one(),
            max_loan_to_value: ltv,
            borrow_index: Decimal256::one(),
            borrow_rate: Decimal256::one(),
            liquidity_rate: Decimal256::one(),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: block_time,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let market = th_init_market(deps.as_mut(), b"uusd", &mock_market);

        // Set tax data for uusd
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("uusd"), Uint128::new(100u128))],
        );

        // Set user as having the market_collateral deposited
        let deposit_amount = 110000u64;
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &borrower_addr, &user)
            .unwrap();

        // Set the querier to return collateral balance
        let deposit_coin_address = Addr::unchecked("matoken");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(borrower_addr.clone(), Uint128::from(deposit_amount))],
        );

        // borrow with insufficient collateral, should fail
        let new_block_time = 120u64;
        let time_elapsed = new_block_time - market.interests_last_updated;
        let liquidity_index = calculate_applied_linear_interest_rate(
            market.liquidity_index,
            market.liquidity_rate,
            time_elapsed,
        );
        let collateral = Decimal256::from_uint256(Uint256::from(deposit_amount)) * liquidity_index;
        let max_to_borrow = Uint256::one() * (collateral * ltv);
        let msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "uusd".to_string(),
            },
            amount: max_to_borrow + Uint256::from(1u128),
        };
        let env = mock_env_at_block_time(new_block_time);
        let info = mock_info("borrower");
        let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "borrow amount exceeds maximum allowed given current collateral value"
            )
            .into()
        );

        let valid_amount = Uint256::from(deposit_amount) * ltv - Uint256::from(1000u128);
        let msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "uusd".to_string(),
            },
            amount: valid_amount,
        };
        let env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        execute(deps.as_mut(), env, info, msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market,
            block_time,
            initial_liquidity,
            TestUtilizationDeltas {
                less_liquidity: valid_amount.into(),
                more_debt: valid_amount.into(),
                ..Default::default()
            },
        );

        let market_after_borrow = MARKETS.load(&deps.storage, b"uusd").unwrap();

        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());

        let debt = DEBTS
            .load(&deps.storage, (b"uusd", &borrower_addr))
            .unwrap();

        assert_eq!(valid_amount, debt.amount_scaled);
        assert_eq!(
            market_after_borrow.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn test_collateral_check() {
        // NOTE: available liquidity stays fixed as the test environment does not get changes in
        // contract balances on subsequent calls. They would change from call to call in practice
        let available_liquidity_1 = 1000000000u128;
        let available_liquidity_2 = 2000000000u128;
        let available_liquidity_3 = 3000000000u128;
        let mut deps = th_setup(&[
            coin(available_liquidity_2, "depositedcoin2"),
            coin(available_liquidity_3, "uusd"),
        ]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("depositedcoin2"), Uint128::new(100u128))],
        );

        let cw20_contract_addr = Addr::unchecked("depositedcoin1");
        deps.querier.set_cw20_balances(
            cw20_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(available_liquidity_1),
            )],
        );

        let exchange_rate_1 = Decimal::one();
        let exchange_rate_2 = Decimal::from_ratio(15u128, 4u128);
        let exchange_rate_3 = Decimal::one();

        let exchange_rates = &[(String::from("depositedcoin2"), exchange_rate_2)];
        deps.querier
            .set_native_exchange_rates(String::from("uusd"), &exchange_rates[..]);

        let mock_market_1 = Market {
            ma_token_address: Addr::unchecked("matoken1"),
            max_loan_to_value: Decimal256::from_ratio(8, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: Addr::unchecked("matoken2"),
            max_loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: Addr::unchecked("matoken3"),
            max_loan_to_value: Decimal256::from_ratio(4, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let market_1_initial =
            th_init_market(deps.as_mut(), cw20_contract_addr.as_bytes(), &mock_market_1);
        // should get index 1
        let market_2_initial = th_init_market(deps.as_mut(), b"depositedcoin2", &mock_market_2);
        // should get index 2
        let market_3_initial = th_init_market(deps.as_mut(), b"uusd", &mock_market_3);

        let borrower_addr = Addr::unchecked("borrower");

        // Set user as having all the markets as collateral
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_3_initial.index).unwrap();

        USERS
            .save(deps.as_mut().storage, &borrower_addr, &user)
            .unwrap();

        let ma_token_address_1 = Addr::unchecked("matoken1");
        let ma_token_address_2 = Addr::unchecked("matoken2");
        let ma_token_address_3 = Addr::unchecked("matoken3");

        let balance_1 = Uint128::new(4_000_000);
        let balance_2 = Uint128::new(7_000_000);
        let balance_3 = Uint128::new(3_000_000);

        // Set the querier to return a certain collateral balance
        deps.querier
            .set_cw20_balances(ma_token_address_1, &[(borrower_addr.clone(), balance_1)]);
        deps.querier
            .set_cw20_balances(ma_token_address_2, &[(borrower_addr.clone(), balance_2)]);
        deps.querier
            .set_cw20_balances(ma_token_address_3, &[(borrower_addr, balance_3)]);

        let max_borrow_allowed_in_uusd = (market_1_initial.max_loan_to_value
            * Uint256::from(balance_1)
            * Decimal256::from(exchange_rate_1))
            + (market_2_initial.max_loan_to_value
                * Uint256::from(balance_2)
                * Decimal256::from(exchange_rate_2))
            + (market_3_initial.max_loan_to_value
                * Uint256::from(balance_3)
                * Decimal256::from(exchange_rate_3));
        let exceeding_borrow_amount = (max_borrow_allowed_in_uusd
            / Decimal256::from(exchange_rate_2))
            + Uint256::from(100_u64);
        let permissible_borrow_amount = (max_borrow_allowed_in_uusd
            / Decimal256::from(exchange_rate_2))
            - Uint256::from(100_u64);

        // borrow above the allowed amount given current collateral, should fail
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            amount: exceeding_borrow_amount,
        };
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("borrower");
        let error_res = execute(deps.as_mut(), env.clone(), info.clone(), borrow_msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "borrow amount exceeds maximum allowed given current collateral value"
            )
            .into()
        );

        // borrow permissible amount given current collateral, should succeed
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            amount: permissible_borrow_amount,
        };
        execute(deps.as_mut(), env, info, borrow_msg).unwrap();
    }

    #[test]
    pub fn test_handle_liquidate() {
        // Setup
        let available_liquidity_collateral = 1_000_000_000u128;
        let available_liquidity_debt = 2_000_000_000u128;
        let mut deps = th_setup(&[coin(available_liquidity_collateral, "collateral")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("collateral"), Uint128::new(100u128))],
        );

        let debt_contract_addr = Addr::unchecked("debt");
        let user_address = Addr::unchecked("user");
        let _collateral_address = Addr::unchecked("collateral");
        let liquidator_address = Addr::unchecked("liquidator");

        let collateral_max_ltv = Decimal256::from_ratio(5, 10);
        let collateral_maintenance_margin = Decimal256::from_ratio(6, 10);
        let collateral_liquidation_bonus = Decimal256::from_ratio(1, 10);
        let collateral_price = Decimal::from_ratio(2_u128, 1_u128);
        let debt_price = Decimal::from_ratio(1_u128, 1_u128);
        let user_collateral_balance = Uint128::new(2_000_000);
        // TODO: As this is a cw20, it's price will be 1uusd, review this when oracle is
        // implemented.
        let user_debt = Uint256::from(3_000_000_u64); // ltv = 0.75
        let close_factor = Decimal256::from_ratio(1, 2);

        let first_debt_to_repay = Uint256::from(400_000_u64);
        let first_block_time = 15_000_000;

        let second_debt_to_repay = Uint256::from(10_000_000_u64);
        let second_block_time = 16_000_000;

        // Global debt for the debt market
        let mut expected_global_debt_scaled = Uint256::from(1_800_000_000_u64);

        CONFIG
            .update(deps.as_mut().storage, |mut config| -> StdResult<_> {
                config.close_factor = close_factor;
                Ok(config)
            })
            .unwrap();

        deps.querier.set_cw20_balances(
            debt_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(available_liquidity_debt),
            )],
        );

        // initialize collateral and debt markets
        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[("collateral".to_string(), collateral_price)],
        );

        let collateral_market_ma_token_addr = Addr::unchecked("ma_collateral");
        let collateral_market = Market {
            ma_token_address: collateral_market_ma_token_addr.clone(),
            max_loan_to_value: collateral_max_ltv,
            maintenance_margin: collateral_maintenance_margin,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint256::from(800_000_000_u64),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            borrow_rate: Decimal256::from_ratio(2, 10),
            liquidity_rate: Decimal256::from_ratio(2, 10),
            reserve_factor: Decimal256::from_ratio(2, 100),
            asset_type: AssetType::Native,
            interests_last_updated: 0,
            ..Default::default()
        };

        let debt_market = Market {
            max_loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: expected_global_debt_scaled,
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            borrow_rate: Decimal256::from_ratio(2, 10),
            liquidity_rate: Decimal256::from_ratio(2, 10),
            reserve_factor: Decimal256::from_ratio(3, 100),
            asset_type: AssetType::Cw20,
            interests_last_updated: 0,
            ..Default::default()
        };

        let collateral_market_initial =
            th_init_market(deps.as_mut(), b"collateral", &collateral_market);

        let debt_market_initial =
            th_init_market(deps.as_mut(), debt_contract_addr.as_bytes(), &debt_market);

        let mut expected_user_debt_scaled = user_debt / debt_market_initial.liquidity_index;

        // Set user as having collateral and debt in respective markets
        {
            let mut user = User::default();
            set_bit(&mut user.collateral_assets, collateral_market_initial.index).unwrap();
            set_bit(&mut user.borrowed_assets, debt_market_initial.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &user_address, &user)
                .unwrap();
        }

        // trying to liquidate user with zero collateral balance should fail
        {
            deps.querier.set_cw20_balances(
                collateral_market_ma_token_addr,
                &[(user_address.clone(), Uint128::zero())],
            );

            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(debt_contract_addr.as_str());
            let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
            assert_eq!(
                error_res,
                StdError::generic_err(
                    "user has no balance in specified collateral asset to be liquidated"
                )
                .into()
            );
        }

        // Set the querier to return positive collateral balance
        deps.querier.set_cw20_balances(
            Addr::unchecked("ma_collateral"),
            &[(user_address.clone(), user_collateral_balance)],
        );

        // trying to liquidate user with zero outstanding debt should fail (uncollateralized has not impact)
        {
            let debt = Debt {
                amount_scaled: Uint256::zero(),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint256::from(10000u128),
                uncollateralized: true,
            };
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                    &debt,
                )
                .unwrap();
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (b"uncollateralized_debt", &user_address),
                    &uncollateralized_debt,
                )
                .unwrap();

            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(debt_contract_addr.as_str());
            let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
            assert_eq!(error_res, StdError::generic_err("User has no outstanding debt in the specified debt asset and thus cannot be liquidated").into());
        }

        // set user to have positive debt amount in debt asset
        {
            let debt = Debt {
                amount_scaled: expected_user_debt_scaled,
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint256::from(10000u128),
                uncollateralized: true,
            };
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                    &debt,
                )
                .unwrap();
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (b"uncollateralized_debt", &user_address),
                    &uncollateralized_debt,
                )
                .unwrap();
        }

        //  trying to liquidate without sending funds should fail
        {
            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: Uint128::zero(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(debt_contract_addr.as_str());
            let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
            assert_eq!(
                error_res,
                StdError::generic_err("Must send more than 0 debt in order to liquidate").into()
            );
        }

        // Perform first successful liquidation receiving ma_token in return
        {
            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = first_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info("debt");
            let res = execute(deps.as_mut(), env.clone(), info, liquidate_msg).unwrap();

            // get expected indices and rates for debt market
            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_initial,
                block_time,
                available_liquidity_debt,
                TestUtilizationDeltas {
                    less_debt: first_debt_to_repay.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_after = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            // TODO: not multiplying by collateral because it is a cw20 and Decimal::one
            // is the default price. Set a different price when implementing the oracle
            let expected_liquidated_collateral_amount = first_debt_to_repay
                * (Decimal256::one() + collateral_liquidation_bonus)
                / Decimal256::from(collateral_price);

            let expected_liquidated_collateral_amount_scaled = expected_liquidated_collateral_amount
                / get_updated_liquidity_index(&collateral_market_after, env.block.time.seconds());

            assert_eq!(
                res.messages,
                vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "ma_collateral".to_string(),
                    msg: to_binary(&mars::ma_token::msg::ExecuteMsg::TransferOnLiquidation {
                        sender: user_address.to_string(),
                        recipient: liquidator_address.to_string(),
                        amount: expected_liquidated_collateral_amount_scaled.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),]
            );

            mars::testing::assert_eq_vec(
                res.attributes,
                vec![
                    attr("action", "liquidate"),
                    attr("collateral_market", "collateral"),
                    attr("debt_market", debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount,
                    ),
                    attr("debt_amount_repaid", first_debt_to_repay),
                    attr("refund_amount", 0),
                    attr("borrow_index", expected_debt_rates.borrow_index),
                    attr("liquidity_index", expected_debt_rates.liquidity_index),
                    attr("borrow_rate", expected_debt_rates.borrow_rate),
                    attr("liquidity_rate", expected_debt_rates.liquidity_rate),
                ],
            );

            // check user still has deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(get_bit(user.collateral_assets, collateral_market_before.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_before.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let debt = DEBTS
                .load(
                    &deps.storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            let expected_less_debt_scaled = first_debt_to_repay / expected_debt_rates.borrow_index;

            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            assert_eq!(expected_user_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_debt_scaled = expected_global_debt_scaled - expected_less_debt_scaled;

            assert_eq!(
                expected_global_debt_scaled,
                debt_market_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                Uint256::zero(),
                collateral_market_after.protocol_income_to_distribute
            );
            assert_eq!(
                debt_market_before.protocol_income_to_distribute
                    + expected_debt_rates.protocol_income_to_distribute,
                debt_market_after.protocol_income_to_distribute
            );
        }

        // Perform second successful liquidation sending an excess amount (should refund)
        // and receive underlying collateral
        {
            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: false,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: second_debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = second_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info("debt");
            let res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_debt_indices = th_get_expected_indices(&debt_market_before, block_time);
            let user_debt_asset_total_debt =
                expected_user_debt_scaled * expected_debt_indices.borrow;
            // Since debt is being over_repayed, we expect to max out the liquidatable debt
            let expected_less_debt = user_debt_asset_total_debt * close_factor;

            let expected_refund_amount = second_debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_before,
                block_time,
                available_liquidity_debt, //this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_debt: expected_less_debt.into(),
                    less_liquidity: expected_refund_amount.into(),
                    ..Default::default()
                },
            );

            // TODO: not multiplying by collateral because it is a cw20 and Decimal::one
            // is the default price. Set a different price when implementing the oracle
            let expected_liquidated_collateral_amount = expected_less_debt
                * (Decimal256::one() + collateral_liquidation_bonus)
                / Decimal256::from(collateral_price);

            let expected_collateral_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &collateral_market_before,
                block_time,
                available_liquidity_collateral, //this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: expected_liquidated_collateral_amount.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_after = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            let expected_liquidated_collateral_amount_scaled =
                expected_liquidated_collateral_amount / expected_collateral_rates.liquidity_index;

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: "ma_collateral".to_string(),
                        msg: to_binary(&mars::ma_token::msg::ExecuteMsg::Burn {
                            user: user_address.to_string(),
                            amount: expected_liquidated_collateral_amount_scaled.into(),
                        })
                        .unwrap(),
                        funds: vec![]
                    })),
                    SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                        to_address: liquidator_address.to_string(),
                        amount: vec![deduct_tax(
                            deps.as_ref(),
                            Coin {
                                denom: String::from("collateral"),
                                amount: expected_liquidated_collateral_amount.into(),
                            }
                        )
                        .unwrap()],
                    })),
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: "debt".to_string(),
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            recipient: liquidator_address.to_string(),
                            amount: expected_refund_amount.into(),
                        })
                        .unwrap(),
                        funds: vec![]
                    })),
                ]
            );

            mars::testing::assert_eq_vec(
                vec![
                    attr("action", "liquidate"),
                    attr("collateral_market", "collateral"),
                    attr("debt_market", debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount,
                    ),
                    attr("debt_amount_repaid", expected_less_debt),
                    attr("refund_amount", expected_refund_amount),
                    attr("borrow_index", expected_debt_rates.borrow_index),
                    attr("liquidity_index", expected_debt_rates.liquidity_index),
                    attr("borrow_rate", expected_debt_rates.borrow_rate),
                    attr("liquidity_rate", expected_debt_rates.liquidity_rate),
                    attr("borrow_index", expected_collateral_rates.borrow_index),
                    attr("liquidity_index", expected_collateral_rates.liquidity_index),
                    attr("borrow_rate", expected_collateral_rates.borrow_rate),
                    attr("liquidity_rate", expected_collateral_rates.liquidity_rate),
                ],
                res.attributes,
            );

            // check user still has deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled = expected_less_debt / expected_debt_rates.borrow_index;
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt = DEBTS
                .load(
                    &deps.storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            assert_eq!(expected_user_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_debt_scaled = expected_global_debt_scaled - expected_less_debt_scaled;
            assert_eq!(
                expected_global_debt_scaled,
                debt_market_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                debt_market_before.protocol_income_to_distribute
                    + expected_debt_rates.protocol_income_to_distribute,
                debt_market_after.protocol_income_to_distribute
            );
            assert_eq!(
                expected_collateral_rates.protocol_income_to_distribute,
                collateral_market_after.protocol_income_to_distribute
            );
        }

        // Perform full liquidation receiving ma_token in return (user should not be able to use asset as collateral)
        {
            let user_collateral_balance_scaled = Uint256::from(100u128);
            let mut expected_user_debt_scaled = Uint256::from(400u128);
            let debt_to_repay = Uint256::from(300u128);

            // Set the querier to return positive collateral balance
            deps.querier.set_cw20_balances(
                Addr::unchecked("ma_collateral"),
                &[(user_address.clone(), user_collateral_balance_scaled.into())],
            );

            // set user to have positive debt amount in debt asset
            let debt = Debt {
                amount_scaled: expected_user_debt_scaled,
                uncollateralized: false,
            };
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                    &debt,
                )
                .unwrap();

            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.to_string(),
                    user_address: user_address.to_string(),
                    receive_ma_token: false,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = second_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info("debt");
            let res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_collateral_indices =
                th_get_expected_indices(&collateral_market_before, block_time);
            let user_collateral_balance =
                user_collateral_balance_scaled * expected_collateral_indices.liquidity;

            // Since debt is being over_repayed, we expect to liquidate total collateral
            let expected_less_debt = Decimal256::from(collateral_price) * user_collateral_balance
                / Decimal256::from(debt_price)
                / (Decimal256::one() + collateral_liquidation_bonus);

            let expected_refund_amount = debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_before,
                block_time,
                available_liquidity_debt, // this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_debt: expected_less_debt.into(),
                    less_liquidity: expected_refund_amount.into(),
                    ..Default::default()
                },
            );

            let expected_collateral_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &collateral_market_before,
                block_time,
                available_liquidity_collateral, // this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: user_collateral_balance.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_after = MARKETS
                .load(&deps.storage, debt_contract_addr.as_bytes())
                .unwrap();

            // NOTE: expected_liquidated_collateral_amount_scaled should be equal user_collateral_balance_scaled
            // but there are rounding errors
            let expected_liquidated_collateral_amount_scaled =
                user_collateral_balance / expected_collateral_rates.liquidity_index;

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: "ma_collateral".to_string(),
                        msg: to_binary(&mars::ma_token::msg::ExecuteMsg::Burn {
                            user: user_address.to_string(),
                            amount: expected_liquidated_collateral_amount_scaled.into(),
                        })
                        .unwrap(),
                        funds: vec![]
                    })),
                    SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                        to_address: liquidator_address.to_string(),
                        amount: vec![deduct_tax(
                            deps.as_ref(),
                            Coin {
                                denom: String::from("collateral"),
                                amount: user_collateral_balance.into(),
                            }
                        )
                        .unwrap()],
                    })),
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: "debt".to_string(),
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            recipient: liquidator_address.to_string(),
                            amount: expected_refund_amount.into(),
                        })
                        .unwrap(),
                        funds: vec![]
                    })),
                ]
            );

            mars::testing::assert_eq_vec(
                vec![
                    attr("action", "liquidate"),
                    attr("collateral_market", "collateral"),
                    attr("debt_market", debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr("collateral_amount_liquidated", user_collateral_balance),
                    attr("debt_amount_repaid", expected_less_debt),
                    attr("refund_amount", expected_refund_amount),
                    attr("borrow_index", expected_debt_rates.borrow_index),
                    attr("liquidity_index", expected_debt_rates.liquidity_index),
                    attr("borrow_rate", expected_debt_rates.borrow_rate),
                    attr("liquidity_rate", expected_debt_rates.liquidity_rate),
                    attr("borrow_index", expected_collateral_rates.borrow_index),
                    attr("liquidity_index", expected_collateral_rates.liquidity_index),
                    attr("borrow_rate", expected_collateral_rates.borrow_rate),
                    attr("liquidity_rate", expected_collateral_rates.liquidity_rate),
                ],
                res.attributes,
            );

            // check user doesn't have deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(!get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled = expected_less_debt / expected_debt_rates.borrow_index;
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt = DEBTS
                .load(
                    &deps.storage,
                    (debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            assert_eq!(expected_user_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_debt_scaled = expected_global_debt_scaled - expected_less_debt_scaled;
            assert_eq!(
                expected_global_debt_scaled,
                debt_market_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                expected_debt_rates.protocol_income_to_distribute,
                debt_market_after.protocol_income_to_distribute
                    - debt_market_before.protocol_income_to_distribute
            );
            assert_eq!(
                expected_collateral_rates.protocol_income_to_distribute,
                collateral_market_after.protocol_income_to_distribute
                    - collateral_market_before.protocol_income_to_distribute
            );
        }
    }

    #[test]
    fn test_liquidation_health_factor_check() {
        // initialize collateral and debt markets
        let available_liquidity_collateral = 1000000000u128;
        let available_liquidity_debt = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity_collateral, "collateral")]);

        let debt_contract_addr = Addr::unchecked("debt");
        deps.querier.set_cw20_balances(
            debt_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(available_liquidity_debt),
            )],
        );

        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[("collateral".to_string(), Decimal::one())],
        );

        let collateral_ltv = Decimal256::from_ratio(5, 10);
        let collateral_maintenance_margin = Decimal256::from_ratio(7, 10);
        let collateral_liquidation_bonus = Decimal256::from_ratio(1, 10);

        let collateral_market = Market {
            ma_token_address: Addr::unchecked("collateral"),
            max_loan_to_value: collateral_ltv,
            maintenance_margin: collateral_maintenance_margin,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let debt_market = Market {
            ma_token_address: Addr::unchecked("debt"),
            max_loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::from(20_000_000u64),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };

        // initialize markets
        let collateral_market_initial =
            th_init_market(deps.as_mut(), b"collateral", &collateral_market);

        let debt_market_initial =
            th_init_market(deps.as_mut(), debt_contract_addr.as_bytes(), &debt_market);

        // test health factor check
        let healthy_user_address = Addr::unchecked("healthy_user");

        // Set user as having collateral and debt in respective markets
        let mut healthy_user = User::default();

        set_bit(
            &mut healthy_user.collateral_assets,
            collateral_market_initial.index,
        )
        .unwrap();
        set_bit(&mut healthy_user.borrowed_assets, debt_market_initial.index).unwrap();

        USERS
            .save(deps.as_mut().storage, &healthy_user_address, &healthy_user)
            .unwrap();

        // set initial collateral and debt balances for user
        let collateral_address = Addr::unchecked("collateral");
        let healthy_user_collateral_balance = Uint128::new(10_000_000);

        // Set the querier to return a certain collateral balance
        deps.querier.set_cw20_balances(
            collateral_address,
            &[(
                healthy_user_address.clone(),
                healthy_user_collateral_balance,
            )],
        );

        let healthy_user_debt_amount =
            Uint256::from(healthy_user_collateral_balance) * collateral_maintenance_margin;
        let healthy_user_debt = Debt {
            amount_scaled: healthy_user_debt_amount,
            uncollateralized: false,
        };
        let uncollateralized_debt = Debt {
            amount_scaled: Uint256::from(10000u128),
            uncollateralized: true,
        };
        DEBTS
            .save(
                deps.as_mut().storage,
                (debt_contract_addr.as_bytes(), &healthy_user_address),
                &healthy_user_debt,
            )
            .unwrap();
        DEBTS
            .save(
                deps.as_mut().storage,
                (b"uncollateralized_debt", &healthy_user_address),
                &uncollateralized_debt,
            )
            .unwrap();

        // perform liquidation (should fail because health factor is > 1)
        let liquidator_address = Addr::unchecked("liquidator");
        let debt_to_cover = Uint256::from(1_000_000u64);

        let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                collateral_asset: Asset::Native {
                    denom: "collateral".to_string(),
                },
                debt_asset_address: debt_contract_addr.to_string(),
                user_address: healthy_user_address.to_string(),
                receive_ma_token: true,
            })
            .unwrap(),
            sender: liquidator_address.to_string(),
            amount: debt_to_cover.into(),
        });

        let env = mock_env(MockEnvParams::default());
        let info = mock_info(debt_contract_addr.as_str());
        let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "User's health factor is not less than 1 and thus cannot be liquidated"
            )
            .into()
        );
    }

    #[test]
    fn test_finalize_liquidity_token_transfer() {
        // Setup
        let mut deps = th_setup(&[]);
        let env = mock_env(MockEnvParams::default());
        let info_matoken = mock_info("masomecoin");

        let mock_market = Market {
            ma_token_address: Addr::unchecked("masomecoin"),
            liquidity_index: Decimal256::one(),
            maintenance_margin: Decimal256::from_ratio(5, 10),
            ..Default::default()
        };
        let market = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        let debt_mock_market = Market {
            borrow_index: Decimal256::one(),
            ..Default::default()
        };
        let debt_market = th_init_market(deps.as_mut(), b"debtcoin", &debt_mock_market);

        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[
                ("somecoin".to_string(), Decimal::from_ratio(1u128, 2u128)),
                ("debtcoin".to_string(), Decimal::from_ratio(2u128, 1u128)),
            ],
        );

        let sender_address = Addr::unchecked("fromaddr");
        let recipient_address = Addr::unchecked("toaddr");

        deps.querier.set_cw20_balances(
            Addr::unchecked("masomecoin"),
            &[(sender_address.clone(), Uint128::new(500_000))],
        );

        {
            let mut sender_user = User::default();
            set_bit(&mut sender_user.collateral_assets, market.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &sender_address, &sender_user)
                .unwrap();
        }

        // Finalize transfer with sender not borrowing passes
        {
            let msg = ExecuteMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.to_string(),
                recipient_address: recipient_address.to_string(),
                sender_previous_balance: Uint128::new(1_000_000),
                recipient_previous_balance: Uint128::new(0),
                amount: Uint128::new(500_000),
            };

            execute(deps.as_mut(), env.clone(), info_matoken.clone(), msg).unwrap();

            let sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            let recipient_user = USERS.load(&deps.storage, &recipient_address).unwrap();
            assert!(get_bit(sender_user.collateral_assets, market.index).unwrap());
            // Should create user and set deposited to true as previous balance is 0
            assert!(get_bit(recipient_user.collateral_assets, market.index).unwrap());
        }

        // Finalize transfer with health factor < 1 for sender doesn't go through
        {
            // set debt for user in order for health factor to be < 1
            let debt = Debt {
                amount_scaled: Uint256::from(500_000u128),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint256::from(10000u128),
                uncollateralized: true,
            };
            DEBTS
                .save(deps.as_mut().storage, (b"debtcoin", &sender_address), &debt)
                .unwrap();
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (b"uncollateralized_debt", &sender_address),
                    &uncollateralized_debt,
                )
                .unwrap();
            let mut sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_market.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &sender_address, &sender_user)
                .unwrap();
        }

        {
            let msg = ExecuteMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.to_string(),
                recipient_address: recipient_address.to_string(),
                sender_previous_balance: Uint128::new(1_000_000),
                recipient_previous_balance: Uint128::new(0),
                amount: Uint128::new(500_000),
            };

            let error_res =
                execute(deps.as_mut(), env.clone(), info_matoken.clone(), msg).unwrap_err();
            assert_eq!(error_res, StdError::generic_err("Cannot make token transfer if it results in a health factor lower than 1 for the sender").into());
        }

        // Finalize transfer with health factor > 1 for goes through
        {
            // set debt for user in order for health factor to be > 1
            let debt = Debt {
                amount_scaled: Uint256::from(1_000u128),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint256::from(10000u128),
                uncollateralized: true,
            };
            DEBTS
                .save(deps.as_mut().storage, (b"debtcoin", &sender_address), &debt)
                .unwrap();
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (b"uncollateralized_debt", &sender_address),
                    &uncollateralized_debt,
                )
                .unwrap();
            let mut sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_market.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &sender_address, &sender_user)
                .unwrap();
        }

        {
            let msg = ExecuteMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.to_string(),
                recipient_address: recipient_address.to_string(),
                sender_previous_balance: Uint128::new(500_000),
                recipient_previous_balance: Uint128::new(500_000),
                amount: Uint128::new(500_000),
            };

            execute(deps.as_mut(), env.clone(), info_matoken, msg).unwrap();

            let sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            let recipient_user = USERS.load(&deps.storage, &recipient_address).unwrap();
            // Should set deposited to false as: previous_balance - amount = 0
            assert!(!get_bit(sender_user.collateral_assets, market.index).unwrap());
            assert!(get_bit(recipient_user.collateral_assets, market.index).unwrap());
        }

        // Calling this with other token fails
        {
            let msg = ExecuteMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.to_string(),
                recipient_address: recipient_address.to_string(),
                sender_previous_balance: Uint128::new(500_000),
                recipient_previous_balance: Uint128::new(500_000),
                amount: Uint128::new(500_000),
            };
            let info = mock_info("othertoken");

            let error_res = execute(deps.as_mut(), env, info, msg).unwrap_err();
            assert_eq!(error_res, StdError::not_found("alloc::vec::Vec<u8>").into());
        }
    }

    #[test]
    fn test_uncollateralized_loan_limits() {
        let available_liquidity = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128::new(100u128))],
        );

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);

        let borrower_addr = Addr::unchecked("borrower");

        let mut block_time = mock_market.interests_last_updated + 10000u64;
        let initial_uncollateralized_loan_limit = Uint128::from(2400_u128);

        // Update uncollateralized loan limit
        let update_limit_msg = ExecuteMsg::UpdateUncollateralizedLoanLimit {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            user_address: borrower_addr.to_string(),
            new_limit: initial_uncollateralized_loan_limit,
        };

        // update limit as unauthorized user, should fail
        let update_limit_env = mock_env_at_block_time(block_time);
        let info = mock_info("random");
        let error_res = execute(
            deps.as_mut(),
            update_limit_env.clone(),
            info,
            update_limit_msg.clone(),
        )
        .unwrap_err();
        assert_eq!(error_res, MarsError::Unauthorized {}.into());

        // Update borrower limit as owner
        let info = mock_info("owner");
        execute(deps.as_mut(), update_limit_env, info, update_limit_msg).unwrap();

        // check user's limit has been updated to the appropriate amount
        let limit = UNCOLLATERALIZED_LOAN_LIMITS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();
        assert_eq!(limit, initial_uncollateralized_loan_limit);

        // check user's uncollateralized debt flag is true (limit > 0)
        let debt = DEBTS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();
        assert!(debt.uncollateralized);

        // Borrow asset
        block_time += 1000_u64;
        let initial_borrow_amount =
            initial_uncollateralized_loan_limit.multiply_ratio(1_u64, 2_u64);
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(initial_borrow_amount),
        };
        let borrow_env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        let res = execute(deps.as_mut(), borrow_env, info, borrow_msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps.as_ref(),
            &market_initial,
            block_time,
            available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: initial_borrow_amount.into(),
                more_debt: initial_borrow_amount.into(),
                ..Default::default()
            },
        );

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                to_address: borrower_addr.to_string(),
                amount: vec![deduct_tax(
                    deps.as_ref(),
                    Coin {
                        denom: String::from("somecoin"),
                        amount: initial_borrow_amount,
                    }
                )
                .unwrap()],
            }))]
        );

        assert_eq!(
            res.attributes,
            vec![
                attr("action", "borrow"),
                attr("market", "somecoin"),
                attr("user", "borrower"),
                attr("amount", initial_borrow_amount),
                attr("borrow_index", expected_params.borrow_index),
                attr("liquidity_index", expected_params.liquidity_index),
                attr("borrow_rate", expected_params.borrow_rate),
                attr("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        // Check debt
        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());

        let debt = DEBTS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();

        let expected_debt_scaled_after_borrow =
            Uint256::from(initial_borrow_amount) / expected_params.borrow_index;

        assert_eq!(expected_debt_scaled_after_borrow, debt.amount_scaled);

        // Borrow an amount less than initial limit but exceeding current limit
        let remaining_limit = initial_uncollateralized_loan_limit - initial_borrow_amount;
        let exceeding_limit = remaining_limit + Uint128::from(100_u64);

        block_time += 1000_u64;
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(exceeding_limit),
        };
        let borrow_env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        let error_res = execute(deps.as_mut(), borrow_env, info, borrow_msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(
                "borrow amount exceeds uncollateralized loan limit given existing debt"
            )
            .into()
        );

        // Borrow a valid amount given uncollateralized loan limit
        block_time += 1000_u64;
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(remaining_limit),
        };
        let borrow_env = mock_env_at_block_time(block_time);
        let info = mock_info("borrower");
        execute(deps.as_mut(), borrow_env, info, borrow_msg).unwrap();

        // Set limit to zero
        let update_allowance_msg = ExecuteMsg::UpdateUncollateralizedLoanLimit {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            user_address: borrower_addr.to_string(),
            new_limit: Uint128::zero(),
        };
        let allowance_env = mock_env_at_block_time(block_time);
        let info = mock_info("owner");
        execute(deps.as_mut(), allowance_env, info, update_allowance_msg).unwrap();

        // check user's allowance is zero
        let allowance = UNCOLLATERALIZED_LOAN_LIMITS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();
        assert_eq!(allowance, Uint128::zero());

        // check user's uncollateralized debt flag is false (limit == 0)
        let debt = DEBTS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();
        assert!(!debt.uncollateralized);
    }

    #[test]
    fn test_update_asset_collateral() {
        let mut deps = th_setup(&[]);

        let user_addr = Addr::unchecked(String::from("user"));

        let ma_token_address_1 = Addr::unchecked("matoken1");
        let mock_market_1 = Market {
            ma_token_address: ma_token_address_1.clone(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: Addr::unchecked("matoken2"),
            ..Default::default()
        };
        let cw20_contract_addr = Addr::unchecked("depositedcoin1");

        // Should get index 0
        let market_1_initial =
            th_init_market(deps.as_mut(), cw20_contract_addr.as_bytes(), &mock_market_1);
        // Should get index 1
        let market_2_initial = th_init_market(deps.as_mut(), b"depositedcoin2", &mock_market_2);

        // Set second asset as collateral
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &user_addr, &user)
            .unwrap();

        // Set the querier to return zero for the first asset
        deps.querier.set_cw20_balances(
            ma_token_address_1.clone(),
            &[(user_addr.clone(), Uint128::zero())],
        );

        // Enable first market index which is currently disabled as collateral and ma-token balance is 0
        let update_msg = ExecuteMsg::UpdateUserCollateralAssetStatus {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.to_string(),
            },
            enable: true,
        };
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("user");
        let error_res =
            execute(deps.as_mut(), env.clone(), info.clone(), update_msg.clone()).unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err(format!(
                "User address {} has no balance in specified collateral asset {}",
                user_addr.as_str(),
                String::from(cw20_contract_addr.as_str())
            ))
            .into()
        );

        let user = USERS.load(&deps.storage, &user_addr).unwrap();
        let market_1_collateral = get_bit(user.collateral_assets, market_1_initial.index).unwrap();
        // Balance for first asset is zero so don't update bit
        assert!(!market_1_collateral);

        // Set the querier to return balance more than zero for the first asset
        deps.querier.set_cw20_balances(
            ma_token_address_1,
            &[(user_addr.clone(), Uint128::new(100_000))],
        );

        // Enable first market index which is currently disabled as collateral and ma-token balance is more than 0
        let _res = execute(deps.as_mut(), env.clone(), info.clone(), update_msg).unwrap();
        let user = USERS.load(&deps.storage, &user_addr).unwrap();
        let market_1_collateral = get_bit(user.collateral_assets, market_1_initial.index).unwrap();
        // Balance for first asset is more than zero so update bit
        assert!(market_1_collateral);

        // Disable second market index
        let update_msg = ExecuteMsg::UpdateUserCollateralAssetStatus {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            enable: false,
        };
        let _res = execute(deps.as_mut(), env, info, update_msg).unwrap();
        let user = USERS.load(&deps.storage, &user_addr).unwrap();
        let market_2_collateral = get_bit(user.collateral_assets, market_2_initial.index).unwrap();
        assert!(!market_2_collateral);
    }

    #[test]
    fn test_distribute_protocol_income() {
        // initialize contract with liquidity
        let available_liquidity = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128::new(100u128))],
        );

        let asset = Asset::Native {
            denom: String::from("somecoin"),
        };
        let protocol_income_to_distribute = Uint256::from(1_000_000_u64);

        // initialize market with non-zero amount of protocol_income_to_distribute
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            protocol_income_to_distribute,
            ..Default::default()
        };
        // should get index 0
        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);

        let mut block_time = mock_market.interests_last_updated + 10000u64;

        // call function providing amount exceeding protocol_income_to_distribute, should fail
        let exceeding_amount = protocol_income_to_distribute + Uint256::from(1_000_u64);
        let distribute_income_msg = ExecuteMsg::DistributeProtocolIncome {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(exceeding_amount),
        };
        let env = mock_env_at_block_time(block_time);
        let info = mock_info("anyone");

        let error_res = execute(
            deps.as_mut(),
            env.clone(),
            info.clone(),
            distribute_income_msg,
        )
        .unwrap_err();
        assert_eq!(
            error_res,
            StdError::generic_err("amount specified exceeds market's income to be distributed")
                .into()
        );

        // call function providing amount less than protocol_income_to_distribute
        let permissible_amount = Decimal256::from_ratio(1, 2) * protocol_income_to_distribute;
        let distribute_income_msg = ExecuteMsg::DistributeProtocolIncome {
            asset: asset.clone(),
            amount: Some(permissible_amount),
        };
        let res = execute(deps.as_mut(), env.clone(), info, distribute_income_msg).unwrap();

        let config = CONFIG.load(&deps.storage).unwrap();
        let market_after_distribution = MARKETS.load(&deps.storage, b"somecoin").unwrap();

        let expected_insurance_fund_amount = permissible_amount * config.insurance_fund_fee_share;
        let expected_treasury_amount = permissible_amount * config.treasury_fee_share;
        let expected_staking_amount =
            permissible_amount - (expected_insurance_fund_amount + expected_treasury_amount);

        let scaled_mint_amount = expected_treasury_amount
            / get_updated_liquidity_index(&market_initial, env.block.time.seconds());

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: "insurance_fund".to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_insurance_fund_amount.into(),
                        }
                    )
                    .unwrap()],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: market_initial.ma_token_address.to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Mint {
                        recipient: "treasury".to_string(),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: "staking".to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_staking_amount.into(),
                        }
                    )
                    .unwrap()],
                }))
            ]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "distribute_protocol_income"),
                attr("asset", "somecoin"),
                attr("amount", permissible_amount),
            ]
        );

        let expected_remaining_income_to_be_distributed =
            protocol_income_to_distribute - permissible_amount;
        assert_eq!(
            market_after_distribution.protocol_income_to_distribute,
            expected_remaining_income_to_be_distributed
        );

        // call function without providing an amount, should send full remaining amount to contracts
        block_time += 1000;
        let env = mock_env_at_block_time(block_time);
        let info = mock_info("anyone");
        let distribute_income_msg = ExecuteMsg::DistributeProtocolIncome {
            asset,
            amount: None,
        };
        let res = execute(deps.as_mut(), env.clone(), info, distribute_income_msg).unwrap();

        // verify messages are correct and protocol_income_to_distribute field is now zero
        let expected_insurance_amount =
            expected_remaining_income_to_be_distributed * config.insurance_fund_fee_share;
        let expected_treasury_amount =
            expected_remaining_income_to_be_distributed * config.treasury_fee_share;
        let expected_staking_amount = expected_remaining_income_to_be_distributed
            - (expected_insurance_amount + expected_treasury_amount);

        let scaled_mint_amount = expected_treasury_amount
            / get_updated_liquidity_index(&market_after_distribution, env.block.time.seconds());

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: "insurance_fund".to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_insurance_fund_amount.into(),
                        }
                    )
                    .unwrap()],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: market_initial.ma_token_address.to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Mint {
                        recipient: "treasury".to_string(),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                    funds: vec![]
                })),
                SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                    to_address: "staking".to_string(),
                    amount: vec![deduct_tax(
                        deps.as_ref(),
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_staking_amount.into(),
                        }
                    )
                    .unwrap()],
                }))
            ]
        );
        assert_eq!(
            res.attributes,
            vec![
                attr("action", "distribute_protocol_income"),
                attr("asset", "somecoin"),
                attr("amount", expected_remaining_income_to_be_distributed),
            ]
        );

        let market_after_second_distribution = MARKETS.load(&deps.storage, b"somecoin").unwrap();
        assert_eq!(
            market_after_second_distribution.protocol_income_to_distribute,
            Uint256::zero()
        );
    }

    #[test]
    fn test_query_collateral() {
        let mut deps = th_setup(&[]);

        let user_addr = Addr::unchecked("user");

        // Setup first market containing a CW20 asset
        let cw20_contract_addr_1 = Addr::unchecked("depositedcoin1");
        deps.querier
            .set_cw20_symbol(cw20_contract_addr_1.clone(), "DP1".to_string());
        let market_1_initial = th_init_market(
            deps.as_mut(),
            cw20_contract_addr_1.as_bytes(),
            &Market {
                asset_type: AssetType::Cw20,
                ..Default::default()
            },
        );

        // Setup second market containing a native asset
        let market_2_initial = th_init_market(
            deps.as_mut(),
            String::from("uusd").as_bytes(),
            &Market {
                ..Default::default()
            },
        );

        // Set second market as collateral
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &user_addr, &user)
            .unwrap();

        // Assert markets correctly return collateral status
        let res = query_collateral(deps.as_ref(), user_addr.clone()).unwrap();
        assert_eq!(res.collateral[0].denom, String::from("DP1"));
        assert!(!res.collateral[0].enabled);
        assert_eq!(res.collateral[1].denom, String::from("uusd"));
        assert!(res.collateral[1].enabled);

        // Set first market as collateral
        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &user_addr, &user)
            .unwrap();

        // Assert markets correctly return collateral status
        let res = query_collateral(deps.as_ref(), user_addr).unwrap();
        assert_eq!(res.collateral[0].denom, String::from("DP1"));
        assert!(res.collateral[0].enabled);
        assert_eq!(res.collateral[1].denom, String::from("uusd"));
        assert!(res.collateral[1].enabled);
    }

    // TEST HELPERS

    fn th_setup(contract_balances: &[Coin]) -> OwnedDeps<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(contract_balances);
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("owner");
        let config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(1u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InstantiateMsg { config };
        instantiate(deps.as_mut(), env, info, msg).unwrap();
        deps
    }

    impl Default for Market {
        fn default() -> Self {
            Market {
                index: 0,
                ma_token_address: zero_address(),
                liquidity_index: Default::default(),
                borrow_index: Default::default(),
                borrow_rate: Default::default(),
                min_borrow_rate: Decimal256::zero(),
                max_borrow_rate: Decimal256::one(),
                liquidity_rate: Default::default(),
                max_loan_to_value: Default::default(),
                reserve_factor: Default::default(),
                interests_last_updated: 0,
                debt_total_scaled: Default::default(),
                asset_type: AssetType::Native,
                maintenance_margin: Decimal256::one(),
                liquidation_bonus: Decimal256::zero(),
                protocol_income_to_distribute: Uint256::zero(),
                pid_parameters: PidParameters {
                    kp_1: Default::default(),
                    optimal_utilization_rate: Default::default(),
                    kp_augmentation_threshold: Default::default(),
                    kp_2: Default::default(),
                },
            }
        }
    }

    fn th_init_market(deps: DepsMut, key: &[u8], market: &Market) -> Market {
        let mut index = 0;

        RED_BANK
            .update(deps.storage, |mut mm: RedBank| -> StdResult<RedBank> {
                index = mm.market_count;
                mm.market_count += 1;
                Ok(mm)
            })
            .unwrap();

        let new_market = Market {
            index,
            ..market.clone()
        };

        MARKETS.save(deps.storage, key, &new_market).unwrap();

        MARKET_REFERENCES
            .save(
                deps.storage,
                U32Key::new(index),
                &MarketReferences {
                    reference: key.to_vec(),
                },
            )
            .unwrap();

        MARKET_MA_TOKENS
            .save(deps.storage, &new_market.ma_token_address, &key.to_vec())
            .unwrap();

        new_market
    }

    #[derive(Default, Debug)]
    struct TestInterestResults {
        borrow_index: Decimal256,
        liquidity_index: Decimal256,
        borrow_rate: Decimal256,
        liquidity_rate: Decimal256,
        protocol_income_to_distribute: Uint256,
    }

    /// Deltas to be using in expected indices/rates results
    #[derive(Default, Debug)]
    struct TestUtilizationDeltas {
        less_liquidity: u128,
        more_debt: u128,
        less_debt: u128,
    }

    /// Takes a market before an action (ie: a borrow) among some test parameters
    /// used in that action and computes the expected indices and rates after that action.
    fn th_get_expected_indices_and_rates(
        deps: &Deps,
        market: &Market,
        block_time: u64,
        initial_liquidity: u128,
        deltas: TestUtilizationDeltas,
    ) -> TestInterestResults {
        let expected_indices = th_get_expected_indices(market, block_time);

        // Compute protocol income to be distributed (using values up to the instant
        // before the contract call is made)
        let previous_borrow_index = market.borrow_index;
        let previous_debt_total = market.debt_total_scaled * previous_borrow_index;
        let current_debt_total = market.debt_total_scaled * expected_indices.borrow;
        let interest_accrued = if current_debt_total > previous_debt_total {
            current_debt_total - previous_debt_total
        } else {
            Uint256::zero()
        };
        let expected_protocol_income_to_distribute = interest_accrued * market.reserve_factor;

        // When borrowing, new computed index is used for scaled amount
        let more_debt_scaled = Uint256::from(deltas.more_debt) / expected_indices.borrow;
        // When repaying, new computed index is used for scaled amount
        let less_debt_scaled = Uint256::from(deltas.less_debt) / expected_indices.borrow;
        // NOTE: Don't panic here so that the total repay of debt can be simulated
        // when less debt is greater than outstanding debt
        let new_debt_total_scaled =
            if (market.debt_total_scaled + more_debt_scaled) > less_debt_scaled {
                market.debt_total_scaled + more_debt_scaled - less_debt_scaled
            } else {
                Uint256::zero()
            };
        let dec_debt_total =
            Decimal256::from_uint256(new_debt_total_scaled) * expected_indices.borrow;
        let total_protocol_income_to_distribute =
            market.protocol_income_to_distribute + expected_protocol_income_to_distribute;

        let config = CONFIG.load(deps.storage).unwrap();

        let dec_protocol_income_minus_treasury_amount =
            (Decimal256::one() - config.treasury_fee_share) * total_protocol_income_to_distribute;
        let contract_current_balance = Uint256::from(initial_liquidity);
        let liquidity_taken = Uint256::from(deltas.less_liquidity);
        let dec_liquidity_total = Decimal256::from_uint256(
            contract_current_balance - liquidity_taken - dec_protocol_income_minus_treasury_amount,
        );
        let expected_utilization_rate = dec_debt_total / (dec_liquidity_total + dec_debt_total);

        // interest rates
        let (expected_borrow_rate, expected_liquidity_rate) =
            get_updated_interest_rates(market, expected_utilization_rate);

        TestInterestResults {
            borrow_index: expected_indices.borrow,
            liquidity_index: expected_indices.liquidity,
            borrow_rate: expected_borrow_rate,
            liquidity_rate: expected_liquidity_rate,
            protocol_income_to_distribute: expected_protocol_income_to_distribute,
        }
    }

    /// Expected results for applying accumulated interest
    struct TestExpectedIndices {
        liquidity: Decimal256,
        borrow: Decimal256,
    }

    fn th_get_expected_indices(market: &Market, block_time: u64) -> TestExpectedIndices {
        let seconds_elapsed = block_time - market.interests_last_updated;
        // market indices
        let expected_liquidity_index = calculate_applied_linear_interest_rate(
            market.liquidity_index,
            market.liquidity_rate,
            seconds_elapsed,
        );

        let expected_borrow_index = calculate_applied_linear_interest_rate(
            market.borrow_index,
            market.borrow_rate,
            seconds_elapsed,
        );

        TestExpectedIndices {
            liquidity: expected_liquidity_index,
            borrow: expected_borrow_index,
        }
    }
}
