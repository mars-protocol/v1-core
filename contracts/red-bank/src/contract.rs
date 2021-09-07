use std::str;

use cosmwasm_std::{
    entry_point, from_binary, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps,
    DepsMut, Env, Event, MessageInfo, Order, Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use cw20_base::msg::InstantiateMarketingInfo;
use cw_storage_plus::U32Key;

use mars::address_provider;
use mars::address_provider::msg::MarsContract;
use mars::ma_token;
use mars::red_bank::{
    msg::{
        AmountResponse, CollateralInfo, CollateralResponse, ConfigResponse, CreateOrUpdateConfig,
        DebtInfo, DebtResponse, ExecuteMsg, InitOrUpdateAssetParams, InstantiateMsg, MarketInfo,
        MarketResponse, MarketsListResponse, QueryMsg, ReceiveMsg,
        UncollateralizedLoanLimitResponse, UserPositionResponse,
    },
    UserHealthStatus,
};

use mars::asset::{Asset, AssetType};
use mars::error::MarsError;
use mars::helpers::{cw20_get_balance, cw20_get_symbol, option_string_to_addr, zero_address};
use mars::tax::deduct_tax;

use crate::accounts::get_user_position;
use crate::error::ContractError;
use crate::interest_rate::{
    apply_accumulated_interests, get_descaled_amount, get_scaled_amount, get_updated_borrow_index,
    get_updated_liquidity_index, update_interest_rates,
};
use crate::state::{
    Config, Debt, GlobalState, Market, User, CONFIG, DEBTS, GLOBAL_STATE, MARKETS,
    MARKET_REFERENCES_BY_INDEX, MARKET_REFERENCES_BY_MA_TOKEN, UNCOLLATERALIZED_LOAN_LIMITS, USERS,
};
use mars::math::reverse_decimal;

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

    GLOBAL_STATE.save(deps.storage, &GlobalState { market_count: 0 })?;

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

        ExecuteMsg::InitAssetTokenCallback { reference } => {
            execute_init_asset_token_callback(deps, env, info, reference)
        }

        ExecuteMsg::UpdateAsset {
            asset,
            asset_params,
        } => execute_update_asset(deps, env, info, asset, asset_params),

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
            debt_asset_denom,
            user_address,
            receive_ma_token,
        } => {
            let sender = info.sender.clone();
            let user_addr = deps.api.addr_validate(&user_address)?;
            let sent_debt_asset_amount =
                get_denom_amount_from_coins(&info.funds, &debt_asset_denom);
            execute_liquidate(
                deps,
                env,
                info,
                sender,
                collateral_asset,
                Asset::Native {
                    denom: debt_asset_denom,
                },
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
        } => execute_finalize_liquidity_token_transfer(
            deps,
            env,
            info,
            sender_address,
            recipient_address,
            sender_previous_balance,
            recipient_previous_balance,
            amount,
        ),

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
                cw20_msg.amount,
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
                cw20_msg.amount,
                AssetType::Cw20,
            )
        }
        ReceiveMsg::LiquidateCw20 {
            collateral_asset,
            user_address,
            receive_ma_token,
        } => {
            let debt_asset_addr = info.sender.clone();
            let liquidator_addr = deps.api.addr_validate(&cw20_msg.sender)?;
            let user_addr = deps.api.addr_validate(&user_address)?;
            execute_liquidate(
                deps,
                env,
                info,
                liquidator_addr,
                collateral_asset,
                Asset::Cw20 {
                    contract_addr: debt_asset_addr.to_string(),
                },
                user_addr,
                cw20_msg.amount,
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
    amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    let withdrawer_addr = info.sender;

    let (asset_label, asset_reference, _asset_type) = asset.get_attributes();
    let mut market = MARKETS.load(deps.storage, asset_reference.as_slice())?;

    let asset_ma_addr = market.ma_token_address.clone();
    let withdrawer_balance_scaled =
        cw20_get_balance(&deps.querier, asset_ma_addr, withdrawer_addr.clone())?;

    if withdrawer_balance_scaled.is_zero() {
        return Err(StdError::generic_err(
            format!("User has no balance (asset: {})", asset_label,),
        )
        .into());
    }

    // Check user has sufficient balance to send back
    let (withdraw_amount, withdraw_amount_scaled) = match amount {
        Some(amount) => {
            let amount_scaled = get_scaled_amount(
                amount,
                get_updated_liquidity_index(&market, env.block.time.seconds()),
            );
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
            let withdrawer_balance = get_descaled_amount(
                withdrawer_balance_scaled,
                get_updated_liquidity_index(&market, env.block.time.seconds()),
            );
            (withdrawer_balance, withdrawer_balance_scaled)
        }
    };

    let mut withdrawer = USERS.load(deps.storage, &withdrawer_addr)?;
    let asset_as_collateral = get_bit(withdrawer.collateral_assets, market.index)?;
    let user_is_borrowing = !withdrawer.borrowed_assets.is_zero();

    // if asset is used as collateral and user is borrowing we need to validate health factor after withdraw,
    // otherwise no reasons to block the withdraw
    if asset_as_collateral && user_is_borrowing {
        let global_state = GLOBAL_STATE.load(deps.storage)?;
        let config = CONFIG.load(deps.storage)?;

        let oracle_address = address_provider::helpers::query_address(
            &deps.querier,
            config.address_provider_address,
            MarsContract::Oracle,
        )?;

        let user_position = get_user_position(
            deps.as_ref(),
            env.block.time.seconds(),
            &withdrawer_addr,
            oracle_address,
            &withdrawer,
            global_state.market_count,
        )?;

        let withdraw_asset_price =
            user_position.get_asset_price(asset_reference.as_slice(), &asset_label)?;

        let withdraw_amount_in_uusd = withdraw_amount * withdraw_asset_price;

        let health_factor_after_withdraw = Decimal::from_ratio(
            user_position.weighted_maintenance_margin_in_uusd
                - (withdraw_amount_in_uusd * market.maintenance_margin),
            user_position.total_collateralized_debt_in_uusd,
        );
        if health_factor_after_withdraw < Decimal::one() {
            return Err(StdError::generic_err(
                "User's health factor can't be less than 1 after withdraw",
            )
            .into());
        }
    }

    let mut events = vec![];
    // if amount to withdraw equals the user's balance then unset collateral bit
    if asset_as_collateral && withdraw_amount_scaled == withdrawer_balance_scaled {
        unset_bit(&mut withdrawer.collateral_assets, market.index)?;
        USERS.save(deps.storage, &withdrawer_addr, &withdrawer)?;
        events.push(build_collateral_position_changed_event(
            asset_label.as_str(),
            false,
            withdrawer_addr.to_string(),
        ));
    }

    apply_accumulated_interests(&env, &mut market);
    update_interest_rates(
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
            amount: withdraw_amount_scaled,
        })?,
        funds: vec![],
    });

    let send_underlying_asset_msg = build_send_asset_msg(
        deps.as_ref(),
        env.contract.address,
        withdrawer_addr.clone(),
        asset,
        withdraw_amount,
    )?;

    events.push(build_interests_updated_event(asset_label.as_str(), &market));

    let res = Response::new()
        .add_attribute("action", "withdraw")
        .add_attribute("market", asset_label.as_str())
        .add_attribute("user", withdrawer_addr.as_str())
        .add_attribute("burn_amount", withdraw_amount_scaled)
        .add_attribute("withdraw_amount", withdraw_amount)
        .add_message(burn_ma_tokens_msg)
        .add_message(send_underlying_asset_msg)
        .add_events(events);
    Ok(res)
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

    let mut money_market = GLOBAL_STATE.load(deps.storage)?;

    let (asset_label, asset_reference, asset_type) = asset.get_attributes();
    let market_option = MARKETS.may_load(deps.storage, asset_reference.as_slice())?;
    match market_option {
        None => {
            let market_idx = money_market.market_count;
            let new_market = Market::create(env.block.time, market_idx, asset_type, asset_params)?;

            // Save new market
            MARKETS.save(deps.storage, asset_reference.as_slice(), &new_market)?;

            // Save index to reference mapping
            MARKET_REFERENCES_BY_INDEX.save(
                deps.storage,
                U32Key::new(market_idx),
                &asset_reference.to_vec(),
            )?;

            // Increment market count
            money_market.market_count += 1;
            GLOBAL_STATE.save(deps.storage, &money_market)?;

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
            let mut addresses_query = address_provider::helpers::query_addresses(
                &deps.querier,
                config.address_provider_address,
                vec![MarsContract::Incentives, MarsContract::ProtocolAdmin],
            )?;

            let protocol_admin_address = addresses_query.pop().unwrap();
            let incentives_address = addresses_query.pop().unwrap();

            let res = Response::new()
                .add_attribute("action", "init_asset")
                .add_attribute("asset", asset_label)
                .add_message(CosmosMsg::Wasm(WasmMsg::Instantiate {
                    admin: Some(protocol_admin_address.to_string()),
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
                        marketing: Some(InstantiateMarketingInfo {
                            project: Some(String::from("Mars Protocol")),
                            description: Some(format!(
                                "Interest earning token representing deposits for {}",
                                symbol
                            )),
                            marketing: Some(protocol_admin_address.to_string()),
                            logo: None,
                        }),
                        init_hook: Some(ma_token::msg::InitHook {
                            contract_addr: env.contract.address.to_string(),
                            msg: to_binary(&ExecuteMsg::InitAssetTokenCallback {
                                reference: asset_reference,
                            })?,
                        }),
                        red_bank_address: env.contract.address.to_string(),
                        incentives_address: incentives_address.into(),
                    })?,
                    funds: vec![],
                    label: String::from(""),
                }));
            Ok(res)
        }
        Some(_) => Err(StdError::generic_err("Asset already initialized").into()),
    }
}

pub fn execute_init_asset_token_callback(
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
        MARKET_REFERENCES_BY_MA_TOKEN.save(deps.storage, &ma_contract_addr, &reference)?;

        Ok(Response::default())
    } else {
        // Can do this only once
        Err(MarsError::Unauthorized {}.into())
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

    let (asset_label, asset_reference, _asset_type) = asset.get_attributes();
    let market_option = MARKETS.may_load(deps.storage, asset_reference.as_slice())?;
    match market_option {
        Some(market) => {
            let updated_market = market.update_with(asset_params)?;

            // Save updated market
            MARKETS.save(deps.storage, asset_reference.as_slice(), &updated_market)?;

            let res = Response::new()
                .add_attribute("action", "update_asset")
                .add_attribute("asset", asset_label);
            Ok(res)
        }
        None => Err(StdError::generic_err("Asset not initialized").into()),
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
    deposit_amount: Uint128,
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

    let mut events = vec![];
    let has_deposited_asset = get_bit(user.collateral_assets, market.index)?;
    if !has_deposited_asset {
        set_bit(&mut user.collateral_assets, market.index)?;
        USERS.save(deps.storage, &depositor_address, &user)?;
        events.push(build_collateral_position_changed_event(
            asset_label,
            true,
            depositor_address.to_string(),
        ));
    }

    apply_accumulated_interests(&env, &mut market);
    update_interest_rates(&deps, &env, asset_reference, &mut market, Uint128::zero())?;
    MARKETS.save(deps.storage, asset_reference, &market)?;

    if market.liquidity_index.is_zero() {
        return Err(StdError::generic_err("Cannot have 0 as liquidity index").into());
    }
    let mint_amount = get_scaled_amount(
        deposit_amount,
        get_updated_liquidity_index(&market, env.block.time.seconds()),
    );

    events.push(build_interests_updated_event(asset_label, &market));

    let res = Response::new()
        .add_attribute("action", "deposit")
        .add_attribute("market", asset_label)
        .add_attribute("user", depositor_address.as_str())
        .add_attribute("amount", deposit_amount)
        .add_events(events)
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: market.ma_token_address.into(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: depositor_address.into(),
                amount: mint_amount,
            })?,
            funds: vec![],
        }));

    Ok(res)
}

/// Add debt for the borrower and send the borrowed funds
pub fn execute_borrow(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset: Asset,
    borrow_amount: Uint128,
) -> Result<Response, ContractError> {
    let borrower_address = info.sender;
    let (asset_label, asset_reference, asset_type) = asset.get_attributes();

    // Cannot borrow zero amount
    if borrow_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Borrow amount must be greater than 0 {}",
            asset_label,
        ))
        .into());
    }

    // Load market and user state
    let global_state = GLOBAL_STATE.load(deps.storage)?;
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

    let is_borrowing_asset = get_bit(user.borrowed_assets, borrow_market.index)?;

    // Check if user can borrow specified amount
    let mut uncollateralized_debt = false;
    if uncollateralized_loan_limit.is_zero() {
        // Collateralized loan: check max ltv is not exceeded
        let config = CONFIG.load(deps.storage)?;
        let oracle_address = address_provider::helpers::query_address(
            &deps.querier,
            config.address_provider_address,
            MarsContract::Oracle,
        )?;

        let user_position = get_user_position(
            deps.as_ref(),
            env.block.time.seconds(),
            &borrower_address,
            oracle_address.clone(),
            &user,
            global_state.market_count,
        )?;

        let borrow_asset_price = if is_borrowing_asset {
            // if user was already borrowing, get price from user position
            user_position.get_asset_price(asset_reference.as_slice(), &asset_label)?
        } else {
            mars::oracle::helpers::query_price(
                deps.querier,
                oracle_address,
                &asset_label,
                asset_reference.clone(),
                asset_type,
            )?
        };

        let borrow_amount_in_uusd = borrow_amount * borrow_asset_price;

        if user_position.total_debt_in_uusd + borrow_amount_in_uusd > user_position.max_debt_in_uusd
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
                amount_scaled: Uint128::zero(),
                uncollateralized: uncollateralized_debt,
            });

        let asset_market = MARKETS.load(deps.storage, asset_reference.as_slice())?;
        let debt_amount = get_descaled_amount(
            borrower_debt.amount_scaled,
            get_updated_borrow_index(&asset_market, env.block.time.seconds()),
        );
        if borrow_amount + debt_amount > uncollateralized_loan_limit {
            return Err(StdError::generic_err(
                "borrow amount exceeds uncollateralized loan limit given existing debt",
            )
            .into());
        }
    }

    apply_accumulated_interests(&env, &mut borrow_market);

    let mut events = vec![];
    // Set borrowing asset for user
    if !is_borrowing_asset {
        set_bit(&mut user.borrowed_assets, borrow_market.index)?;
        USERS.save(deps.storage, &borrower_address, &user)?;
        events.push(build_debt_position_changed_event(
            asset_label.as_str(),
            true,
            borrower_address.to_string(),
        ));
    }

    // Set new debt
    let mut debt = DEBTS
        .may_load(
            deps.storage,
            (asset_reference.as_slice(), &borrower_address),
        )?
        .unwrap_or(Debt {
            amount_scaled: Uint128::zero(),
            uncollateralized: uncollateralized_debt,
        });
    let borrow_amount_scaled = get_scaled_amount(
        borrow_amount,
        get_updated_borrow_index(&borrow_market, env.block.time.seconds()),
    );
    debt.amount_scaled += borrow_amount_scaled;
    DEBTS.save(
        deps.storage,
        (asset_reference.as_slice(), &borrower_address),
        &debt,
    )?;

    borrow_market.debt_total_scaled += borrow_amount_scaled;

    update_interest_rates(
        &deps,
        &env,
        asset_reference.as_slice(),
        &mut borrow_market,
        borrow_amount,
    )?;
    MARKETS.save(deps.storage, asset_reference.as_slice(), &borrow_market)?;

    // Send borrow amount to borrower
    let send_msg = build_send_asset_msg(
        deps.as_ref(),
        env.contract.address,
        borrower_address.clone(),
        asset,
        borrow_amount,
    )?;

    events.push(build_interests_updated_event(
        asset_label.as_str(),
        &borrow_market,
    ));

    let res = Response::new()
        .add_attribute("action", "borrow")
        .add_attribute("market", asset_label.as_str())
        .add_attribute("user", borrower_address.as_str())
        .add_attribute("amount", borrow_amount)
        .add_events(events)
        .add_message(send_msg);
    Ok(res)
}

/// Handle the repay of native tokens. Refund extra funds if they exist
pub fn execute_repay(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    repayer_address: Addr,
    asset_reference: &[u8],
    asset_label: &str,
    repay_amount: Uint128,
    asset_type: AssetType,
) -> Result<Response, ContractError> {
    let mut market = MARKETS.load(deps.storage, asset_reference)?;

    // Get repay amount
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

    apply_accumulated_interests(&env, &mut market);

    let mut repay_amount_scaled = get_scaled_amount(
        repay_amount,
        get_updated_borrow_index(&market, env.block.time.seconds()),
    );

    let mut messages = vec![];
    let mut refund_amount = Uint128::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        refund_amount = get_descaled_amount(
            repay_amount_scaled - debt.amount_scaled,
            get_updated_borrow_index(&market, env.block.time.seconds()),
        );
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
        messages.push(refund_msg);
        repay_amount_scaled = debt.amount_scaled;
    }

    debt.amount_scaled -= repay_amount_scaled;
    DEBTS.save(deps.storage, (asset_reference, &repayer_address), &debt)?;

    if repay_amount_scaled > market.debt_total_scaled {
        return Err(StdError::generic_err("Amount to repay is greater than total debt").into());
    }
    market.debt_total_scaled -= repay_amount_scaled;
    update_interest_rates(&deps, &env, asset_reference, &mut market, Uint128::zero())?;
    MARKETS.save(deps.storage, asset_reference, &market)?;

    let mut events = vec![];
    if debt.amount_scaled.is_zero() {
        // Remove asset from borrowed assets
        let mut user = USERS.load(deps.storage, &repayer_address)?;
        unset_bit(&mut user.borrowed_assets, market.index)?;
        USERS.save(deps.storage, &repayer_address, &user)?;
        events.push(build_debt_position_changed_event(
            asset_label,
            false,
            repayer_address.to_string(),
        ));
    }

    events.push(build_interests_updated_event(asset_label, &market));

    let res = Response::new()
        .add_attribute("action", "repay")
        .add_attribute("market", asset_label)
        .add_attribute("user", repayer_address)
        .add_attribute("amount", repay_amount - refund_amount)
        .add_messages(messages)
        .add_events(events);
    Ok(res)
}

/// Execute loan liquidations on under-collateralized loans
pub fn execute_liquidate(
    mut deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    liquidator_address: Addr,
    collateral_asset: Asset,
    debt_asset: Asset,
    user_address: Addr,
    sent_debt_asset_amount: Uint128,
    receive_ma_token: bool,
) -> Result<Response, ContractError> {
    let block_time = env.block.time.seconds();
    let (debt_asset_label, debt_asset_reference, _) = debt_asset.get_attributes();

    // 1. Validate liquidation
    // If user (contract) has a positive uncollateralized limit then the user
    // cannot be liquidated
    if let Some(limit) = UNCOLLATERALIZED_LOAN_LIMITS.may_load(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
    )? {
        if !limit.is_zero() {
            return Err(StdError::generic_err(
                "user has a positive uncollateralized loan limit and thus cannot be liquidated",
            )
            .into());
        }
    };

    // liquidator must send positive amount of funds in the debt asset
    if sent_debt_asset_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Must send more than 0 {} in order to liquidate",
            debt_asset_label,
        ))
        .into());
    }

    let (collateral_asset_label, collateral_asset_reference, _) = collateral_asset.get_attributes();

    let mut collateral_market =
        MARKETS.load(deps.storage, collateral_asset_reference.as_slice())?;

    // check if user has available collateral in specified collateral asset to be liquidated
    let user_collateral_balance_scaled = cw20_get_balance(
        &deps.querier,
        collateral_market.ma_token_address.clone(),
        user_address.clone(),
    )?;
    let user_collateral_balance = get_descaled_amount(
        user_collateral_balance_scaled,
        get_updated_liquidity_index(&collateral_market, block_time),
    );
    if user_collateral_balance.is_zero() {
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
    let global_state = GLOBAL_STATE.load(deps.storage)?;
    let user = USERS.load(deps.storage, &user_address)?;
    let oracle_address = address_provider::helpers::query_address(
        &deps.querier,
        config.address_provider_address,
        MarsContract::Oracle,
    )?;
    let user_position = get_user_position(
        deps.as_ref(),
        block_time,
        &user_address,
        oracle_address,
        &user,
        global_state.market_count,
    )?;

    let health_factor = match user_position.health_status {
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
    if health_factor >= Decimal::one() {
        return Err(StdError::generic_err(
            "User's health factor is not less than 1 and thus cannot be liquidated",
        )
        .into());
    }

    let mut debt_market = MARKETS.load(deps.storage, debt_asset_reference.as_slice())?;

    // 3. Compute debt to repay and collateral to liquidate
    let collateral_price = user_position.get_asset_price(
        collateral_asset_reference.as_slice(),
        &collateral_asset_label,
    )?;
    let debt_price =
        user_position.get_asset_price(debt_asset_reference.as_slice(), &debt_asset_label)?;

    apply_accumulated_interests(&env, &mut debt_market);

    let user_debt_asset_total_debt = get_descaled_amount(
        user_debt.amount_scaled,
        get_updated_borrow_index(&debt_market, block_time),
    );

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
    let mut events = vec![];

    // 4. Update collateral positions and market depending on whether the liquidator elects to
    // receive ma_tokens or the underlying asset
    if receive_ma_token {
        process_ma_token_transfer_to_liquidator(
            deps.branch(),
            block_time,
            &user_address,
            &liquidator_address,
            collateral_asset_label.as_str(),
            &collateral_market,
            collateral_amount_to_liquidate,
            &mut messages,
            &mut events,
        )?;
    } else {
        process_underlying_asset_transfer_to_liquidator(
            deps.branch(),
            &env,
            &user_address,
            &liquidator_address,
            collateral_asset,
            collateral_asset_reference.as_slice(),
            &mut collateral_market,
            collateral_amount_to_liquidate,
            &mut messages,
        )?;
    }

    // if max collateral to liquidate equals the user's balance then unset collateral bit
    if collateral_amount_to_liquidate == user_collateral_balance {
        let mut user = USERS.load(deps.storage, &user_address)?;
        unset_bit(&mut user.collateral_assets, collateral_market.index)?;
        USERS.save(deps.storage, &user_address, &user)?;
        events.push(build_collateral_position_changed_event(
            collateral_asset_label.as_str(),
            false,
            user_address.to_string(),
        ));
    }

    // 5. Update debt market and positions

    let debt_amount_to_repay_scaled = get_scaled_amount(
        debt_amount_to_repay,
        get_updated_borrow_index(&debt_market, block_time),
    );

    // update user and market debt
    let mut debt = DEBTS.load(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
    )?;
    // NOTE: Should be > 0 as amount to repay is capped by the close factor
    debt.amount_scaled -= debt_amount_to_repay_scaled;
    DEBTS.save(
        deps.storage,
        (debt_asset_reference.as_slice(), &user_address),
        &debt,
    )?;
    debt_market.debt_total_scaled -= debt_amount_to_repay_scaled;

    update_interest_rates(
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
    if refund_amount > Uint128::zero() {
        let refund_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address,
            liquidator_address.clone(),
            debt_asset,
            refund_amount,
        )?;
        messages.push(refund_msg);
    }

    events.push(build_interests_updated_event(
        debt_asset_label.as_str(),
        &debt_market,
    ));
    if !receive_ma_token {
        events.push(build_interests_updated_event(
            collateral_asset_label.as_str(),
            &collateral_market,
        ));
    }

    let res = Response::new()
        .add_attribute("action", "liquidate")
        .add_attribute("collateral_market", collateral_asset_label.as_str())
        .add_attribute("debt_market", debt_asset_label.as_str())
        .add_attribute("user", user_address.as_str())
        .add_attribute("liquidator", liquidator_address.as_str())
        .add_attribute(
            "collateral_amount_liquidated",
            collateral_amount_to_liquidate.to_string(),
        )
        .add_attribute("debt_amount_repaid", debt_amount_to_repay.to_string())
        .add_attribute("refund_amount", refund_amount.to_string())
        .add_events(events)
        .add_messages(messages);
    Ok(res)
}

/// Transfer ma tokens from user to liquidator
fn process_ma_token_transfer_to_liquidator(
    deps: DepsMut,
    block_time: u64,
    user_addr: &Addr,
    liquidator_addr: &Addr,
    collateral_asset_label: &str,
    collateral_market: &Market,
    collateral_amount_to_liquidate: Uint128,
    messages: &mut Vec<CosmosMsg>,
    events: &mut Vec<Event>,
) -> StdResult<()> {
    let mut liquidator = USERS
        .may_load(deps.storage, &liquidator_addr)?
        .unwrap_or_default();

    // Set liquidator's deposited bit to true if not already true
    // NOTE: previous checks should ensure this amount is not zero
    let liquidator_is_using_as_collateral =
        get_bit(liquidator.collateral_assets, collateral_market.index)?;
    if !liquidator_is_using_as_collateral {
        set_bit(&mut liquidator.collateral_assets, collateral_market.index)?;
        USERS.save(deps.storage, &liquidator_addr, &liquidator)?;
        events.push(build_collateral_position_changed_event(
            collateral_asset_label,
            true,
            liquidator_addr.to_string(),
        ));
    }

    let collateral_amount_to_liquidate_scaled = get_scaled_amount(
        collateral_amount_to_liquidate,
        get_updated_liquidity_index(&collateral_market, block_time),
    );

    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: collateral_market.ma_token_address.to_string(),
        msg: to_binary(&mars::ma_token::msg::ExecuteMsg::TransferOnLiquidation {
            sender: user_addr.to_string(),
            recipient: liquidator_addr.to_string(),
            amount: collateral_amount_to_liquidate_scaled,
        })?,
        funds: vec![],
    }));

    Ok(())
}

/// Burn ma_tokens from user and send underlying asset to liquidator
fn process_underlying_asset_transfer_to_liquidator(
    deps: DepsMut,
    env: &Env,
    user_addr: &Addr,
    liquidator_addr: &Addr,
    collateral_asset: Asset,
    collateral_asset_reference: &[u8],
    mut collateral_market: &mut Market,
    collateral_amount_to_liquidate: Uint128,
    messages: &mut Vec<CosmosMsg>,
) -> StdResult<()> {
    let block_time = env.block.time.seconds();

    // Ensure contract has enough collateral to send back underlying asset
    let contract_collateral_balance = match collateral_asset.clone() {
        Asset::Native { denom } => {
            deps.querier
                .query_balance(env.contract.address.clone(), denom.as_str())?
                .amount
        }
        Asset::Cw20 {
            contract_addr: token_addr,
        } => {
            let token_addr = deps.api.addr_validate(&token_addr)?;
            cw20_get_balance(&deps.querier, token_addr, env.contract.address.clone())?
        }
    };

    if contract_collateral_balance < collateral_amount_to_liquidate {
        return Err(StdError::generic_err(
            "contract does not have enough collateral liquidity to send back underlying asset",
        )
        .into());
    }

    // Apply update collateral interest as liquidity is reduced
    apply_accumulated_interests(&env, &mut collateral_market);
    update_interest_rates(
        &deps,
        &env,
        collateral_asset_reference,
        &mut collateral_market,
        collateral_amount_to_liquidate,
    )?;

    let collateral_amount_to_liquidate_scaled = get_scaled_amount(
        collateral_amount_to_liquidate,
        get_updated_liquidity_index(&collateral_market, block_time),
    );

    let burn_ma_tokens_msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: collateral_market.ma_token_address.to_string(),
        msg: to_binary(&mars::ma_token::msg::ExecuteMsg::Burn {
            user: user_addr.to_string(),

            amount: collateral_amount_to_liquidate_scaled,
        })?,
        funds: vec![],
    });

    let send_underlying_asset_msg = build_send_asset_msg(
        deps.as_ref(),
        env.contract.address.clone(),
        liquidator_addr.clone(),
        collateral_asset,
        collateral_amount_to_liquidate,
    )?;
    messages.push(burn_ma_tokens_msg);
    messages.push(send_underlying_asset_msg);

    Ok(())
}

/// Computes debt to repay (in debt asset),
/// collateral to liquidate (in collateral asset) and
/// amount to refund the liquidator (in debt asset)
fn liquidation_compute_amounts(
    collateral_price: Decimal,
    debt_price: Decimal,
    close_factor: Decimal,
    user_collateral_balance: Uint128,
    liquidation_bonus: Decimal,
    user_debt_asset_total_debt: Uint128,
    sent_debt_asset_amount: Uint128,
) -> (Uint128, Uint128, Uint128) {
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
        debt_amount_to_repay_in_uusd * (Decimal::one() + liquidation_bonus);
    let mut collateral_amount_to_liquidate =
        collateral_amount_to_liquidate_in_uusd * reverse_decimal(collateral_price);

    // If collateral amount to liquidate is higher than user_collateral_balance,
    // liquidate the full balance and adjust the debt amount to repay accordingly
    if collateral_amount_to_liquidate > user_collateral_balance {
        collateral_amount_to_liquidate = user_collateral_balance;
        debt_amount_to_repay = (collateral_price * collateral_amount_to_liquidate)
            * reverse_decimal(debt_price)
            * reverse_decimal(Decimal::one() + liquidation_bonus);
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
    let market_reference = MARKET_REFERENCES_BY_MA_TOKEN.load(deps.storage, &info.sender)?;
    let market = MARKETS.load(deps.storage, market_reference.as_slice())?;

    // Check user health factor is above 1
    let global_state = GLOBAL_STATE.load(deps.storage)?;
    let mut from_user = USERS.load(deps.storage, &from_address)?;
    let config = CONFIG.load(deps.storage)?;
    let oracle_address = address_provider::helpers::query_address(
        &deps.querier,
        config.address_provider_address,
        MarsContract::Oracle,
    )?;
    let user_position = get_user_position(
        deps.as_ref(),
        env.block.time.seconds(),
        &from_address,
        oracle_address,
        &from_user,
        global_state.market_count,
    )?;
    if let UserHealthStatus::Borrowing(health_factor) = user_position.health_status {
        if health_factor < Decimal::one() {
            return Err(StdError::generic_err("Cannot make token transfer if it results in a health factor lower than 1 for the sender").into());
        }
    }

    let asset_label = String::from_utf8(market_reference).expect("Found invalid UTF-8");
    let mut events = vec![];

    // Update users's positions
    if from_address != to_address {
        if from_previous_balance.checked_sub(amount)?.is_zero() {
            unset_bit(&mut from_user.collateral_assets, market.index)?;
            USERS.save(deps.storage, &from_address, &from_user)?;
            events.push(build_collateral_position_changed_event(
                asset_label.as_str(),
                false,
                from_address.to_string(),
            ))
        }

        if to_previous_balance.is_zero() && !amount.is_zero() {
            let mut to_user = USERS
                .may_load(deps.storage, &to_address)?
                .unwrap_or_default();
            set_bit(&mut to_user.collateral_assets, market.index)?;
            USERS.save(deps.storage, &to_address, &to_user)?;
            events.push(build_collateral_position_changed_event(
                asset_label.as_str(),
                true,
                to_address.to_string(),
            ))
        }
    }

    let res = Response::new()
        .add_attribute("action", "finalize_liquidity_token_transfer")
        .add_events(events);
    Ok(res)
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

    let (asset_label, asset_reference, _) = asset.get_attributes();

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
                amount_scaled: Uint128::zero(),
                uncollateralized: false,
            });
            // if limit == 0 then uncollateralized = false, otherwise uncollateralized = true
            debt.uncollateralized = !new_limit.is_zero();
            Ok(debt)
        },
    )?;

    let res = Response::new()
        .add_attribute("action", "update_uncollateralized_loan_limit")
        .add_attribute("user", user_address.as_str())
        .add_attribute("asset", asset_label)
        .add_attribute("new_allowance", new_limit.to_string());
    Ok(res)
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

    let mut events = vec![];

    let (collateral_asset_label, collateral_asset_reference, _) = asset.get_attributes();
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
            events.push(build_collateral_position_changed_event(
                collateral_asset_label.as_str(),
                true,
                user_address.to_string(),
            ));
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
        events.push(build_collateral_position_changed_event(
            collateral_asset_label.as_str(),
            false,
            user_address.to_string(),
        ));
    }

    let res = Response::new()
        .add_attribute("action", "update_user_collateral_asset_status")
        .add_attribute("user", user_address.as_str())
        .add_attribute("asset", collateral_asset_label)
        .add_attribute("has_collateral", has_collateral_asset.to_string())
        .add_attribute("enable", enable.to_string())
        .add_events(events);
    Ok(res)
}

/// Send accumulated asset income to protocol contracts
pub fn execute_distribute_protocol_income(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    asset: Asset,
    amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    // Get config
    let config = CONFIG.load(deps.storage)?;

    let (asset_label, asset_reference, _) = asset.get_attributes();
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

    market.protocol_income_to_distribute -= amount_to_distribute;
    MARKETS.save(deps.storage, asset_reference.as_slice(), &market)?;

    let mars_contracts = vec![
        MarsContract::InsuranceFund,
        MarsContract::Staking,
        MarsContract::Treasury,
    ];
    let mut addresses_query = address_provider::helpers::query_addresses(
        &deps.querier,
        config.address_provider_address,
        mars_contracts,
    )?;

    let treasury_address = addresses_query.pop().unwrap();
    let staking_address = addresses_query.pop().unwrap();
    let insurance_fund_address = addresses_query.pop().unwrap();

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
    let staking_amount = amount_to_distribute - amount_to_distribute_before_staking_rewards;

    let mut messages = vec![];
    // only build and add send message if fee is non-zero
    if !insurance_fund_amount.is_zero() {
        let insurance_fund_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address.clone(),
            insurance_fund_address,
            asset.clone(),
            insurance_fund_amount,
        )?;
        messages.push(insurance_fund_msg);
    }

    if !treasury_amount.is_zero() {
        let scaled_mint_amount = get_scaled_amount(
            treasury_amount,
            get_updated_liquidity_index(&market, env.block.time.seconds()),
        );
        let treasury_fund_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: market.ma_token_address.into(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: treasury_address.into(),
                amount: scaled_mint_amount,
            })?,
            funds: vec![],
        });
        messages.push(treasury_fund_msg);
    }

    if !staking_amount.is_zero() {
        let staking_msg = build_send_asset_msg(
            deps.as_ref(),
            env.contract.address,
            staking_address,
            asset,
            staking_amount,
        )?;
        messages.push(staking_msg);
    }

    let res = Response::new()
        .add_attribute("action", "distribute_protocol_income")
        .add_attribute("asset", asset_label)
        .add_attribute("amount", amount_to_distribute)
        .add_messages(messages);
    Ok(res)
}

// QUERIES

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Market { asset } => to_binary(&query_market(deps, asset)?),
        QueryMsg::MarketsList {} => to_binary(&query_markets_list(deps)?),
        QueryMsg::UserDebt { user_address } => {
            let address = deps.api.addr_validate(&user_address)?;
            to_binary(&query_debt(deps, address)?)
        }
        QueryMsg::UserCollateral { user_address } => {
            let address = deps.api.addr_validate(&user_address)?;
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
        QueryMsg::ScaledLiquidityAmount { asset, amount } => {
            to_binary(&query_scaled_liquidity_amount(deps, env, asset, amount)?)
        }
        QueryMsg::ScaledDebtAmount { asset, amount } => {
            to_binary(&query_scaled_debt_amount(deps, env, asset, amount)?)
        }
        QueryMsg::DescaledLiquidityAmount {
            ma_token_address,
            amount,
        } => to_binary(&query_descaled_liquidity_amount(
            deps,
            env,
            ma_token_address,
            amount,
        )?),
        QueryMsg::UserPosition { user_address } => {
            let address = deps.api.addr_validate(&user_address)?;
            to_binary(&query_user_position(deps, env, address)?)
        }
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    let money_market = GLOBAL_STATE.load(deps.storage)?;

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
    let (label, reference, _) = asset.get_attributes();
    let market = match MARKETS.load(deps.storage, reference.as_slice()) {
        Ok(market) => market,
        Err(_) => {
            return Err(StdError::generic_err(format!(
                "failed to load market for: {}",
                label
            )))
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
                    amount_scaled: debt.amount_scaled,
                })
            } else {
                Ok(DebtInfo {
                    denom,
                    amount_scaled: Uint128::zero(),
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
    let (asset_label, asset_reference, _) = asset.get_attributes();
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

fn query_scaled_liquidity_amount(
    deps: Deps,
    env: Env,
    asset: Asset,
    amount: Uint128,
) -> StdResult<AmountResponse> {
    let asset_reference = asset.get_reference();
    let market = MARKETS.load(deps.storage, asset_reference.as_slice())?;
    let scaled_amount = get_scaled_amount(
        amount,
        get_updated_liquidity_index(&market, env.block.time.seconds()),
    );
    Ok(AmountResponse {
        amount: scaled_amount,
    })
}

fn query_scaled_debt_amount(
    deps: Deps,
    env: Env,
    asset: Asset,
    amount: Uint128,
) -> StdResult<AmountResponse> {
    let asset_reference = asset.get_reference();
    let market = MARKETS.load(deps.storage, asset_reference.as_slice())?;
    let scaled_amount = get_scaled_amount(
        amount,
        get_updated_borrow_index(&market, env.block.time.seconds()),
    );
    Ok(AmountResponse {
        amount: scaled_amount,
    })
}

fn query_descaled_liquidity_amount(
    deps: Deps,
    env: Env,
    ma_token_address: String,
    amount: Uint128,
) -> StdResult<AmountResponse> {
    let ma_token_address = deps.api.addr_validate(&ma_token_address)?;
    let market_reference = MARKET_REFERENCES_BY_MA_TOKEN.load(deps.storage, &ma_token_address)?;
    let market = MARKETS.load(deps.storage, market_reference.as_slice())?;
    let descaled_amount = get_descaled_amount(
        amount,
        get_updated_liquidity_index(&market, env.block.time.seconds()),
    );
    Ok(AmountResponse {
        amount: descaled_amount,
    })
}

fn query_user_position(deps: Deps, env: Env, address: Addr) -> StdResult<UserPositionResponse> {
    let config = CONFIG.load(deps.storage)?;
    let global_state = GLOBAL_STATE.load(deps.storage)?;
    let user = USERS.load(deps.storage, &address)?;
    let oracle_address = address_provider::helpers::query_address(
        &deps.querier,
        config.address_provider_address,
        MarsContract::Oracle,
    )?;
    let user_position = get_user_position(
        deps,
        env.block.time.seconds(),
        &address,
        oracle_address,
        &user,
        global_state.market_count,
    )?;

    Ok(UserPositionResponse {
        total_collateral_in_uusd: user_position.total_collateral_in_uusd,
        total_debt_in_uusd: user_position.total_debt_in_uusd,
        total_collateralized_debt_in_uusd: user_position.total_collateralized_debt_in_uusd,
        max_debt_in_uusd: user_position.max_debt_in_uusd,
        weighted_maintenance_margin_in_uusd: user_position.weighted_maintenance_margin_in_uusd,
        health_status: user_position.health_status,
    })
}

// EVENTS

fn build_interests_updated_event(label: &str, market: &Market) -> Event {
    Event::new("interests_updated")
        .add_attribute("market", label)
        .add_attribute("borrow_index", market.borrow_index.to_string())
        .add_attribute("liquidity_index", market.liquidity_index.to_string())
        .add_attribute("borrow_rate", market.borrow_rate.to_string())
        .add_attribute("liquidity_rate", market.liquidity_rate.to_string())
}

fn build_collateral_position_changed_event(label: &str, enabled: bool, user_addr: String) -> Event {
    Event::new("collateral_position_changed")
        .add_attribute("market", label)
        .add_attribute("using_as_collateral", enabled.to_string())
        .add_attribute("user", user_addr)
}

fn build_debt_position_changed_event(label: &str, enabled: bool, user_addr: String) -> Event {
    Event::new("debt_position_changed")
        .add_attribute("market", label)
        .add_attribute("borrowing", enabled.to_string())
        .add_attribute("user", user_addr)
}

// HELPERS

// native coins
fn get_denom_amount_from_coins(coins: &[Coin], denom: &str) -> Uint128 {
    coins
        .iter()
        .find(|c| c.denom == denom)
        .map(|c| c.amount)
        .unwrap_or_else(Uint128::zero)
}

fn get_market_denom(
    deps: Deps,
    market_reference: Vec<u8>,
    asset_type: AssetType,
) -> StdResult<String> {
    match asset_type {
        AssetType::Native => match String::from_utf8(market_reference) {
            Ok(denom) => Ok(denom),
            Err(_) => Err(StdError::generic_err("failed to encode key into string")),
        },
        AssetType::Cw20 => {
            let cw20_contract_address = match String::from_utf8(market_reference) {
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
pub fn get_bit(bitmap: Uint128, index: u32) -> StdResult<bool> {
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
    amount: Uint128,
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

fn build_send_cw20_token_msg(
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

pub fn market_get_from_index(deps: &Deps, index: u32) -> StdResult<(Vec<u8>, Market)> {
    let asset_reference_vec =
        match MARKET_REFERENCES_BY_INDEX.load(deps.storage, U32Key::new(index)) {
            Ok(asset_reference_vec) => asset_reference_vec,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "no market reference exists with index: {}",
                    index
                )))
            }
        };

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
    use crate::interest_rate::{calculate_applied_linear_interest_rate, SCALING_FACTOR};
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{attr, coin, from_binary, Decimal, OwnedDeps, SubMsg};
    use mars::interest_rate_models::{
        DynamicInterestRate, InterestRateModel, InterestRateStrategy, LinearInterestRate,
    };
    use mars::red_bank::msg::ExecuteMsg::UpdateConfig;
    use mars::testing::{
        assert_generic_error_message, mock_dependencies, mock_env, mock_env_at_block_time,
        mock_info, MarsMockQuerier, MockEnvParams,
    };

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
        let mut insurance_fund_fee_share = Decimal::from_ratio(11u128, 10u128);
        let mut treasury_fee_share = Decimal::from_ratio(12u128, 10u128);
        let mut close_factor = Decimal::from_ratio(13u128, 10u128);
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
        insurance_fund_fee_share = Decimal::from_ratio(7u128, 10u128);
        treasury_fee_share = Decimal::from_ratio(4u128, 10u128);
        close_factor = Decimal::from_ratio(1u128, 2u128);
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
            "Invalid fee share amounts. Sum of insurance and treasury fee shares exceeds one",
        );

        // *
        // init config with valid params
        // *
        insurance_fund_fee_share = Decimal::from_ratio(5u128, 10u128);
        treasury_fee_share = Decimal::from_ratio(3u128, 10u128);
        close_factor = Decimal::from_ratio(1u128, 2u128);
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
        let mut insurance_fund_fee_share = Decimal::from_ratio(1u128, 10u128);
        let mut treasury_fee_share = Decimal::from_ratio(3u128, 10u128);
        let mut close_factor = Decimal::from_ratio(1u128, 4u128);
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
        insurance_fund_fee_share = Decimal::from_ratio(11u128, 10u128);
        treasury_fee_share = Decimal::from_ratio(12u128, 10u128);
        close_factor = Decimal::from_ratio(13u128, 10u128);
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
        insurance_fund_fee_share = Decimal::from_ratio(10u128, 10u128);
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
                "Invalid fee share amounts. Sum of insurance and treasury fee shares exceeds one"
            )
            .into()
        );

        // *
        // update config with all new params
        // *
        insurance_fund_fee_share = Decimal::from_ratio(5u128, 100u128);
        treasury_fee_share = Decimal::from_ratio(3u128, 100u128);
        close_factor = Decimal::from_ratio(1u128, 20u128);
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
            insurance_fund_fee_share: Some(Decimal::from_ratio(5u128, 10u128)),
            treasury_fee_share: Some(Decimal::from_ratio(3u128, 10u128)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal::from_ratio(1u128, 2u128)),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(5u128, 100u128),
            max_borrow_rate: Decimal::from_ratio(50u128, 100u128),
            kp_1: Decimal::from_ratio(3u128, 1u128),
            optimal_utilization_rate: Decimal::from_ratio(80u128, 100u128),
            kp_augmentation_threshold: Decimal::from_ratio(2000u128, 1u128),
            kp_2: Decimal::from_ratio(2u128, 1u128),
        };
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal::from_ratio(20u128, 100u128)),
            max_loan_to_value: Some(Decimal::from_ratio(8u128, 10u128)),
            reserve_factor: Some(Decimal::from_ratio(1u128, 100u128)),
            maintenance_margin: Some(Decimal::one()),
            liquidation_bonus: Some(Decimal::zero()),
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(dynamic_ir.clone())),
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
            ..asset_params.clone()
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
            max_loan_to_value: Some(Decimal::from_ratio(110u128, 10u128)),
            reserve_factor: Some(Decimal::from_ratio(120u128, 100u128)),
            ..asset_params.clone()
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
            max_loan_to_value: Some(Decimal::from_ratio(5u128, 10u128)),
            maintenance_margin: Some(Decimal::from_ratio(5u128, 10u128)),
            ..asset_params.clone()
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
        let invalid_dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(5u128, 10u128),
            max_borrow_rate: Decimal::from_ratio(4u128, 10u128),
            ..dynamic_ir.clone()
        };
        let invalid_asset_params = InitOrUpdateAssetParams {
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(invalid_dynamic_ir)),
            ..asset_params.clone()
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
        let invalid_dynamic_ir = DynamicInterestRate {
            optimal_utilization_rate: Decimal::from_ratio(11u128, 10u128),
            ..dynamic_ir.clone()
        };
        let invalid_asset_params = InitOrUpdateAssetParams {
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(invalid_dynamic_ir)),
            ..asset_params.clone()
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
        let market_reference = MARKET_REFERENCES_BY_INDEX
            .load(&deps.storage, U32Key::new(0))
            .unwrap();
        assert_eq!(b"someasset", market_reference.as_slice());

        // Should have market count of 1
        let money_market = GLOBAL_STATE.load(&deps.storage).unwrap();
        assert_eq!(money_market.market_count, 1);

        // should instantiate a liquidity token
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Instantiate {
                admin: Some("protocol_admin".to_string()),
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
                    init_hook: Some(ma_token::msg::InitHook {
                        contract_addr: MOCK_CONTRACT_ADDR.to_string(),
                        msg: to_binary(&ExecuteMsg::InitAssetTokenCallback {
                            reference: market_reference,
                        })
                        .unwrap()
                    }),
                    marketing: Some(InstantiateMarketingInfo {
                        project: Some("Mars Protocol".to_string()),
                        description: Some(
                            "Interest earning token representing deposits for someasset"
                                .to_string()
                        ),

                        marketing: Some("protocol_admin".to_string()),
                        logo: None,
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
        assert_eq!(Decimal::one(), market.liquidity_index);

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
        let market_reference = MARKET_REFERENCES_BY_INDEX
            .load(&deps.storage, U32Key::new(1))
            .unwrap();
        assert_eq!(cw20_addr.as_bytes(), market_reference.as_slice());

        // should have an asset_type of cw20
        assert_eq!(AssetType::Cw20, market.asset_type);

        // Should have market count of 2
        let money_market = GLOBAL_STATE.load(&deps.storage).unwrap();
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
        assert_eq!(Decimal::one(), market.liquidity_index);

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
            insurance_fund_fee_share: Some(Decimal::from_ratio(5u128, 10u128)),
            treasury_fee_share: Some(Decimal::from_ratio(3u128, 10u128)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal::from_ratio(1u128, 2u128)),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(5u128, 100u128),
            max_borrow_rate: Decimal::from_ratio(50u128, 100u128),
            kp_1: Decimal::from_ratio(3u128, 1u128),
            optimal_utilization_rate: Decimal::from_ratio(80u128, 100u128),
            kp_augmentation_threshold: Decimal::from_ratio(2000u128, 1u128),
            kp_2: Decimal::from_ratio(2u128, 1u128),
        };
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal::from_ratio(20u128, 100u128)),
            max_loan_to_value: Some(Decimal::from_ratio(50u128, 100u128)),
            reserve_factor: Some(Decimal::from_ratio(1u128, 100u128)),
            maintenance_margin: Some(Decimal::from_ratio(80u128, 100u128)),
            liquidation_bonus: Some(Decimal::from_ratio(10u128, 100u128)),
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(dynamic_ir.clone())),
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
            maintenance_margin: Some(Decimal::from_ratio(110u128, 10u128)),
            ..asset_params.clone()
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
            max_loan_to_value: Some(Decimal::from_ratio(6u128, 10u128)),
            maintenance_margin: Some(Decimal::from_ratio(5u128, 10u128)),
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
        let invalid_dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(4u128, 10u128),
            max_borrow_rate: Decimal::from_ratio(4u128, 10u128),
            ..dynamic_ir
        };
        let invalid_asset_params = InitOrUpdateAssetParams {
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(invalid_dynamic_ir.clone())),
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
        let invalid_dynamic_ir = DynamicInterestRate {
            optimal_utilization_rate: Decimal::from_ratio(11u128, 10u128),
            ..dynamic_ir
        };
        let invalid_asset_params = InitOrUpdateAssetParams {
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(invalid_dynamic_ir.clone())),
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
        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(5u128, 100u128),
            max_borrow_rate: Decimal::from_ratio(50u128, 100u128),
            kp_1: Decimal::from_ratio(3u128, 1u128),
            optimal_utilization_rate: Decimal::from_ratio(80u128, 100u128),
            kp_augmentation_threshold: Decimal::from_ratio(2000u128, 1u128),
            kp_2: Decimal::from_ratio(2u128, 1u128),
        };
        let asset_params = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal::from_ratio(20u128, 100u128)),
            max_loan_to_value: Some(Decimal::from_ratio(60u128, 100u128)),
            reserve_factor: Some(Decimal::from_ratio(10u128, 100u128)),
            maintenance_margin: Some(Decimal::from_ratio(90u128, 100u128)),
            liquidation_bonus: Some(Decimal::from_ratio(12u128, 100u128)),
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(dynamic_ir.clone())),
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

        let new_market_reference = MARKET_REFERENCES_BY_INDEX
            .load(&deps.storage, U32Key::new(0))
            .unwrap();
        assert_eq!(b"someasset", new_market_reference.as_slice());

        let new_money_market = GLOBAL_STATE.load(&deps.storage).unwrap();
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
            max_loan_to_value: None,
            reserve_factor: None,
            maintenance_margin: None,
            liquidation_bonus: None,
            interest_rate_strategy: None,
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
        if let InterestRateStrategy::Dynamic(market_dynamic_ir) = new_market.interest_rate_strategy
        {
            assert_eq!(
                dynamic_ir.min_borrow_rate,
                market_dynamic_ir.min_borrow_rate
            );
            assert_eq!(
                dynamic_ir.max_borrow_rate,
                market_dynamic_ir.max_borrow_rate
            );
            assert_eq!(dynamic_ir.kp_1, market_dynamic_ir.kp_1);
            assert_eq!(
                dynamic_ir.kp_augmentation_threshold,
                market_dynamic_ir.kp_augmentation_threshold
            );
            assert_eq!(dynamic_ir.kp_2, market_dynamic_ir.kp_2);
        } else {
            panic!("INCORRECT STRATEGY")
        }
    }

    #[test]
    fn test_update_asset_with_new_interest_rate_strategy() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let config = CreateOrUpdateConfig {
            owner: Some("owner".to_string()),
            address_provider_address: Some("address_provider".to_string()),
            insurance_fund_fee_share: Some(Decimal::from_ratio(5u128, 10u128)),
            treasury_fee_share: Some(Decimal::from_ratio(3u128, 10u128)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal::from_ratio(1u128, 2u128)),
        };
        let msg = InstantiateMsg { config };
        let info = mock_info("owner");
        instantiate(deps.as_mut(), env.clone(), info, msg).unwrap();

        let dynamic_ir = DynamicInterestRate {
            min_borrow_rate: Decimal::from_ratio(10u128, 100u128),
            max_borrow_rate: Decimal::from_ratio(60u128, 100u128),
            kp_1: Decimal::from_ratio(4u128, 1u128),
            optimal_utilization_rate: Decimal::from_ratio(90u128, 100u128),
            kp_augmentation_threshold: Decimal::from_ratio(2000u128, 1u128),
            kp_2: Decimal::from_ratio(3u128, 1u128),
        };
        let asset_params_with_dynamic_ir = InitOrUpdateAssetParams {
            initial_borrow_rate: Some(Decimal::from_ratio(15u128, 100u128)),
            max_loan_to_value: Some(Decimal::from_ratio(50u128, 100u128)),
            reserve_factor: Some(Decimal::from_ratio(2u128, 100u128)),
            maintenance_margin: Some(Decimal::from_ratio(80u128, 100u128)),
            liquidation_bonus: Some(Decimal::from_ratio(10u128, 100u128)),
            interest_rate_strategy: Some(InterestRateStrategy::Dynamic(dynamic_ir.clone())),
        };

        let msg = ExecuteMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params_with_dynamic_ir.clone(),
        };
        let info = mock_info("owner");
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // Verify if IR strategy is saved correctly
        let new_market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(
            new_market.interest_rate_strategy,
            InterestRateStrategy::Dynamic(dynamic_ir)
        );

        let linear_ir = LinearInterestRate {
            optimal_utilization_rate: Decimal::from_ratio(80u128, 100u128),
            base: Decimal::from_ratio(0u128, 100u128),
            slope_1: Decimal::from_ratio(8u128, 100u128),
            slope_2: Decimal::from_ratio(48u128, 100u128),
        };
        let asset_params_with_linear_ir = InitOrUpdateAssetParams {
            interest_rate_strategy: Some(InterestRateStrategy::Linear(linear_ir.clone())),
            ..asset_params_with_dynamic_ir
        };
        let msg = ExecuteMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params_with_linear_ir.clone(),
        };
        let info = mock_info("owner");
        let _res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

        // Verify if IR strategy is updated
        let new_market = MARKETS.load(&deps.storage, b"someasset").unwrap();
        assert_eq!(
            new_market.interest_rate_strategy,
            InterestRateStrategy::Linear(linear_ir)
        );
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
        let reserve_factor = Decimal::from_ratio(1u128, 10u128);

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal::from_ratio(11u128, 10u128),
            max_loan_to_value: Decimal::one(),
            borrow_index: Decimal::from_ratio(1u128, 1u128),
            borrow_rate: Decimal::from_ratio(10u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor,
            debt_total_scaled: Uint128::new(10_000_000 * SCALING_FACTOR),
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

        let expected_mint_amount = get_scaled_amount(
            Uint128::from(deposit_amount),
            expected_params.liquidity_index,
        );

        // mints coin_amount/liquidity_index
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
                attr("market", "somecoin"),
                attr("user", "depositor"),
                attr("amount", deposit_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_collateral_position_changed_event("somecoin", true, "depositor".to_string()),
                th_build_interests_updated_event("somecoin", &expected_params)
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
            liquidity_index: Decimal::from_ratio(11u128, 10u128),
            max_loan_to_value: Decimal::one(),
            borrow_index: Decimal::from_ratio(1u128, 1u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(4u128, 100u128),
            debt_total_scaled: Uint128::new(10_000_000 * SCALING_FACTOR),
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

        let expected_mint_amount = get_scaled_amount(
            Uint128::from(deposit_amount),
            expected_params.liquidity_index,
        );

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
                attr("market", cw20_addr.clone()),
                attr("user", "depositor"),
                attr("amount", deposit_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_collateral_position_changed_event(
                    cw20_addr.as_str(),
                    true,
                    "depositor".to_string()
                ),
                th_build_interests_updated_event(cw20_addr.as_str(), &expected_params)
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

        let initial_liquidity_index = Decimal::from_ratio(15u128, 10u128);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal::from_ratio(2u128, 1u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(1u128, 10u128),

            debt_total_scaled: Uint128::new(10_000_000 * SCALING_FACTOR),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let withdraw_amount = Uint128::from(20000u128);
        let seconds_elapsed = 2000u64;

        deps.querier.set_cw20_balances(
            Addr::unchecked("matoken"),
            &[(
                Addr::unchecked("withdrawer"),
                Uint128::new(2_000_000 * SCALING_FACTOR),
            )],
        );

        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        MARKET_REFERENCES_BY_MA_TOKEN
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

        let withdraw_amount_scaled =
            get_scaled_amount(withdraw_amount, expected_params.liquidity_index);

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
                attr("burn_amount", withdraw_amount_scaled.to_string()),
                attr("withdraw_amount", withdraw_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![th_build_interests_updated_event(
                "somecoin",
                &expected_params
            )]
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
            &[(
                Addr::unchecked("withdrawer"),
                Uint128::new(2_000_000 * SCALING_FACTOR),
            )],
        );

        let initial_liquidity_index = Decimal::from_ratio(15u128, 10u128);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal::from_ratio(2u128, 1u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(2u128, 100u128),
            debt_total_scaled: Uint128::new(10_000_000 * SCALING_FACTOR),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let withdraw_amount = Uint128::from(20000u128);
        let seconds_elapsed = 2000u64;

        let market_initial =
            th_init_market(deps.as_mut(), cw20_contract_addr.as_bytes(), &mock_market);
        MARKET_REFERENCES_BY_MA_TOKEN
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

        let withdraw_amount_scaled =
            get_scaled_amount(withdraw_amount, expected_params.liquidity_index);

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
                attr("burn_amount", withdraw_amount_scaled.to_string()),
                attr("withdraw_amount", withdraw_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![th_build_interests_updated_event(
                "somecontract",
                &expected_params
            )]
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
            liquidity_index: Decimal::from_ratio(15u128, 10u128),
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
            amount: Some(Uint128::from(2000u128)),
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
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            max_loan_to_value: Decimal::from_ratio(40u128, 100u128),
            maintenance_margin: Decimal::from_ratio(60u128, 100u128),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_2_addr = Addr::unchecked("matoken2");
        let market_2 = Market {
            ma_token_address: ma_token_2_addr,
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            max_loan_to_value: Decimal::from_ratio(50u128, 100u128),
            maintenance_margin: Decimal::from_ratio(80u128, 100u128),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_3_addr = Addr::unchecked("matoken3");
        let market_3 = Market {
            ma_token_address: ma_token_3_addr.clone(),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            max_loan_to_value: Decimal::from_ratio(20u128, 100u128),
            maintenance_margin: Decimal::from_ratio(40u128, 100u128),
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
        let ma_token_1_balance_scaled = Uint128::new(100_000 * SCALING_FACTOR);
        deps.querier.set_cw20_balances(
            ma_token_1_addr,
            &[(withdrawer_addr.clone(), ma_token_1_balance_scaled.into())],
        );
        let ma_token_3_balance_scaled = Uint128::new(600_000 * SCALING_FACTOR);
        deps.querier.set_cw20_balances(
            ma_token_3_addr,
            &[(withdrawer_addr.clone(), ma_token_3_balance_scaled.into())],
        );

        // Set user to have positive debt amount in debt asset
        // Uncollateralized debt shouldn't count for health factor
        let token_2_debt_scaled = Uint128::new(200_000 * SCALING_FACTOR);
        let debt = Debt {
            amount_scaled: token_2_debt_scaled,
            uncollateralized: false,
        };
        let uncollateralized_debt = Debt {
            amount_scaled: Uint128::new(200_000 * SCALING_FACTOR),
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

        deps.querier
            .set_oracle_price(b"token1".to_vec(), token_1_exchange_rate);
        deps.querier
            .set_oracle_price(b"token2".to_vec(), token_2_exchange_rate);
        deps.querier
            .set_oracle_price(b"token3".to_vec(), token_3_exchange_rate);

        let env = mock_env(MockEnvParams::default());
        let info = mock_info("withdrawer");

        // Calculate how much to withdraw to have health factor equal to one
        let how_much_to_withdraw = {
            let token_1_weighted_lt_in_uusd = get_descaled_amount(
                ma_token_1_balance_scaled,
                get_updated_liquidity_index(&market_1_initial, env.block.time.seconds()),
            ) * market_1_initial.maintenance_margin
                * token_1_exchange_rate;
            let token_3_weighted_lt_in_uusd = get_descaled_amount(
                ma_token_3_balance_scaled,
                get_updated_liquidity_index(&market_3_initial, env.block.time.seconds()),
            ) * market_3_initial.maintenance_margin
                * token_3_exchange_rate;
            let weighted_maintenance_margin_in_uusd =
                token_1_weighted_lt_in_uusd + token_3_weighted_lt_in_uusd;

            let total_collateralized_debt_in_uusd = get_descaled_amount(
                token_2_debt_scaled,
                get_updated_borrow_index(&market_2_initial, env.block.time.seconds()),
            ) * token_2_exchange_rate;

            // How much to withdraw in uusd to have health factor equal to one
            let how_much_to_withdraw_in_uusd = (weighted_maintenance_margin_in_uusd
                - total_collateralized_debt_in_uusd)
                * reverse_decimal(market_3_initial.maintenance_margin);
            how_much_to_withdraw_in_uusd * reverse_decimal(token_3_exchange_rate)
        };

        // Withdraw token3 with failure
        // The withdraw amount needs to be a little bit greater to have health factor less than one
        {
            let withdraw_amount = how_much_to_withdraw + Uint128::from(10u128);
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
            let withdraw_amount = how_much_to_withdraw - Uint128::from(10u128);
            let msg = ExecuteMsg::Withdraw {
                asset: Asset::Native {
                    denom: "token3".to_string(),
                },
                amount: Some(withdraw_amount),
            };
            let res = execute(deps.as_mut(), env.clone(), info, msg).unwrap();

            let withdraw_amount_scaled = get_scaled_amount(
                withdraw_amount,
                get_updated_liquidity_index(&market_3_initial, env.block.time.seconds()),
            );

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
                                amount: withdraw_amount,
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

        let initial_liquidity_index = Decimal::from_ratio(15u128, 10u128);
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal::from_ratio(2u128, 1u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(1u128, 10u128),
            debt_total_scaled: Uint128::new(10_000_000 * SCALING_FACTOR),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let withdrawer_balance_scaled = Uint128::new(123_456 * SCALING_FACTOR);
        let seconds_elapsed = 2000u64;

        deps.querier.set_cw20_balances(
            Addr::unchecked("matoken"),
            &[(
                Addr::unchecked("withdrawer"),
                withdrawer_balance_scaled.into(),
            )],
        );

        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        MARKET_REFERENCES_BY_MA_TOKEN
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

        let withdrawer_balance = get_descaled_amount(
            withdrawer_balance_scaled,
            get_updated_liquidity_index(
                &market_initial,
                market_initial.interests_last_updated + seconds_elapsed,
            ),
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
                attr("burn_amount", withdrawer_balance_scaled.to_string()),
                attr("withdraw_amount", withdrawer_balance.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_collateral_position_changed_event(
                    "somecoin",
                    false,
                    "withdrawer".to_string()
                ),
                th_build_interests_updated_event("somecoin", &expected_params)
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

        deps.querier
            .set_oracle_price(b"borrowedcoinnative".to_vec(), Decimal::one());
        deps.querier
            .set_oracle_price(b"depositedcoin".to_vec(), Decimal::one());
        deps.querier
            .set_oracle_price(b"borrowedcoincw20".to_vec(), Decimal::one());

        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("borrowedcoinnative"), Uint128::new(100u128))],
        );

        let mock_market_1 = Market {
            ma_token_address: Addr::unchecked("matoken1"),
            borrow_index: Decimal::from_ratio(12u128, 10u128),
            liquidity_index: Decimal::from_ratio(8u128, 10u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(1u128, 100u128),
            debt_total_scaled: Uint128::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: Addr::unchecked("matoken2"),
            borrow_index: Decimal::one(),
            liquidity_index: Decimal::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: Addr::unchecked("matoken3"),
            borrow_index: Decimal::one(),
            liquidity_index: Decimal::from_ratio(11u128, 10u128),
            max_loan_to_value: Decimal::from_ratio(7u128, 10u128),
            borrow_rate: Decimal::from_ratio(30u128, 100u128),
            reserve_factor: Decimal::from_ratio(3u128, 100u128),
            liquidity_rate: Decimal::from_ratio(20u128, 100u128),
            debt_total_scaled: Uint128::zero(),
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
            &[(borrower_addr.clone(), Uint128::new(10000 * SCALING_FACTOR))],
        );

        // *
        // Borrow cw20 token
        // *
        let block_time = mock_market_1.interests_last_updated + 10000u64;
        let borrow_amount = 2400u128;

        let msg = ExecuteMsg::Borrow {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.to_string(),
            },
            amount: Uint128::from(borrow_amount),
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
                attr("amount", borrow_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_debt_position_changed_event("borrowedcoincw20", true, "borrower".to_string()),
                th_build_interests_updated_event("borrowedcoincw20", &expected_params_cw20)
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
        let expected_debt_scaled_1_after_borrow = get_scaled_amount(
            Uint128::from(borrow_amount),
            expected_params_cw20.borrow_index,
        );

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
            amount: Uint128::from(borrow_amount),
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
            + get_scaled_amount(
                Uint128::from(borrow_amount),
                expected_params_cw20.borrow_index,
            );
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
            amount: Uint128::from(borrow_amount),
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
                attr("amount", borrow_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_debt_position_changed_event(
                    "borrowedcoinnative",
                    true,
                    "borrower".to_string()
                ),
                th_build_interests_updated_event("borrowedcoinnative", &expected_params_native)
            ]
        );

        let debt2 = DEBTS
            .load(&deps.storage, (b"borrowedcoinnative", &borrower_addr))
            .unwrap();
        let market_2_after_borrow_2 = MARKETS.load(&deps.storage, b"borrowedcoinnative").unwrap();

        let expected_debt_scaled_2_after_borrow_2 = get_scaled_amount(
            Uint128::from(borrow_amount),
            expected_params_native.borrow_index,
        );
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

            amount: Uint128::from(83968_u128),
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
                attr("amount", repay_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![th_build_interests_updated_event(
                "borrowedcoinnative",
                &expected_params_native
            )]
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
            - get_scaled_amount(
                Uint128::from(repay_amount),
                expected_params_native.borrow_index,
            );
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

        let repay_amount: u128 = get_descaled_amount(
            expected_debt_scaled_2_after_repay_some_2,
            expected_params_native.borrow_index,
        )
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
                attr("amount", repay_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_debt_position_changed_event(
                    "borrowedcoinnative",
                    false,
                    "borrower".to_string()
                ),
                th_build_interests_updated_event("borrowedcoinnative", &expected_params_native)
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

        assert_eq!(Uint128::zero(), debt2.amount_scaled);
        assert_eq!(
            Uint128::zero(),
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

        let expected_repay_amount_scaled = get_scaled_amount(
            Uint128::from(repay_amount),
            expected_params_cw20.borrow_index,
        );
        let expected_refund_amount: u128 = (get_descaled_amount(
            expected_repay_amount_scaled - expected_debt_scaled_1_after_borrow_again,
            expected_params_cw20.borrow_index,
        ))
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
                    Uint128::new(repay_amount - expected_refund_amount).to_string()
                ),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_debt_position_changed_event(
                    "borrowedcoincw20",
                    false,
                    "borrower".to_string()
                ),
                th_build_interests_updated_event("borrowedcoincw20", &expected_params_cw20)
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
        assert_eq!(Uint128::zero(), debt1.amount_scaled);
        assert_eq!(Uint128::zero(), market_1_after_repay_1.debt_total_scaled);
    }

    #[test]
    fn test_borrow_uusd() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "uusd")]);
        let block_time = 1;

        let borrower_addr = Addr::unchecked("borrower");
        let ltv = Decimal::from_ratio(7u128, 10u128);

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal::one(),
            max_loan_to_value: ltv,
            borrow_index: Decimal::from_ratio(20u128, 10u128),
            borrow_rate: Decimal::one(),
            liquidity_rate: Decimal::one(),
            debt_total_scaled: Uint128::zero(),
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
        let deposit_amount_scaled = Uint128::new(110_000 * SCALING_FACTOR);
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &borrower_addr, &user)
            .unwrap();

        // Set the querier to return collateral balance
        let deposit_coin_address = Addr::unchecked("matoken");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(borrower_addr.clone(), deposit_amount_scaled.into())],
        );

        // borrow with insufficient collateral, should fail
        let new_block_time = 120u64;
        let time_elapsed = new_block_time - market.interests_last_updated;
        let liquidity_index = calculate_applied_linear_interest_rate(
            market.liquidity_index,
            market.liquidity_rate,
            time_elapsed,
        );
        let collateral = get_descaled_amount(deposit_amount_scaled, liquidity_index);
        let max_to_borrow = collateral * ltv;
        let msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "uusd".to_string(),
            },
            amount: max_to_borrow + Uint128::from(1u128),
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

        let valid_amount = max_to_borrow - Uint128::from(1000u128);
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

        assert_eq!(
            valid_amount,
            get_descaled_amount(debt.amount_scaled, market_after_borrow.borrow_index)
        );
        assert_eq!(
            market_after_borrow.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn test_borrow_full_liquidity_and_then_repay() {
        let initial_liquidity = 50000;
        let mut deps = th_setup(&[coin(initial_liquidity, "uusd")]);
        let info = mock_info("borrower");
        let borrower_addr = Addr::unchecked("borrower");
        let block_time = 1;
        let ltv = Decimal::one();

        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            liquidity_index: Decimal::one(),
            max_loan_to_value: ltv,
            borrow_index: Decimal::one(),
            borrow_rate: Decimal::one(),
            liquidity_rate: Decimal::one(),
            debt_total_scaled: Uint128::zero(),
            reserve_factor: Decimal::from_ratio(12u128, 100u128),
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

        // User should have amount of collateral more than initial liquidity in order to borrow full liquidity
        let deposit_amount = initial_liquidity + 1000u128;
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market.index).unwrap();
        USERS
            .save(deps.as_mut().storage, &borrower_addr, &user)
            .unwrap();

        // Set the querier to return collateral balance
        let deposit_coin_address = Addr::unchecked("matoken");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(
                borrower_addr.clone(),
                Uint128::new(deposit_amount * SCALING_FACTOR),
            )],
        );

        // Borrow full liquidity
        {
            let env = mock_env_at_block_time(block_time);
            let msg = ExecuteMsg::Borrow {
                asset: Asset::Native {
                    denom: "uusd".to_string(),
                },
                amount: initial_liquidity.into(),
            };
            let _res = execute(deps.as_mut(), env, info.clone(), msg).unwrap();

            let market_after_borrow = MARKETS.load(&deps.storage, b"uusd").unwrap();
            let debt_total = get_descaled_amount(
                market_after_borrow.debt_total_scaled,
                market_after_borrow.borrow_index,
            );
            assert_eq!(debt_total, initial_liquidity.into());
        }

        let new_block_time = 12000u64;
        // We need to update balance after borrowing
        deps.querier.set_contract_balances(&[coin(0, "uusd")]);

        // Try to borrow more than available liquidity
        {
            let env = mock_env_at_block_time(new_block_time);
            let msg = ExecuteMsg::Borrow {
                asset: Asset::Native {
                    denom: "uusd".to_string(),
                },
                amount: 100u128.into(),
            };
            let error_res = execute(deps.as_mut(), env, info.clone(), msg).unwrap_err();
            assert_eq!(
                error_res,
                StdError::generic_err(
                    "Protocol income to be distributed and liquidity taken cannot be greater than available liquidity"
                )
                    .into()
            );
        }

        // Repay part of the debt
        {
            let env = mock_env_at_block_time(new_block_time);
            let info = cosmwasm_std::testing::mock_info("borrower", &[coin(2000, "uusd")]);
            let msg = ExecuteMsg::RepayNative {
                denom: String::from("uusd"),
            };
            let _res = execute(deps.as_mut(), env, info, msg).unwrap();

            let market_after_deposit = MARKETS.load(&deps.storage, b"uusd").unwrap();
            assert!(!market_after_deposit.protocol_income_to_distribute.is_zero());
        }
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

        deps.querier
            .set_oracle_price(cw20_contract_addr.as_bytes().to_vec(), exchange_rate_1);
        deps.querier
            .set_oracle_price(b"depositedcoin2".to_vec(), exchange_rate_2);
        // NOTE: uusd price (asset3) should be set to 1 by the oracle helper

        let exchange_rates = &[(String::from("depositedcoin2"), exchange_rate_2)];
        deps.querier
            .set_native_exchange_rates(String::from("uusd"), &exchange_rates[..]);

        let mock_market_1 = Market {
            ma_token_address: Addr::unchecked("matoken1"),
            max_loan_to_value: Decimal::from_ratio(8u128, 10u128),
            debt_total_scaled: Uint128::zero(),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::from_ratio(1u128, 2u128),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: Addr::unchecked("matoken2"),
            max_loan_to_value: Decimal::from_ratio(6u128, 10u128),
            debt_total_scaled: Uint128::zero(),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::from_ratio(1u128, 2u128),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: Addr::unchecked("matoken3"),
            max_loan_to_value: Decimal::from_ratio(4u128, 10u128),
            debt_total_scaled: Uint128::zero(),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::from_ratio(1u128, 2u128),
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

        let balance_1 = Uint128::new(4_000_000 * SCALING_FACTOR);
        let balance_2 = Uint128::new(7_000_000 * SCALING_FACTOR);
        let balance_3 = Uint128::new(3_000_000 * SCALING_FACTOR);

        // Set the querier to return a certain collateral balance
        deps.querier.set_cw20_balances(
            ma_token_address_1,
            &[(borrower_addr.clone(), balance_1.into())],
        );
        deps.querier.set_cw20_balances(
            ma_token_address_2,
            &[(borrower_addr.clone(), balance_2.into())],
        );
        deps.querier
            .set_cw20_balances(ma_token_address_3, &[(borrower_addr, balance_3.into())]);

        let max_borrow_allowed_in_uusd = (market_1_initial.max_loan_to_value
            * get_descaled_amount(balance_1, market_1_initial.liquidity_index)
            * exchange_rate_1)
            + (market_2_initial.max_loan_to_value
                * get_descaled_amount(balance_2, market_2_initial.liquidity_index)
                * exchange_rate_2)
            + (market_3_initial.max_loan_to_value
                * get_descaled_amount(balance_3, market_3_initial.liquidity_index)
                * exchange_rate_3);
        let exceeding_borrow_amount = (max_borrow_allowed_in_uusd
            * reverse_decimal(exchange_rate_2))
            + Uint128::from(100_u64);
        let permissible_borrow_amount = (max_borrow_allowed_in_uusd
            * reverse_decimal(exchange_rate_2))
            - Uint128::from(100_u64);

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
        let available_liquidity_cw20_debt = 2_000_000_000u128;
        let available_liquidity_native_debt = 2_000_000_000u128;
        let mut deps = th_setup(&[
            coin(available_liquidity_collateral, "collateral"),
            coin(available_liquidity_native_debt, "native_debt"),
        ]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[
                (String::from("collateral"), Uint128::new(100u128)),
                (String::from("native_debt"), Uint128::new(120u128)),
            ],
        );

        let cw20_debt_contract_addr = Addr::unchecked("cw20_debt");
        let user_address = Addr::unchecked("user");
        let liquidator_address = Addr::unchecked("liquidator");

        let collateral_max_ltv = Decimal::from_ratio(5u128, 10u128);
        let collateral_maintenance_margin = Decimal::from_ratio(6u128, 10u128);
        let collateral_liquidation_bonus = Decimal::from_ratio(1u128, 10u128);
        let collateral_price = Decimal::from_ratio(2_u128, 1_u128);
        let cw20_debt_price = Decimal::from_ratio(11_u128, 10_u128);
        let native_debt_price = Decimal::from_ratio(15_u128, 10_u128);
        let user_collateral_balance = 2_000_000;
        let user_debt = Uint128::from(3_000_000_u64); // ltv = 0.75
        let close_factor = Decimal::from_ratio(1u128, 2u128);

        let first_debt_to_repay = Uint128::from(400_000_u64);
        let first_block_time = 15_000_000;

        let second_debt_to_repay = Uint128::from(10_000_000_u64);
        let second_block_time = 16_000_000;

        // Global debt for the debt market
        let mut expected_global_cw20_debt_scaled = Uint128::new(1_800_000_000 * SCALING_FACTOR);
        let mut expected_global_native_debt_scaled = Uint128::new(500_000_000 * SCALING_FACTOR);

        CONFIG
            .update(deps.as_mut().storage, |mut config| -> StdResult<_> {
                config.close_factor = close_factor;
                Ok(config)
            })
            .unwrap();

        deps.querier.set_cw20_balances(
            cw20_debt_contract_addr.clone(),
            &[(
                Addr::unchecked(MOCK_CONTRACT_ADDR),
                Uint128::new(available_liquidity_cw20_debt),
            )],
        );

        // initialize collateral and debt markets
        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[
                ("collateral".to_string(), collateral_price),
                ("native_debt".to_string(), native_debt_price),
            ],
        );

        deps.querier
            .set_oracle_price(b"collateral".to_vec(), collateral_price);
        deps.querier
            .set_oracle_price(cw20_debt_contract_addr.as_bytes().to_vec(), cw20_debt_price);
        deps.querier
            .set_oracle_price(b"native_debt".to_vec(), native_debt_price);

        let collateral_market_ma_token_addr = Addr::unchecked("ma_collateral");
        let collateral_market = Market {
            ma_token_address: collateral_market_ma_token_addr.clone(),
            max_loan_to_value: collateral_max_ltv,
            maintenance_margin: collateral_maintenance_margin,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint128::new(800_000_000 * SCALING_FACTOR),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            borrow_rate: Decimal::from_ratio(2u128, 10u128),
            liquidity_rate: Decimal::from_ratio(2u128, 10u128),
            reserve_factor: Decimal::from_ratio(2u128, 100u128),
            asset_type: AssetType::Native,
            interests_last_updated: 0,
            ..Default::default()
        };

        let cw20_debt_market = Market {
            max_loan_to_value: Decimal::from_ratio(6u128, 10u128),
            debt_total_scaled: expected_global_cw20_debt_scaled,
            liquidity_index: Decimal::from_ratio(12u128, 10u128),
            borrow_index: Decimal::from_ratio(14u128, 10u128),
            borrow_rate: Decimal::from_ratio(2u128, 10u128),
            liquidity_rate: Decimal::from_ratio(2u128, 10u128),
            reserve_factor: Decimal::from_ratio(3u128, 100u128),
            asset_type: AssetType::Cw20,
            interests_last_updated: 0,
            ..Default::default()
        };

        let native_debt_market = Market {
            max_loan_to_value: Decimal::from_ratio(4u128, 10u128),
            debt_total_scaled: expected_global_native_debt_scaled,
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            borrow_rate: Decimal::from_ratio(3u128, 10u128),
            liquidity_rate: Decimal::from_ratio(3u128, 10u128),
            reserve_factor: Decimal::from_ratio(2u128, 100u128),
            asset_type: AssetType::Native,
            interests_last_updated: 0,
            ..Default::default()
        };

        let collateral_market_initial =
            th_init_market(deps.as_mut(), b"collateral", &collateral_market);

        let cw20_debt_market_initial = th_init_market(
            deps.as_mut(),
            cw20_debt_contract_addr.as_bytes(),
            &cw20_debt_market,
        );

        let native_debt_market_initial =
            th_init_market(deps.as_mut(), b"native_debt", &native_debt_market);

        let mut expected_user_cw20_debt_scaled =
            get_scaled_amount(user_debt, cw20_debt_market_initial.borrow_index);

        // Set user as having collateral and debt in respective markets
        {
            let mut user = User::default();
            set_bit(&mut user.collateral_assets, collateral_market_initial.index).unwrap();
            set_bit(&mut user.borrowed_assets, cw20_debt_market_initial.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &user_address, &user)
                .unwrap();
        }

        // trying to liquidate user with zero collateral balance should fail
        {
            deps.querier.set_cw20_balances(
                collateral_market_ma_token_addr.clone(),
                &[(user_address.clone(), Uint128::zero())],
            );

            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(cw20_debt_contract_addr.as_str());
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
            collateral_market_ma_token_addr.clone(),
            &[(
                user_address.clone(),
                Uint128::new(user_collateral_balance * SCALING_FACTOR),
            )],
        );

        // trying to liquidate user with zero outstanding debt should fail (uncollateralized has not impact)
        {
            let debt = Debt {
                amount_scaled: Uint128::zero(),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint128::new(10_000 * SCALING_FACTOR),
                uncollateralized: true,
            };
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
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
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(cw20_debt_contract_addr.as_str());
            let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
            assert_eq!(error_res, StdError::generic_err("User has no outstanding debt in the specified debt asset and thus cannot be liquidated").into());
        }

        // set user to have positive debt amount in debt asset
        {
            let debt = Debt {
                amount_scaled: expected_user_cw20_debt_scaled,
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint128::new(10_000 * SCALING_FACTOR),
                uncollateralized: true,
            };
            DEBTS
                .save(
                    deps.as_mut().storage,
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
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

        // trying to liquidate without sending funds should fail
        {
            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: Uint128::zero(),
            });

            let env = mock_env(MockEnvParams::default());
            let info = mock_info(cw20_debt_contract_addr.as_str());
            let error_res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap_err();
            assert_eq!(
                error_res,
                StdError::generic_err("Must send more than 0 cw20_debt in order to liquidate")
                    .into()
            );
        }

        // Perform first successful liquidation receiving ma_token in return
        {
            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    user_address: user_address.to_string(),
                    receive_ma_token: true,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: first_debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = first_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info(cw20_debt_contract_addr.as_str());
            let res = execute(deps.as_mut(), env.clone(), info, liquidate_msg).unwrap();

            // get expected indices and rates for debt market
            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &cw20_debt_market_initial,
                block_time,
                available_liquidity_cw20_debt,
                TestUtilizationDeltas {
                    less_debt: first_debt_to_repay.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_after = MARKETS
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            let expected_liquidated_collateral_amount = first_debt_to_repay
                * cw20_debt_price
                * (Decimal::one() + collateral_liquidation_bonus)
                * reverse_decimal(collateral_price);

            let expected_liquidated_collateral_amount_scaled = get_scaled_amount(
                expected_liquidated_collateral_amount,
                get_updated_liquidity_index(&collateral_market_after, env.block.time.seconds()),
            );

            assert_eq!(
                res.messages,
                vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: collateral_market_ma_token_addr.to_string(),
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
                    attr("debt_market", cw20_debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount.to_string(),
                    ),
                    attr("debt_amount_repaid", first_debt_to_repay.to_string()),
                    attr("refund_amount", "0"),
                ],
            );
            assert_eq!(
                res.events,
                vec![
                    build_collateral_position_changed_event(
                        "collateral",
                        true,
                        liquidator_address.to_string()
                    ),
                    th_build_interests_updated_event(
                        cw20_debt_contract_addr.as_str(),
                        &expected_debt_rates
                    )
                ]
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
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            let expected_less_debt_scaled =
                get_scaled_amount(first_debt_to_repay, expected_debt_rates.borrow_index);

            expected_user_cw20_debt_scaled =
                expected_user_cw20_debt_scaled - expected_less_debt_scaled;

            assert_eq!(expected_user_cw20_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_cw20_debt_scaled =
                expected_global_cw20_debt_scaled - expected_less_debt_scaled;

            assert_eq!(
                expected_global_cw20_debt_scaled,
                debt_market_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                Uint128::zero(),
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
                    user_address: user_address.to_string(),
                    receive_ma_token: false,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: second_debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = second_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info(cw20_debt_contract_addr.as_str());
            let res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_debt_indices = th_get_expected_indices(&debt_market_before, block_time);
            let user_debt_asset_total_debt =
                get_descaled_amount(expected_user_cw20_debt_scaled, expected_debt_indices.borrow);
            // Since debt is being over_repayed, we expect to max out the liquidatable debt
            let expected_less_debt = user_debt_asset_total_debt * close_factor;

            let expected_refund_amount = second_debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_before,
                block_time,
                available_liquidity_cw20_debt, // this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_debt: expected_less_debt.into(),
                    less_liquidity: expected_refund_amount.into(),
                    ..Default::default()
                },
            );

            let expected_liquidated_collateral_amount = expected_less_debt
                * cw20_debt_price
                * (Decimal::one() + collateral_liquidation_bonus)
                * reverse_decimal(collateral_price);

            let expected_collateral_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &collateral_market_before,
                block_time,
                available_liquidity_collateral, // this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: expected_liquidated_collateral_amount.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_after = MARKETS
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            let expected_liquidated_collateral_amount_scaled = get_scaled_amount(
                expected_liquidated_collateral_amount,
                expected_collateral_rates.liquidity_index,
            );

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: collateral_market_ma_token_addr.to_string(),
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
                                amount: expected_liquidated_collateral_amount,
                            }
                        )
                        .unwrap()],
                    })),
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: cw20_debt_contract_addr.to_string(),
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            recipient: liquidator_address.to_string(),
                            amount: expected_refund_amount,
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
                    attr("debt_market", cw20_debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount,
                    ),
                    attr("debt_amount_repaid", expected_less_debt.to_string()),
                    attr("refund_amount", expected_refund_amount.to_string()),
                ],
                res.attributes,
            );
            assert_eq!(
                res.events,
                vec![
                    th_build_interests_updated_event(
                        cw20_debt_contract_addr.as_str(),
                        &expected_debt_rates
                    ),
                    th_build_interests_updated_event("collateral", &expected_collateral_rates),
                ]
            );

            // check user still has deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, cw20_debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled =
                get_scaled_amount(expected_less_debt, expected_debt_rates.borrow_index);
            expected_user_cw20_debt_scaled =
                expected_user_cw20_debt_scaled - expected_less_debt_scaled;

            let debt = DEBTS
                .load(
                    &deps.storage,
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            assert_eq!(expected_user_cw20_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_cw20_debt_scaled =
                expected_global_cw20_debt_scaled - expected_less_debt_scaled;
            assert_eq!(
                expected_global_cw20_debt_scaled,
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
            let user_collateral_balance_scaled = Uint128::new(100 * SCALING_FACTOR);
            let mut expected_user_debt_scaled = Uint128::new(400 * SCALING_FACTOR);
            let debt_to_repay = Uint128::from(300u128);

            // Set the querier to return positive collateral balance
            deps.querier.set_cw20_balances(
                collateral_market_ma_token_addr.clone(),
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
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
                    &debt,
                )
                .unwrap();

            let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
                msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    user_address: user_address.to_string(),
                    receive_ma_token: false,
                })
                .unwrap(),
                sender: liquidator_address.to_string(),
                amount: debt_to_repay.into(),
            });

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            let block_time = second_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = mock_info(cw20_debt_contract_addr.as_str());
            let res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_collateral_indices =
                th_get_expected_indices(&collateral_market_before, block_time);
            let user_collateral_balance = get_descaled_amount(
                user_collateral_balance_scaled,
                expected_collateral_indices.liquidity,
            );

            // Since debt is being over_repayed, we expect to liquidate total collateral
            let expected_less_debt = collateral_price
                * user_collateral_balance
                * reverse_decimal(cw20_debt_price)
                * reverse_decimal(Decimal::one() + collateral_liquidation_bonus);

            let expected_refund_amount = debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_before,
                block_time,
                available_liquidity_cw20_debt, // this is the same as before as it comes from mocks
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
                .load(&deps.storage, cw20_debt_contract_addr.as_bytes())
                .unwrap();

            // NOTE: expected_liquidated_collateral_amount_scaled should be equal user_collateral_balance_scaled
            // but there are rounding errors
            let expected_liquidated_collateral_amount_scaled = get_scaled_amount(
                user_collateral_balance,
                expected_collateral_rates.liquidity_index,
            );

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: collateral_market_ma_token_addr.to_string(),
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
                                amount: user_collateral_balance,
                            }
                        )
                        .unwrap()],
                    })),
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: cw20_debt_contract_addr.to_string(),
                        msg: to_binary(&Cw20ExecuteMsg::Transfer {
                            recipient: liquidator_address.to_string(),
                            amount: expected_refund_amount,
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
                    attr("debt_market", cw20_debt_contract_addr.as_str()),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        user_collateral_balance.to_string(),
                    ),
                    attr("debt_amount_repaid", expected_less_debt.to_string()),
                    attr("refund_amount", expected_refund_amount.to_string()),
                ],
                res.attributes,
            );
            assert_eq!(
                res.events,
                vec![
                    build_collateral_position_changed_event(
                        "collateral",
                        false,
                        user_address.to_string()
                    ),
                    th_build_interests_updated_event(
                        cw20_debt_contract_addr.as_str(),
                        &expected_debt_rates
                    ),
                    th_build_interests_updated_event("collateral", &expected_collateral_rates),
                ]
            );

            // check user doesn't have deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(!get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, cw20_debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled =
                get_scaled_amount(expected_less_debt, expected_debt_rates.borrow_index);
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt = DEBTS
                .load(
                    &deps.storage,
                    (cw20_debt_contract_addr.as_bytes(), &user_address),
                )
                .unwrap();

            assert_eq!(expected_user_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_cw20_debt_scaled =
                expected_global_cw20_debt_scaled - expected_less_debt_scaled;
            assert_eq!(
                expected_global_cw20_debt_scaled,
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

        // Perform native liquidation receiving ma_token in return
        {
            let mut user = User::default();
            set_bit(&mut user.collateral_assets, collateral_market_initial.index).unwrap();
            set_bit(&mut user.borrowed_assets, native_debt_market_initial.index).unwrap();
            USERS
                .save(deps.as_mut().storage, &user_address, &user)
                .unwrap();

            let user_collateral_balance_scaled = Uint128::new(200 * SCALING_FACTOR);
            let mut expected_user_debt_scaled = Uint128::new(800 * SCALING_FACTOR);
            let debt_to_repay = Uint128::from(500u128);

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
                    (b"native_debt", &user_address),
                    &debt,
                )
                .unwrap();

            let liquidate_msg = ExecuteMsg::LiquidateNative {
                collateral_asset: Asset::Native {
                    denom: "collateral".to_string(),
                },
                debt_asset_denom: "native_debt".to_string(),
                user_address: user_address.to_string(),
                receive_ma_token: false,
            };

            let collateral_market_before = MARKETS.load(&deps.storage, b"collateral").unwrap();
            let debt_market_before = MARKETS.load(&deps.storage, b"native_debt").unwrap();

            let block_time = second_block_time;
            let env = mock_env_at_block_time(block_time);
            let info = cosmwasm_std::testing::mock_info(
                liquidator_address.as_str(),
                &[coin(debt_to_repay.u128(), "native_debt")],
            );
            let res = execute(deps.as_mut(), env, info, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_collateral_indices =
                th_get_expected_indices(&collateral_market_before, block_time);
            let user_collateral_balance = get_descaled_amount(
                user_collateral_balance_scaled,
                expected_collateral_indices.liquidity,
            );

            // Since debt is being over_repayed, we expect to liquidate total collateral
            let expected_less_debt = collateral_price
                * user_collateral_balance
                * reverse_decimal(native_debt_price)
                * reverse_decimal(Decimal::one() + collateral_liquidation_bonus);

            let expected_refund_amount = debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps.as_ref(),
                &debt_market_before,
                block_time,
                available_liquidity_native_debt, // this is the same as before as it comes from mocks
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
            let debt_market_after = MARKETS.load(&deps.storage, b"native_debt").unwrap();

            // NOTE: expected_liquidated_collateral_amount_scaled should be equal user_collateral_balance_scaled
            // but there are rounding errors
            let expected_liquidated_collateral_amount_scaled = get_scaled_amount(
                user_collateral_balance,
                expected_collateral_rates.liquidity_index,
            );

            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: collateral_market_ma_token_addr.to_string(),
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
                                amount: user_collateral_balance,
                            }
                        )
                        .unwrap()],
                    })),
                    SubMsg::new(CosmosMsg::Bank(BankMsg::Send {
                        to_address: liquidator_address.to_string(),
                        amount: vec![deduct_tax(
                            deps.as_ref(),
                            Coin {
                                denom: String::from("native_debt"),
                                amount: expected_refund_amount,
                            }
                        )
                        .unwrap()],
                    }))
                ]
            );

            mars::testing::assert_eq_vec(
                vec![
                    attr("action", "liquidate"),
                    attr("collateral_market", "collateral"),
                    attr("debt_market", "native_debt"),
                    attr("user", user_address.as_str()),
                    attr("liquidator", liquidator_address.as_str()),
                    attr(
                        "collateral_amount_liquidated",
                        user_collateral_balance.to_string(),
                    ),
                    attr("debt_amount_repaid", expected_less_debt.to_string()),
                    attr("refund_amount", expected_refund_amount.to_string()),
                ],
                res.attributes,
            );
            assert_eq!(
                res.events,
                vec![
                    build_collateral_position_changed_event(
                        "collateral",
                        false,
                        user_address.to_string()
                    ),
                    th_build_interests_updated_event("native_debt", &expected_debt_rates),
                    th_build_interests_updated_event("collateral", &expected_collateral_rates),
                ]
            );

            // check user doesn't have deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = USERS.load(&deps.storage, &user_address).unwrap();
            assert!(!get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, native_debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled =
                get_scaled_amount(expected_less_debt, expected_debt_rates.borrow_index);
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt = DEBTS
                .load(&deps.storage, (b"native_debt", &user_address))
                .unwrap();

            assert_eq!(expected_user_debt_scaled, debt.amount_scaled);

            // check global debt decreased by the appropriate amount
            expected_global_native_debt_scaled =
                expected_global_native_debt_scaled - expected_less_debt_scaled;
            assert_eq!(
                expected_global_native_debt_scaled,
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

        deps.querier
            .set_oracle_price(b"collateral".to_vec(), Decimal::one());
        deps.querier
            .set_oracle_price(b"debt".to_vec(), Decimal::one());

        let collateral_ltv = Decimal::from_ratio(5u128, 10u128);
        let collateral_maintenance_margin = Decimal::from_ratio(7u128, 10u128);
        let collateral_liquidation_bonus = Decimal::from_ratio(1u128, 10u128);

        let collateral_market = Market {
            ma_token_address: Addr::unchecked("collateral"),
            max_loan_to_value: collateral_ltv,
            maintenance_margin: collateral_maintenance_margin,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint128::zero(),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let debt_market = Market {
            ma_token_address: Addr::unchecked("debt"),
            max_loan_to_value: Decimal::from_ratio(6u128, 10u128),
            debt_total_scaled: Uint128::new(20_000_000 * SCALING_FACTOR),
            liquidity_index: Decimal::one(),
            borrow_index: Decimal::one(),
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
        let healthy_user_collateral_balance_scaled = Uint128::new(10_000_000 * SCALING_FACTOR);

        // Set the querier to return a certain collateral balance
        deps.querier.set_cw20_balances(
            collateral_address,
            &[(
                healthy_user_address.clone(),
                healthy_user_collateral_balance_scaled.into(),
            )],
        );

        let healthy_user_debt_amount_scaled =
            Uint128::new(healthy_user_collateral_balance_scaled.u128())
                * collateral_maintenance_margin;
        let healthy_user_debt = Debt {
            amount_scaled: healthy_user_debt_amount_scaled.into(),
            uncollateralized: false,
        };
        let uncollateralized_debt = Debt {
            amount_scaled: Uint128::new(10_000 * SCALING_FACTOR),
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
        let debt_to_cover = Uint128::from(1_000_000u64);

        let liquidate_msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            msg: to_binary(&ReceiveMsg::LiquidateCw20 {
                collateral_asset: Asset::Native {
                    denom: "collateral".to_string(),
                },
                user_address: healthy_user_address.to_string(),
                receive_ma_token: true,
            })
            .unwrap(),
            sender: liquidator_address.to_string(),
            amount: debt_to_cover,
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
            liquidity_index: Decimal::one(),
            maintenance_margin: Decimal::from_ratio(5u128, 10u128),
            ..Default::default()
        };
        let market = th_init_market(deps.as_mut(), b"somecoin", &mock_market);
        let debt_mock_market = Market {
            borrow_index: Decimal::one(),
            ..Default::default()
        };
        let debt_market = th_init_market(deps.as_mut(), b"debtcoin", &debt_mock_market);

        deps.querier
            .set_oracle_price(b"somecoin".to_vec(), Decimal::from_ratio(1u128, 2u128));
        deps.querier
            .set_oracle_price(b"debtcoin".to_vec(), Decimal::from_ratio(2u128, 1u128));

        let sender_address = Addr::unchecked("fromaddr");
        let recipient_address = Addr::unchecked("toaddr");

        deps.querier.set_cw20_balances(
            Addr::unchecked("masomecoin"),
            &[(
                sender_address.clone(),
                Uint128::new(500_000 * SCALING_FACTOR),
            )],
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
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
                sender_previous_balance: Uint128::new(1_000_000),
                recipient_previous_balance: Uint128::new(0),
                amount: Uint128::new(500_000),
            };

            let res = execute(deps.as_mut(), env.clone(), info_matoken.clone(), msg).unwrap();

            let sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            let recipient_user = USERS.load(&deps.storage, &recipient_address).unwrap();
            assert!(get_bit(sender_user.collateral_assets, market.index).unwrap());
            // Should create user and set deposited to true as previous balance is 0
            assert!(get_bit(recipient_user.collateral_assets, market.index).unwrap());

            assert_eq!(
                res.events,
                vec![build_collateral_position_changed_event(
                    "somecoin",
                    true,
                    recipient_address.to_string()
                )]
            );
        }

        // Finalize transfer with health factor < 1 for sender doesn't go through
        {
            // set debt for user in order for health factor to be < 1
            let debt = Debt {
                amount_scaled: Uint128::new(500_000 * SCALING_FACTOR),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint128::new(10_000 * SCALING_FACTOR),
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
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
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
                amount_scaled: Uint128::new(1_000 * SCALING_FACTOR),
                uncollateralized: false,
            };
            let uncollateralized_debt = Debt {
                amount_scaled: Uint128::new(10_000u128 * SCALING_FACTOR),
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
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
                sender_previous_balance: Uint128::new(500_000),
                recipient_previous_balance: Uint128::new(500_000),
                amount: Uint128::new(500_000),
            };

            let res = execute(deps.as_mut(), env.clone(), info_matoken, msg).unwrap();

            let sender_user = USERS.load(&deps.storage, &sender_address).unwrap();
            let recipient_user = USERS.load(&deps.storage, &recipient_address).unwrap();
            // Should set deposited to false as: previous_balance - amount = 0
            assert!(!get_bit(sender_user.collateral_assets, market.index).unwrap());
            assert!(get_bit(recipient_user.collateral_assets, market.index).unwrap());

            assert_eq!(
                res.events,
                vec![build_collateral_position_changed_event(
                    "somecoin",
                    false,
                    sender_address.to_string()
                )]
            );
        }

        // Calling this with other token fails
        {
            let msg = ExecuteMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address,
                recipient_address: recipient_address,
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
            borrow_index: Decimal::from_ratio(12u128, 10u128),
            liquidity_index: Decimal::from_ratio(8u128, 10u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(1u128, 10u128),
            debt_total_scaled: Uint128::zero(),
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
            amount: initial_borrow_amount,
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
                attr("amount", initial_borrow_amount.to_string()),
            ]
        );
        assert_eq!(
            res.events,
            vec![
                build_debt_position_changed_event("somecoin", true, "borrower".to_string()),
                th_build_interests_updated_event("somecoin", &expected_params)
            ]
        );

        // Check debt
        let user = USERS.load(&deps.storage, &borrower_addr).unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());

        let debt = DEBTS
            .load(&deps.storage, (b"somecoin", &borrower_addr))
            .unwrap();

        let expected_debt_scaled_after_borrow =
            get_scaled_amount(initial_borrow_amount, expected_params.borrow_index);

        assert_eq!(expected_debt_scaled_after_borrow, debt.amount_scaled);

        // Borrow an amount less than initial limit but exceeding current limit
        let remaining_limit = initial_uncollateralized_loan_limit - initial_borrow_amount;
        let exceeding_limit = remaining_limit + Uint128::from(100_u64);

        block_time += 1000_u64;
        let borrow_msg = ExecuteMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: exceeding_limit,
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
            amount: remaining_limit,
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
        let protocol_income_to_distribute = Uint128::from(1_000_000_u64);

        // initialize market with non-zero amount of protocol_income_to_distribute
        let mock_market = Market {
            ma_token_address: Addr::unchecked("matoken"),
            borrow_index: Decimal::from_ratio(12u128, 10u128),
            liquidity_index: Decimal::from_ratio(8u128, 10u128),
            borrow_rate: Decimal::from_ratio(20u128, 100u128),
            liquidity_rate: Decimal::from_ratio(10u128, 100u128),
            reserve_factor: Decimal::from_ratio(1u128, 10u128),
            debt_total_scaled: Uint128::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            protocol_income_to_distribute,
            ..Default::default()
        };
        // should get index 0
        let market_initial = th_init_market(deps.as_mut(), b"somecoin", &mock_market);

        let mut block_time = mock_market.interests_last_updated + 10000u64;

        // call function providing amount exceeding protocol_income_to_distribute, should fail
        let exceeding_amount = protocol_income_to_distribute + Uint128::from(1_000_u64);
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
        let permissible_amount = Decimal::from_ratio(1u128, 2u128) * protocol_income_to_distribute;
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

        let scaled_mint_amount = get_scaled_amount(
            expected_treasury_amount,
            get_updated_liquidity_index(&market_initial, env.block.time.seconds()),
        );

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

        let scaled_mint_amount = get_scaled_amount(
            expected_treasury_amount,
            get_updated_liquidity_index(&market_after_distribution, env.block.time.seconds()),
        );

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
            Uint128::zero()
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
            insurance_fund_fee_share: Some(Decimal::from_ratio(5u128, 10u128)),
            treasury_fee_share: Some(Decimal::from_ratio(3u128, 10u128)),
            ma_token_code_id: Some(1u64),
            close_factor: Some(Decimal::from_ratio(1u128, 2u128)),
        };
        let msg = InstantiateMsg { config };
        instantiate(deps.as_mut(), env, info, msg).unwrap();
        deps
    }

    impl Default for Market {
        fn default() -> Self {
            let dynamic_ir = DynamicInterestRate {
                min_borrow_rate: Decimal::zero(),
                max_borrow_rate: Decimal::one(),
                kp_1: Default::default(),
                optimal_utilization_rate: Default::default(),
                kp_augmentation_threshold: Default::default(),
                kp_2: Default::default(),
            };
            Market {
                index: 0,
                ma_token_address: zero_address(),
                liquidity_index: Default::default(),
                borrow_index: Default::default(),
                borrow_rate: Default::default(),
                liquidity_rate: Default::default(),
                max_loan_to_value: Default::default(),
                reserve_factor: Default::default(),
                interests_last_updated: 0,
                debt_total_scaled: Default::default(),
                asset_type: AssetType::Native,
                maintenance_margin: Decimal::one(),
                liquidation_bonus: Decimal::zero(),
                protocol_income_to_distribute: Uint128::zero(),
                interest_rate_strategy: InterestRateStrategy::Dynamic(dynamic_ir),
            }
        }
    }

    fn th_init_market(deps: DepsMut, key: &[u8], market: &Market) -> Market {
        let mut index = 0;

        GLOBAL_STATE
            .update(
                deps.storage,
                |mut mm: GlobalState| -> StdResult<GlobalState> {
                    index = mm.market_count;
                    mm.market_count += 1;
                    Ok(mm)
                },
            )
            .unwrap();

        let new_market = Market {
            index,
            ..market.clone()
        };

        MARKETS.save(deps.storage, key, &new_market).unwrap();

        MARKET_REFERENCES_BY_INDEX
            .save(deps.storage, U32Key::new(index), &key.to_vec())
            .unwrap();

        MARKET_REFERENCES_BY_MA_TOKEN
            .save(deps.storage, &new_market.ma_token_address, &key.to_vec())
            .unwrap();

        new_market
    }

    #[derive(Default, Debug)]
    struct TestInterestResults {
        borrow_index: Decimal,
        liquidity_index: Decimal,
        borrow_rate: Decimal,
        liquidity_rate: Decimal,
        protocol_income_to_distribute: Uint128,
    }

    fn th_build_interests_updated_event(label: &str, ir: &TestInterestResults) -> Event {
        Event::new("interests_updated")
            .add_attribute("market", label)
            .add_attribute("borrow_index", ir.borrow_index.to_string())
            .add_attribute("liquidity_index", ir.liquidity_index.to_string())
            .add_attribute("borrow_rate", ir.borrow_rate.to_string())
            .add_attribute("liquidity_rate", ir.liquidity_rate.to_string())
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

        let expected_protocol_income_to_distribute =
            th_get_expected_protocol_income(market, &expected_indices);

        // When borrowing, new computed index is used for scaled amount
        let more_debt_scaled =
            get_scaled_amount(Uint128::from(deltas.more_debt), expected_indices.borrow);
        // When repaying, new computed index is used for scaled amount
        let less_debt_scaled =
            get_scaled_amount(Uint128::from(deltas.less_debt), expected_indices.borrow);
        // NOTE: Don't panic here so that the total repay of debt can be simulated
        // when less debt is greater than outstanding debt
        let new_debt_total_scaled =
            if (market.debt_total_scaled + more_debt_scaled) > less_debt_scaled {
                market.debt_total_scaled + more_debt_scaled - less_debt_scaled
            } else {
                Uint128::zero()
            };
        let dec_debt_total = get_descaled_amount(new_debt_total_scaled, expected_indices.borrow);
        let total_protocol_income_to_distribute =
            market.protocol_income_to_distribute + expected_protocol_income_to_distribute;

        let config = CONFIG.load(deps.storage).unwrap();

        let dec_protocol_income_minus_treasury_amount =
            (Decimal::one() - config.treasury_fee_share) * total_protocol_income_to_distribute;
        let contract_current_balance = Uint128::from(initial_liquidity);
        let liquidity_taken = Uint128::from(deltas.less_liquidity);
        let dec_liquidity_total =
            contract_current_balance - liquidity_taken - dec_protocol_income_minus_treasury_amount;
        let expected_utilization_rate =
            Decimal::from_ratio(dec_debt_total, dec_liquidity_total + dec_debt_total);

        // interest rates
        let (expected_borrow_rate, expected_liquidity_rate) =
            market.interest_rate_strategy.get_updated_interest_rates(
                expected_utilization_rate,
                market.borrow_rate,
                market.reserve_factor,
            );

        TestInterestResults {
            borrow_index: expected_indices.borrow,
            liquidity_index: expected_indices.liquidity,
            borrow_rate: expected_borrow_rate,
            liquidity_rate: expected_liquidity_rate,
            protocol_income_to_distribute: expected_protocol_income_to_distribute,
        }
    }

    /// Compute protocol income to be distributed (using values up to the instant
    /// before the contract call is made)
    fn th_get_expected_protocol_income(
        market: &Market,
        expected_indices: &TestExpectedIndices,
    ) -> Uint128 {
        let previous_borrow_index = market.borrow_index;
        let previous_debt_total =
            get_descaled_amount(market.debt_total_scaled, previous_borrow_index);
        let current_debt_total =
            get_descaled_amount(market.debt_total_scaled, expected_indices.borrow);
        let interest_accrued = if current_debt_total > previous_debt_total {
            current_debt_total - previous_debt_total
        } else {
            Uint128::zero()
        };
        interest_accrued * market.reserve_factor
    }

    /// Expected results for applying accumulated interest
    struct TestExpectedIndices {
        liquidity: Decimal,
        borrow: Decimal,
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
