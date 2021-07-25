use std::str;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    attr, from_binary, to_binary, Addr, Api, Attribute, BankMsg, Binary, CanonicalAddr, Coin,
    CosmosMsg, DepsMut, Env, MessageInfo, Order, Querier, Response, StdError, StdResult, Storage,
    SubMsg, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};
use terra_cosmwasm::TerraQuerier;

use mars::address_provider;
use mars::address_provider::msg::MarsContract;
use mars::helpers::{cw20_get_balance, cw20_get_symbol, option_string_to_addr, zero_address};
use mars::ma_token;
use mars::red_bank::msg::{
    Asset, AssetType, CollateralInfo, CollateralResponse, ConfigResponse, CreateOrUpdateConfig,
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
            let deposit_amount = get_denom_amount_from_coins(&env.message.sent_funds, &denom);
            let depositor_address = env.message.sender.clone();
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
            let repay_amount = get_denom_amount_from_coins(&env.message.sent_funds, &denom);
            let repayer_address = env.message.sender.clone();
            handle_repay(
                deps,
                env,
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
            let sent_debt_asset_amount =
                get_denom_amount_from_coins(&env.message.sent_funds, &debt_asset);
            let sender = env.message.sender.clone();
            handle_liquidate(
                deps,
                env,
                sender,
                collateral_asset,
                Asset::Native { denom: debt_asset },
                user_address,
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
        } => execute_update_uncollateralized_loan_limit(
            deps,
            env,
            info,
            user_address,
            asset,
            new_limit,
        ),
        ExecuteMsg::UpdateUserCollateralAssetStatus { asset, enable } => {
            execute_update_user_collateral_asset_status(deps, env, info, asset, enable)
        }

        ExecuteMsg::DistributeProtocolIncome { asset, amount } => {
            handle_distribute_protocol_income(deps, env, asset, amount)
        }

        ExecuteMsg::Withdraw { asset, amount } => execute_withdraw(deps, env, asset, amount),
    }
}

/// Update config
pub fn execute_update_config(
    deps: DepsMut,
    env: Env,
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
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            ReceiveMsg::DepositCw20 {} => {
                let token_contract_address = info.sender.clone();
                execute_deposit(
                    deps,
                    env,
                    cw20_msg.sender,
                    deps.api
                        .canonical_address(&token_contract_address)?
                        .as_slice(),
                    token_contract_address.as_str(),
                    Uint256::from(cw20_msg.amount),
                )
            }
            ReceiveMsg::RepayCw20 {} => {
                let token_contract_address = env.message.sender.clone();

                handle_repay(
                    deps,
                    env,
                    cw20_msg.sender,
                    deps.api
                        .canonical_address(&token_contract_address)?
                        .as_slice(),
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
                if env.message.sender != debt_asset_address {
                    return Err(StdError::generic_err(format!(
                        "Incorrect asset, must send {} in order to liquidate",
                        debt_asset_address
                    )));
                }
                let sent_debt_asset_amount = Uint256::from(cw20_msg.amount);
                handle_liquidate(
                    deps,
                    env,
                    cw20_msg.sender,
                    collateral_asset,
                    Asset::Cw20 {
                        contract_addr: debt_asset_address,
                    },
                    user_address,
                    sent_debt_asset_amount,
                    receive_ma_token,
                )
            }
        }
    } else {
        Err(StdError::generic_err("Invalid Cw20ReceiveMsg"))
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
    let withdrawer_addr = info.sender.clone();

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&deps, &asset)?;
    let mut market = MARKETS.load(deps.storage, asset_reference.as_slice())?;

    let asset_ma_addr = market.ma_token_address;
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
        contract_addr: market.ma_token_address.into(),
        msg: to_binary(&ma_token::msg::ExecuteMsg::Burn {
            user: withdrawer_addr.into(),
            amount: withdraw_amount_scaled.into(),
        })?,
        funds: vec![],
    });

    let send_underlying_asset_msg = build_send_asset_msg(
        &deps,
        env.contract.address,
        withdrawer_addr.clone(),
        asset,
        withdraw_amount,
    )?;

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

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&deps, &asset)?;
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
                            minter: env.contract.address.into(),
                            cap: None,
                        }),
                        init_hook: Some(ma_token::msg::InitHook {
                            msg: to_binary(&ExecuteMsg::InitAssetTokenCallback {
                                reference: asset_reference,
                            })?,
                            contract_addr: env.contract.address.into(),
                        }),
                        red_bank_address: env.contract.address.into(),
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
    env: Env,
    info: MessageInfo,
    asset: Asset,
    asset_params: InitOrUpdateAssetParams,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(MarsError::Unauthorized {}.into());
    }

    let (asset_label, asset_reference, _asset_type) = asset_get_attributes(&deps, &asset)?;
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
    env: Env,
    info: MessageInfo,
    reference: Vec<u8>,
) -> Result<Response, ContractError> {
    let mut market = MARKETS.load(deps.storage, reference.as_slice())?;

    if market.ma_token_address == zero_address() {
        let ma_contract_addr = info.sender;

        market.ma_token_address = ma_contract_addr.clone();
        MARKETS.save(deps.storage, reference.as_slice(), &market)?;

        // save ma token contract to reference mapping
        MARKET_MA_TOKENS.save(deps.storage, &ma_contract_addr, &reference);

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
    info: MessageInfo,
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
        USERS.save(deps.storage, &depositor_address, &user);
    }

    market_apply_accumulated_interests(&env, &mut market);
    market_update_interest_rates(&deps, &env, asset_reference, &mut market, Uint256::zero())?;
    MARKETS.save(deps.storage, asset_reference, &market)?;

    if market.liquidity_index.is_zero() {
        return Err(StdError::generic_err("Cannot have 0 as liquidity index").into());
    }
    // FIXME: timestamp or u64?
    let mint_amount =
        deposit_amount / get_updated_liquidity_index(&market, env.block.time.seconds());

    let mut log = vec![
        attr("action", "deposit"),
        attr("market", asset_label),
        attr("user", depositor_address.as_str()),
        attr("amount", deposit_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &market);

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
    let borrower_address = info.sender.clone();

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(&deps, &asset)?;

    // Cannot borrow zero amount
    if borrow_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Borrow amount must be greater than 0 {}",
            asset_label,
        ))
        .into());
    }

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
            &borrower_canonical_addr,
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
        &deps,
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
pub fn handle_repay<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    repayer_address: HumanAddr,
    asset_reference: &[u8],
    asset_label: &str,
    repay_amount: Uint256,
    asset_type: AssetType,
) -> StdResult<HandleResponse> {
    // TODO: assumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    let mut market = markets_state_read(&deps.storage).load(asset_reference)?;

    // Get repay amount
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    // Cannot repay zero amount
    if repay_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Repay amount must be greater than 0 {}",
            asset_label,
        )));
    }

    let repayer_canonical_address = deps.api.canonical_address(&repayer_address)?;

    // Check new debt
    let debts_asset_bucket = debts_asset_state_read(&deps.storage, asset_reference);
    let mut debt = debts_asset_bucket.load(repayer_canonical_address.as_slice())?;

    if debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("Cannot repay 0 debt"));
    }

    market_apply_accumulated_interests(&env, &mut market);

    let mut repay_amount_scaled = repay_amount / get_updated_borrow_index(&market, env.block.time);

    let mut messages: Vec<CosmosMsg> = vec![];
    let mut refund_amount = Uint256::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        // TODO: Should we log this?
        refund_amount = (repay_amount_scaled - debt.amount_scaled)
            * get_updated_borrow_index(&market, env.block.time);
        let refund_msg = match asset_type {
            AssetType::Native => build_send_native_asset_msg(
                deps,
                env.contract.address.clone(),
                repayer_address.clone(),
                asset_label,
                refund_amount,
            )?,
            AssetType::Cw20 => {
                let token_contract_addr = HumanAddr::from(asset_label);
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

    debt.amount_scaled = debt.amount_scaled - repay_amount_scaled;
    let mut debts_asset_bucket = debts_asset_state(&mut deps.storage, asset_reference);
    debts_asset_bucket.save(repayer_canonical_address.as_slice(), &debt)?;

    if repay_amount_scaled > market.debt_total_scaled {
        return Err(StdError::generic_err(
            "Amount to repay is greater than total debt",
        ));
    }
    market.debt_total_scaled = market.debt_total_scaled - repay_amount_scaled;
    market_update_interest_rates(&deps, &env, asset_reference, &mut market, Uint256::zero())?;
    markets_state(&mut deps.storage).save(asset_reference, &market)?;

    if debt.amount_scaled == Uint256::zero() {
        // Remove asset from borrowed assets
        let mut users_bucket = users_state(&mut deps.storage);
        let mut user = users_bucket.load(repayer_canonical_address.as_slice())?;
        unset_bit(&mut user.borrowed_assets, market.index)?;
        users_bucket.save(repayer_canonical_address.as_slice(), &user)?;
    }

    let mut log = vec![
        log("action", "repay"),
        log("market", asset_label),
        log("user", repayer_address),
        log("amount", repay_amount - refund_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &market);

    Ok(HandleResponse {
        data: None,
        log,
        messages,
    })
}

/// Handle loan liquidations on under-collateralized loans
pub fn handle_liquidate<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    liquidator_address: HumanAddr,
    collateral_asset: Asset,
    debt_asset: Asset,
    user_address: HumanAddr,
    sent_debt_asset_amount: Uint256,
    receive_ma_token: bool,
) -> StdResult<HandleResponse> {
    let block_time = env.block.time;

    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let (debt_asset_label, debt_asset_reference, _) = asset_get_attributes(deps, &debt_asset)?;

    // 1. Validate liquidation
    // If user (contract) has a positive uncollateralized limit then the user
    // cannot be liquidated
    let uncollateralized_loan_limits_bucket =
        uncollateralized_loan_limits_read(&deps.storage, debt_asset_reference.as_slice());
    let uncollateralized_loan_limit =
        match uncollateralized_loan_limits_bucket.may_load(user_canonical_address.as_slice()) {
            Ok(Some(limit)) => limit,
            Ok(None) => Uint128::zero(),
            Err(error) => return Err(error),
        };
    if uncollateralized_loan_limit > Uint128::zero() {
        return Err(StdError::generic_err(
            "user has a positive uncollateralized loan limit and thus cannot be liquidated",
        ));
    }

    // liquidator must send positive amount of funds in the debt asset
    if sent_debt_asset_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Must send more than 0 {} in order to liquidate",
            debt_asset_label,
        )));
    }

    let (collateral_asset_label, collateral_asset_reference, _) =
        asset_get_attributes(deps, &collateral_asset)?;

    let mut collateral_market =
        markets_state_read(&deps.storage).load(collateral_asset_reference.as_slice())?;

    // check if user has available collateral in specified collateral asset to be liquidated
    let collateral_ma_address = deps
        .api
        .human_address(&collateral_market.ma_token_address)?;
    let user_collateral_balance = get_updated_liquidity_index(&collateral_market, block_time)
        * Uint256::from(cw20_get_balance(
            &deps.querier,
            collateral_ma_address.clone(),
            user_address.clone(),
        )?);
    if user_collateral_balance == Uint256::zero() {
        return Err(StdError::generic_err(
            "user has no balance in specified collateral asset to be liquidated",
        ));
    }

    // check if user has outstanding debt in the deposited asset that needs to be repayed
    let debts_asset_bucket = debts_asset_state_read(&deps.storage, debt_asset_reference.as_slice());
    let user_debt = debts_asset_bucket.load(user_canonical_address.as_slice())?;
    if user_debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("User has no outstanding debt in the specified debt asset and thus cannot be liquidated"));
    }

    // 2. Compute health factor
    let config = config_state_read(&deps.storage).load()?;
    let money_market = money_market_state_read(&deps.storage).load()?;
    let user = users_state_read(&deps.storage).load(user_canonical_address.as_slice())?;
    let (user_account_settlement, native_asset_prices) = prepare_user_account_settlement(
        &deps,
        block_time,
        &user_canonical_address,
        &user,
        &money_market,
        &mut vec![],
    )?;

    let health_factor = match user_account_settlement.health_status {
        // NOTE: Should not get in practice as it would fail on the debt asset check
        UserHealthStatus::NotBorrowing => {
            return Err(StdError::generic_err(
                "User has no outstanding debt and thus cannot be liquidated",
            ))
        }
        UserHealthStatus::Borrowing(hf) => hf,
    };

    // if health factor is not less than one the user cannot be liquidated
    if health_factor >= Decimal256::one() {
        return Err(StdError::generic_err(
            "User's health factor is not less than 1 and thus cannot be liquidated",
        ));
    }

    let mut debt_market =
        markets_state_read(&deps.storage).load(debt_asset_reference.as_slice())?;

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

    let mut messages: Vec<CosmosMsg> = vec![];
    // 4. Update collateral positions and market depending on whether the liquidator elects to
    // receive ma_tokens or the underlying asset
    if receive_ma_token {
        // Transfer ma tokens from user to liquidator
        let liquidator_canonical_addr = deps.api.canonical_address(&liquidator_address)?;
        let mut liquidator: User = users_state_read(&deps.storage)
            .may_load(liquidator_canonical_addr.as_slice())?
            .unwrap_or_default();

        // set liquidator's deposited bit to true if not already true
        // NOTE: previous checks should ensure this amount is not zero
        let liquidator_is_using_as_collateral =
            get_bit(liquidator.collateral_assets, collateral_market.index)?;
        if !liquidator_is_using_as_collateral {
            set_bit(&mut liquidator.collateral_assets, collateral_market.index)?;
            users_state(&mut deps.storage)
                .save(liquidator_canonical_addr.as_slice(), &liquidator)?;
        }

        let collateral_amount_to_liquidate_scaled = collateral_amount_to_liquidate
            / get_updated_liquidity_index(&collateral_market, block_time);

        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: collateral_ma_address,
            msg: to_binary(&mars::ma_token::msg::HandleMsg::TransferOnLiquidation {
                sender: user_address.clone(),
                recipient: liquidator_address.clone(),
                amount: collateral_amount_to_liquidate_scaled.into(),
            })?,
            send: vec![],
        }))
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
            } => Uint256::from(cw20_get_balance(
                &deps.querier,
                token_addr,
                env.contract.address.clone(),
            )?),
        };
        let contract_collateral_balance = contract_collateral_balance
            * get_updated_liquidity_index(&collateral_market, block_time);
        if contract_collateral_balance < collateral_amount_to_liquidate {
            return Err(StdError::generic_err(
                "contract does not have enough collateral liquidity to send back underlying asset",
            ));
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
            contract_addr: collateral_ma_address,
            msg: to_binary(&mars::ma_token::msg::HandleMsg::Burn {
                user: user_address.clone(),
                amount: collateral_amount_to_liquidate_scaled.into(),
            })?,
            send: vec![],
        });

        let send_underlying_asset_msg = build_send_asset_msg(
            deps,
            env.contract.address.clone(),
            liquidator_address.clone(),
            collateral_asset,
            collateral_amount_to_liquidate,
        )?;
        messages.push(burn_ma_tokens_msg);
        messages.push(send_underlying_asset_msg);
    }

    // if max collateral to liquidate equals the user's balance then unset collateral bit
    if collateral_amount_to_liquidate == user_collateral_balance {
        let mut user = users_state_read(&deps.storage).load(user_canonical_address.as_slice())?;
        unset_bit(&mut user.collateral_assets, collateral_market.index)?;
        users_state(&mut deps.storage).save(user_canonical_address.as_slice(), &user)?;
    }

    // 5. Update debt market and positions

    let debt_amount_to_repay_scaled =
        debt_amount_to_repay / get_updated_borrow_index(&debt_market, block_time);

    // update user and market debt
    let mut debts_asset_bucket =
        debts_asset_state(&mut deps.storage, debt_asset_reference.as_slice());
    let mut debt = debts_asset_bucket.load(user_canonical_address.as_slice())?;
    // NOTE: Should be > 0 as amount to repay is capped by the close factor
    debt.amount_scaled = debt.amount_scaled - debt_amount_to_repay_scaled;
    debts_asset_bucket.save(user_canonical_address.as_slice(), &debt)?;
    debt_market.debt_total_scaled = debt_market.debt_total_scaled - debt_amount_to_repay_scaled;

    market_update_interest_rates(
        deps,
        &env,
        debt_asset_reference.as_slice(),
        &mut debt_market,
        refund_amount,
    )?;

    // save markets
    markets_state(&mut deps.storage).save(&debt_asset_reference.as_slice(), &debt_market)?;
    markets_state(&mut deps.storage)
        .save(&collateral_asset_reference.as_slice(), &collateral_market)?;

    // 6. Build response
    // refund sent amount in excess of actual debt amount to liquidate
    if refund_amount > Uint256::zero() {
        let refund_msg = build_send_asset_msg(
            deps,
            env.contract.address,
            liquidator_address.clone(),
            debt_asset,
            refund_amount,
        )?;
        messages.push(refund_msg);
    }

    let mut log = vec![
        log("action", "liquidate"),
        log("collateral_market", collateral_asset_label),
        log("debt_market", debt_asset_label),
        log("user", user_address.as_str()),
        log("liquidator", liquidator_address.as_str()),
        log(
            "collateral_amount_liquidated",
            collateral_amount_to_liquidate,
        ),
        log("debt_amount_repaid", debt_amount_to_repay),
        log("refund_amount", refund_amount),
    ];

    // TODO: we should distinguish between collateral and market values in some way
    append_indices_and_rates_to_logs(&mut log, &debt_market);
    if !receive_ma_token {
        append_indices_and_rates_to_logs(&mut log, &collateral_market);
    }

    Ok(HandleResponse {
        data: None,
        log,
        messages,
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
    let money_market = RED_BANK.load(&deps.storage)?;
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
        if (from_previous_balance - amount)? == Uint128::zero() {
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
    env: Env,
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

    let (asset_label, asset_reference, _) = asset_get_attributes(&deps, &asset)?;

    UNCOLLATERALIZED_LOAN_LIMITS.save(
        deps.storage,
        (asset_reference.as_slice(), &user_address),
        &new_limit,
    )?;

    // FIXME: use update?
    let mut debt = DEBTS
        .may_load(deps.storage, (asset_reference.as_slice(), &user_address))?
        .unwrap_or(Debt {
            amount_scaled: Uint256::zero(),
            uncollateralized: false,
        });
    // if limit == 0 then uncollateralized = false, otherwise uncollateralized = true
    debt.uncollateralized = !new_limit.is_zero();
    DEBTS.save(
        deps.storage,
        (asset_reference.as_slice(), &user_address),
        &debt,
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
    env: Env,
    info: MessageInfo,
    asset: Asset,
    enable: bool,
) -> Result<Response, ContractError> {
    let user_address = info.sender;
    let mut user = USERS
        .may_load(deps.storage, &user_address)?
        .unwrap_or_default();

    let (collateral_asset_label, collateral_asset_reference, _) =
        asset_get_attributes(&deps, &asset)?;
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
pub fn handle_distribute_protocol_income<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset: Asset,
    amount: Option<Uint256>,
) -> StdResult<HandleResponse> {
    // Get config
    let config = config_state_read(&deps.storage).load()?;

    let (asset_label, asset_reference, _) = asset_get_attributes(deps, &asset)?;
    let mut market = markets_state_read(&deps.storage).load(asset_reference.as_slice())?;

    let amount_to_distribute = match amount {
        Some(amount) => amount,
        None => market.protocol_income_to_distribute,
    };

    if amount_to_distribute > market.protocol_income_to_distribute {
        return Err(StdError::generic_err(
            "amount specified exceeds market's income to be distributed",
        ));
    }

    market.protocol_income_to_distribute =
        market.protocol_income_to_distribute - amount_to_distribute;
    markets_state(&mut deps.storage).save(&asset_reference.as_slice(), &market)?;

    let mut messages = vec![];

    let mars_contracts = vec![
        MarsContract::InsuranceFund,
        MarsContract::Staking,
        MarsContract::Treasury,
    ];
    let expected_len = mars_contracts.len();
    let mut addresses_query = address_provider::helpers::query_addresses(
        &deps,
        &config.address_provider_address,
        mars_contracts,
    )?;
    if addresses_query.len() != expected_len {
        return Err(StdError::generic_err(format!(
            "Incorrect number of addresses, expected {} got {}",
            expected_len,
            addresses_query.len()
        )));
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
        )));
    }
    let staking_amount = amount_to_distribute - (amount_to_distribute_before_staking_rewards);

    // only build and add send message if fee is non-zero
    if !insurance_fund_amount.is_zero() {
        let insurance_fund_msg = build_send_asset_msg(
            deps,
            env.contract.address.clone(),
            insurance_fund_address,
            asset.clone(),
            insurance_fund_amount,
        )?;
        messages.push(insurance_fund_msg);
    }

    if !treasury_amount.is_zero() {
        let scaled_mint_amount =
            treasury_amount / get_updated_liquidity_index(&market, env.block.time);
        let treasury_fund_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&market.ma_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: treasury_address,
                amount: scaled_mint_amount.into(),
            })?,
        });
        messages.push(treasury_fund_msg);
    }

    if !staking_amount.is_zero() {
        let staking_msg = build_send_asset_msg(
            deps,
            env.contract.address,
            staking_address,
            asset,
            staking_amount,
        )?;
        messages.push(staking_msg);
    }

    Ok(HandleResponse {
        messages,
        log: vec![
            log("action", "distribute_protocol_income"),
            log("asset", asset_label),
            log("amount", amount_to_distribute),
        ],
        data: None,
    })
}

// QUERIES

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Market { asset } => to_binary(&query_market(deps, asset)?),
        QueryMsg::MarketsList {} => to_binary(&query_markets_list(deps)?),
        QueryMsg::Debt { address } => to_binary(&query_debt(deps, address)?),
        QueryMsg::Collateral { address } => to_binary(&query_collateral(deps, address)?),
        QueryMsg::UncollateralizedLoanLimit {
            user_address,
            asset,
        } => to_binary(&query_uncollateralized_loan_limit(
            deps,
            user_address,
            asset,
        )?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    let money_market = money_market_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        address_provider_address: deps.api.human_address(&config.address_provider_address)?,
        insurance_fund_fee_share: config.insurance_fund_fee_share,
        treasury_fee_share: config.treasury_fee_share,
        ma_token_code_id: config.ma_token_code_id,
        market_count: money_market.market_count,
        close_factor: config.close_factor,
    })
}

fn query_market<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: Asset,
) -> StdResult<MarketResponse> {
    let market = match asset {
        Asset::Native { denom } => match markets_state_read(&deps.storage).load(denom.as_bytes()) {
            Ok(market) => market,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "failed to load market for: {}",
                    denom
                )))
            }
        },
        Asset::Cw20 { contract_addr } => {
            let cw20_canonical_address = deps.api.canonical_address(&contract_addr)?;
            match markets_state_read(&deps.storage).load(cw20_canonical_address.as_slice()) {
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
        ma_token_address: deps.api.human_address(&market.ma_token_address)?,
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

fn query_markets_list<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<MarketsListResponse> {
    let markets = markets_state_read(&deps.storage);

    let markets_list: StdResult<Vec<_>> = markets
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = get_market_denom(deps, k, v.asset_type)?;

            Ok(MarketInfo {
                denom,
                ma_token_address: deps.api.human_address(&v.ma_token_address)?,
            })
        })
        .collect();

    Ok(MarketsListResponse {
        markets_list: markets_list?,
    })
}

fn query_debt<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<DebtResponse> {
    let markets = markets_state_read(&deps.storage);
    let debtor_address = deps.api.canonical_address(&address)?;
    let user = users_state_read(&deps.storage)
        .may_load(debtor_address.as_slice())?
        .unwrap_or_default();

    let debts: StdResult<Vec<_>> = markets
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = get_market_denom(deps, k.clone(), v.asset_type)?;

            let is_borrowing_asset = get_bit(user.borrowed_assets, v.index)?;
            if is_borrowing_asset {
                let debts_asset_bucket = debts_asset_state_read(&deps.storage, k.as_slice());
                let debt = debts_asset_bucket.load(debtor_address.as_slice())?;
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

fn query_collateral<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<CollateralResponse> {
    let markets = markets_state_read(&deps.storage);
    let canonical_address = deps.api.canonical_address(&address)?;
    let user = users_state_read(&deps.storage)
        .may_load(canonical_address.as_slice())?
        .unwrap_or_default();

    let collateral: StdResult<Vec<_>> = markets
        .range(None, None, Order::Ascending)
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

fn query_uncollateralized_loan_limit<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    user_address: HumanAddr,
    asset: Asset,
) -> StdResult<UncollateralizedLoanLimitResponse> {
    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let (asset_label, asset_reference, _) = asset_get_attributes(deps, &asset)?;
    let uncollateralized_loan_limit =
        uncollateralized_loan_limits_read(&deps.storage, asset_reference.as_slice())
            .load(&user_canonical_address.as_slice());

    match uncollateralized_loan_limit {
        Ok(limit) => Ok(UncollateralizedLoanLimitResponse { limit }),
        Err(_) => Err(StdError::not_found(format!(
            "No uncollateralized loan approved for user_address: {} on asset: {}",
            user_address, asset_label
        ))),
    }
}

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// INTEREST

/// Updates market indices and protocol_income by applying current interest rates on the time between
/// last interest update and current block.
/// Note it does not save the market to the store (that is left to the caller)
pub fn market_apply_accumulated_interests(env: &Env, market: &mut Market) {
    let current_timestamp = env.block.time;
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
            let cw20_human_addr = deps.api.human_address(&CanonicalAddr::from(reference))?;
            cw20_get_balance(&deps.querier, cw20_human_addr, env.contract.address.clone())?
        }
    };

    // TODO: Verify on integration tests that this balance includes the
    // amount sent by the user on deposits and repays(both for cw20 and native).
    // If it doesn't, we should include them on the available_liquidity
    let contract_current_balance = Uint256::from(contract_balance_amount);

    // Get protocol income to be deducted from liquidity (doesn't belong to the money market
    // anymore)
    let config = config_state_read(&deps.storage).load()?;
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
        * get_updated_borrow_index(&market, env.block.time);
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
fn user_get_balances<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
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

        let (asset_reference_vec, market) = market_get_from_index(&deps.storage, i)?;

        let (collateral_amount, ltv, maintenance_margin) = if user_is_using_as_collateral {
            // query asset balance (ma_token contract gives back a scaled value)
            let asset_balance = cw20_get_balance(
                &deps.querier,
                deps.api.human_address(&market.ma_token_address)?,
                deps.api.human_address(user_address)?,
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
            let debts_asset_bucket = debts_asset_state_read(&deps.storage, &asset_reference_vec);
            let user_debt: Debt = debts_asset_bucket.load(user_address.as_slice())?;

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
            AssetType::Cw20 => String::from(
                deps.api
                    .human_address(&CanonicalAddr::from(asset_reference_vec.as_slice()))?
                    .as_str(),
            ),
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
        &deps,
        &money_market,
        user,
        user_address,
        native_asset_prices_to_query,
        block_time,
    )?;
    let native_asset_prices =
        get_native_asset_prices(&deps.querier, &native_asset_prices_to_query)?;

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

fn get_market_denom<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    market_id: Vec<u8>,
    asset_type: AssetType,
) -> StdResult<String> {
    match asset_type {
        AssetType::Native => match String::from_utf8(market_id) {
            Ok(denom) => Ok(denom),
            Err(_) => Err(StdError::generic_err("failed to encode key into string")),
        },
        AssetType::Cw20 => {
            let cw20_contract_address =
                match deps.api.human_address(&CanonicalAddr::from(market_id)) {
                    Ok(cw20_contract_address) => cw20_contract_address,
                    Err(_) => {
                        return Err(StdError::generic_err(
                            "failed to encode key into contract address",
                        ))
                    }
                };

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
    *bitmap = Uint128(bitmap.u128() | (1 << index));
    Ok(())
}

/// Sets bit to 0
fn unset_bit(bitmap: &mut Uint128, index: u32) -> StdResult<()> {
    if index >= 128 {
        return Err(StdError::generic_err("index out of range"));
    }
    *bitmap = Uint128(bitmap.u128() & !(1 << index));
    Ok(())
}

fn build_send_asset_msg(
    deps: &DepsMut,
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
fn build_send_native_asset_msg<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    sender: Addr,
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
        msg: to_binary(&Cw20HandleMsg::Transfer {
            recipient,
            amount: amount.into(),
        })?,
        funds: vec![],
    }))
}

fn get_native_asset_prices<Q: Querier>(
    querier: &Q,
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

// FIXME: canonical address
fn asset_get_attributes(deps: &DepsMut, asset: &Asset) -> StdResult<(String, Vec<u8>, AssetType)> {
    match asset {
        Asset::Native { denom } => {
            let asset_label = denom.as_bytes().to_vec();
            Ok((denom.to_string(), asset_label, AssetType::Native))
        }
        Asset::Cw20 { contract_addr } => {
            let asset_label = String::from(contract_addr.as_str());
            let asset_reference = deps
                .api
                .canonical_address(&contract_addr)?
                .as_slice()
                .to_vec();
            Ok((asset_label, asset_reference, AssetType::Cw20))
        }
    }
}

fn market_get_from_index<S: Storage>(storage: &S, index: u32) -> StdResult<(Vec<u8>, Market)> {
    let asset_reference_vec =
        match market_references_state_read(storage).load(&index.to_be_bytes()) {
            Ok(asset_reference_vec) => asset_reference_vec,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "no market reference exists with index: {}",
                    index
                )))
            }
        }
        .reference;
    match markets_state_read(storage).load(&asset_reference_vec) {
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
    use crate::state::{debts_asset_state_read, users_state_read, PidParameters};
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{coin, from_binary, Decimal, Extern};
    use mars::red_bank::msg::HandleMsg::UpdateConfig;
    use mars::testing::{
        assert_generic_error_message, mock_dependencies, MarsMockQuerier, MockEnvParams,
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
            ma_token_address: Default::default(),
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
        let mut deps = mock_dependencies(20, &[]);

        // Config with base params valid (just update the rest)
        let base_config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("owner")),
            address_provider_address: Some(HumanAddr::from("address_provider")),
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
        let msg = InitMsg {
            config: empty_config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = init(&mut deps, env, msg);
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
        let msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = init(&mut deps, env, msg);
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
        let exceeding_fees_msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = init(&mut deps, env.clone(), exceeding_fees_msg);
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
        let msg = InitMsg { config };

        // we can just call .unwrap() to assert this was a success
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(&deps, QueryMsg::Config {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(10, value.ma_token_code_id);
        assert_eq!(0, value.market_count);
        assert_eq!(insurance_fund_fee_share, value.insurance_fund_fee_share);
        assert_eq!(treasury_fee_share, value.treasury_fee_share);
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with valid params
        // *
        let mut insurance_fund_fee_share = Decimal256::from_ratio(1, 10);
        let mut treasury_fee_share = Decimal256::from_ratio(3, 10);
        let mut close_factor = Decimal256::from_ratio(1, 4);
        let init_config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("owner")),
            address_provider_address: Some(HumanAddr::from("address_provider")),
            ma_token_code_id: Some(20u64),
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
        };
        let msg = InitMsg {
            config: init_config.clone(),
        };
        // we can just call .unwrap() to assert this was a success
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let msg = UpdateConfig {
            config: init_config.clone(),
        };
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

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
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "[close_factor, insurance_fund_fee_share, treasury_fee_share] should be less or equal 1. \
                Invalid params: [close_factor, insurance_fund_fee_share, treasury_fee_share]");

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
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = handle(&mut deps, env.clone(), exceeding_fees_msg);
        assert_generic_error_message(
            response,
            "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one",
        );

        // *
        // update config with all new params
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(5, 100);
        treasury_fee_share = Decimal256::from_ratio(3, 100);
        close_factor = Decimal256::from_ratio(1, 20);
        let config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("new_owner")),
            address_provider_address: Some(HumanAddr::from("new_address_provider")),
            ma_token_code_id: Some(40u64),
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
        };
        let msg = UpdateConfig {
            config: config.clone(),
        };

        // we can just call .unwrap() to assert this was a success
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = config_state_read(&deps.storage).load().unwrap();

        assert_eq!(
            new_config.owner,
            deps.api
                .canonical_address(&HumanAddr::from("new_owner"))
                .unwrap()
        );
        assert_eq!(
            new_config.address_provider_address,
            deps.api
                .canonical_address(&config.address_provider_address.unwrap())
                .unwrap()
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
        let mut deps = mock_dependencies(20, &[]);

        let config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("owner")),
            address_provider_address: Some(HumanAddr::from("address_provider")),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
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
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // init asset with empty params
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let empty_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: None,
            maintenance_margin: None,
            liquidation_bonus: None,
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "All params should be available during initialization",
        );

        // *
        // init asset with some params greater than 1
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(110, 10)),
            reserve_factor: Some(Decimal256::from_ratio(120, 100)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "[max_loan_to_value, reserve_factor, maintenance_margin, liquidation_bonus] should be less or equal 1. \
                Invalid params: [max_loan_to_value, reserve_factor]");

        // *
        // init asset where LTV >= liquidity threshold
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(5, 10)),
            maintenance_margin: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "maintenance_margin should be greater than max_loan_to_value. \
                    maintenance_margin: 0.5, \
                    max_loan_to_value: 0.5",
        );

        // *
        // init asset where min borrow rate >= max borrow rate
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            min_borrow_rate: Some(Decimal256::from_ratio(5, 10)),
            max_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "max_borrow_rate should be greater than min_borrow_rate. max_borrow_rate: 0.4, min_borrow_rate: 0.5",
        );

        // *
        // init asset where optimal utilization rate > 1
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            optimal_utilization_rate: Some(Decimal256::from_ratio(11, 10)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "Optimal utilization rate can't be greater than one",
        );

        // *
        // owner is authorized
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // should have asset market with Canonical default address
        let market = markets_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(CanonicalAddr::default(), market.ma_token_address);
        // should have 0 index
        assert_eq!(0, market.index);
        // should have asset_type Native
        assert_eq!(AssetType::Native, market.asset_type);

        // should store reference in market index
        let market_reference = market_references_state_read(&deps.storage)
            .load(&0_u32.to_be_bytes())
            .unwrap();
        assert_eq!(b"someasset", market_reference.reference.as_slice());

        // Should have market count of 1
        let money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(money_market.market_count, 1);

        // should instantiate a liquidity token
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 5u64,
                msg: to_binary(&ma_token::msg::InitMsg {
                    name: String::from("mars someasset liquidity token"),
                    symbol: String::from("masomeasset"),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        cap: None,
                    }),
                    init_hook: Some(ma_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitAssetTokenCallback {
                            reference: "someasset".into(),
                        })
                        .unwrap(),
                        contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    }),
                    red_bank_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    incentives_address: HumanAddr::from("incentives"),
                })
                .unwrap(),
                send: vec![],
                label: None,
            }),]
        );

        assert_eq!(
            res.log,
            vec![log("action", "init_asset"), log("asset", "someasset"),],
        );

        // *
        // can't init more than once
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Asset already initialized");

        // *
        // callback comes back with created token
        // *
        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: "someasset".into(),
        };
        handle(&mut deps, env, msg).unwrap();

        // should have asset market with contract address
        let market = markets_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mtokencontract"))
                .unwrap(),
            market.ma_token_address
        );
        assert_eq!(Decimal256::one(), market.liquidity_index);

        // *
        // calling this again should not be allowed
        // *
        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: "someasset".into(),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // Initialize a cw20 asset
        // *
        let cw20_addr = HumanAddr::from("otherasset");
        deps.querier
            .set_cw20_symbol(cw20_addr.clone(), "otherasset".to_string());
        let env = cosmwasm_std::testing::mock_env("owner", &[]);

        let msg = HandleMsg::InitAsset {
            asset: Asset::Cw20 {
                contract_addr: cw20_addr.clone(),
            },
            asset_params,
        };
        let res = handle(&mut deps, env, msg).unwrap();
        let cw20_canonical_addr = deps.api.canonical_address(&cw20_addr).unwrap();

        let market = markets_state_read(&deps.storage)
            .load(&cw20_canonical_addr.as_slice())
            .unwrap();
        // should have asset market with Canonical default address
        assert_eq!(CanonicalAddr::default(), market.ma_token_address);
        // should have index 1
        assert_eq!(1, market.index);
        // should have asset_type Cw20
        assert_eq!(AssetType::Cw20, market.asset_type);

        // should store reference in market index
        let market_reference = market_references_state_read(&deps.storage)
            .load(&1_u32.to_be_bytes())
            .unwrap();
        assert_eq!(
            cw20_canonical_addr.as_slice(),
            market_reference.reference.as_slice()
        );

        // should have an asset_type of cw20
        assert_eq!(AssetType::Cw20, market.asset_type);

        // Should have market count of 2
        let money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(2, money_market.market_count);

        assert_eq!(
            res.log,
            vec![log("action", "init_asset"), log("asset", cw20_addr)],
        );
        // *
        // cw20 callback comes back with created token
        // *
        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: Vec::from(cw20_canonical_addr.as_slice()),
        };
        handle(&mut deps, env, msg).unwrap();

        // should have asset market with contract address
        let market = markets_state_read(&deps.storage)
            .load(cw20_canonical_addr.as_slice())
            .unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mtokencontract"))
                .unwrap(),
            market.ma_token_address
        );
        assert_eq!(Decimal256::one(), market.liquidity_index);

        // *
        // calling this again should not be allowed
        // *
        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: Vec::from(cw20_canonical_addr.as_slice()),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());
    }

    #[test]
    fn test_update_asset() {
        let mut deps = mock_dependencies(20, &[]);

        let config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("owner")),
            address_provider_address: Some(HumanAddr::from("address_provider")),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(5u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
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
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // owner is authorized but can't update asset if not initialize firstly
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Asset not initialized");

        // *
        // initialize asset
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        // *
        // update asset with some params greater than 1
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            maintenance_margin: Some(Decimal256::from_ratio(110, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "[max_loan_to_value, reserve_factor, maintenance_margin, liquidation_bonus] should be less or equal 1. \
                Invalid params: [maintenance_margin]");

        // *
        // update asset where LTV >= liquidity threshold
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            max_loan_to_value: Some(Decimal256::from_ratio(6, 10)),
            maintenance_margin: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "maintenance_margin should be greater than max_loan_to_value. \
                    maintenance_margin: 0.5, \
                    max_loan_to_value: 0.6",
        );

        // *
        // init asset where min borrow rate >= max borrow rate
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            min_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            max_borrow_rate: Some(Decimal256::from_ratio(4, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "max_borrow_rate should be greater than min_borrow_rate. max_borrow_rate: 0.4, min_borrow_rate: 0.4",
        );

        // *
        // init asset where optimal utilization rate > 1
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            optimal_utilization_rate: Some(Decimal256::from_ratio(11, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "Optimal utilization rate can't be greater than one",
        );

        // *
        // update asset with new params
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
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
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let new_market = markets_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
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

        let new_market_reference = market_references_state_read(&deps.storage)
            .load(&0_u32.to_be_bytes())
            .unwrap();
        assert_eq!(b"someasset", new_market_reference.reference.as_slice());

        let new_money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(new_money_market.market_count, 1);

        assert_eq!(res.messages, vec![],);

        assert_eq!(
            res.log,
            vec![log("action", "update_asset"), log("asset", "someasset"),],
        );

        // *
        // update asset with empty params
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
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
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let new_market = markets_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
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

        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: "uluna".into(),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::not_found("red_bank::state::Market"));
    }

    #[test]
    fn test_deposit_native_asset() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "somecoin")]);
        let reserve_factor = Decimal256::from_ratio(1, 10);

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
        let market = th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);

        let deposit_amount = 110000;
        let env = mars::testing::mock_env(
            "depositor",
            MockEnvParams {
                sent_funds: &[coin(deposit_amount, "somecoin")],
                block_time: 10000100,
                ..Default::default()
            },
        );
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let res = handle(&mut deps, env.clone(), msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &market,
            env.block.time,
            initial_liquidity,
            Default::default(),
        );

        let expected_mint_amount =
            (Uint256::from(deposit_amount) / expected_params.liquidity_index).into();

        // mints coin_amount/liquidity_index
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("matoken"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("depositor"),
                    amount: expected_mint_amount,
                })
                .unwrap(),
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "deposit"),
                log("market", "somecoin"),
                log("user", "depositor"),
                log("amount", deposit_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        let market = markets_state_read(&deps.storage).load(b"somecoin").unwrap();
        assert_eq!(market.borrow_rate, expected_params.borrow_rate);
        assert_eq!(market.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(market.liquidity_index, expected_params.liquidity_index);
        assert_eq!(market.borrow_index, expected_params.borrow_index);
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let env = cosmwasm_std::testing::mock_env("depositor", &[]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Deposit amount must be greater than 0 somecoin");
    }

    #[test]
    fn test_deposit_cw20() {
        let initial_liquidity = 10_000_000;
        let mut deps = th_setup(&[]);

        let cw20_addr = HumanAddr::from("somecontract");
        let contract_addr_raw = deps.api.canonical_address(&cw20_addr).unwrap();

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
        let market = th_init_market(
            &deps.api,
            &mut deps.storage,
            contract_addr_raw.as_slice(),
            &mock_market,
        );

        // set initial balance on cw20 contract
        deps.querier.set_cw20_balances(
            cw20_addr.clone(),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                initial_liquidity.into(),
            )],
        );
        // set symbol for cw20 contract
        deps.querier
            .set_cw20_symbol(cw20_addr.clone(), "somecoin".to_string());

        let deposit_amount = 110000u128;
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::DepositCw20 {}).unwrap()),
            sender: HumanAddr::from("depositor"),
            amount: Uint128(deposit_amount),
        });
        let env = mars::testing::mock_env(
            "somecontract",
            MockEnvParams {
                sent_funds: &[coin(deposit_amount, "somecoin")],
                block_time: 10000100,
                ..Default::default()
            },
        );

        let res = handle(&mut deps, env.clone(), msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &market,
            env.block.time,
            initial_liquidity,
            Default::default(),
        );

        let expected_mint_amount: Uint256 =
            Uint256::from(deposit_amount) / expected_params.liquidity_index;

        let market = markets_state_read(&deps.storage)
            .load(contract_addr_raw.as_slice())
            .unwrap();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("matoken"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("depositor"),
                    amount: expected_mint_amount.into(),
                })
                .unwrap(),
            })]
        );

        assert_eq!(
            res.log,
            vec![
                log("action", "deposit"),
                log("market", cw20_addr),
                log("user", "depositor"),
                log("amount", deposit_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );
        assert_eq!(
            market.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let env = cosmwasm_std::testing::mock_env("depositor", &[]);
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::DepositCw20 {}).unwrap()),
            sender: HumanAddr::from("depositor"),
            amount: Uint128(deposit_amount),
        });
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::not_found("red_bank::state::Market"));
    }

    #[test]
    fn test_cannot_deposit_if_no_market() {
        let mut deps = th_setup(&[]);

        let env = cosmwasm_std::testing::mock_env("depositer", &[coin(110000, "somecoin")]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::not_found("red_bank::state::Market"));
    }

    #[test]
    fn test_withdraw_native() {
        // Withdraw native token
        let initial_available_liquidity = 12000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128(100u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
            HumanAddr::from("matoken"),
            &[(HumanAddr::from("withdrawer"), Uint128(2000000u128))],
        );

        let market_initial =
            th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);
        market_ma_tokens_state(&mut deps.storage)
            .save(
                deps.api
                    .canonical_address(&HumanAddr::from("matoken"))
                    .unwrap()
                    .as_slice(),
                &(b"somecoin".to_vec()),
            )
            .unwrap();

        let withdrawer_addr = HumanAddr::from("withdrawer");
        let user = User::default();
        let mut users_bucket = users_state(&mut deps.storage);
        let withdrawer_canonical_addr = deps.api.canonical_address(&withdrawer_addr).unwrap();
        users_bucket
            .save(withdrawer_canonical_addr.as_slice(), &user)
            .unwrap();

        let msg = HandleMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(withdraw_amount),
        };

        let env = mars::testing::mock_env(
            "withdrawer",
            MockEnvParams {
                sent_funds: &[],
                block_time: mock_market.interests_last_updated + seconds_elapsed,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let market = markets_state_read(&deps.storage).load(b"somecoin").unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
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
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("matoken"),
                    send: vec![],
                    msg: to_binary(&ma_token::msg::HandleMsg::Burn {
                        user: withdrawer_addr.clone(),
                        amount: withdraw_amount_scaled.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: withdrawer_addr,
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: String::from("somecoin"),
                            amount: withdraw_amount.into(),
                        }
                    )
                    .unwrap()],
                }),
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "withdraw"),
                log("market", "somecoin"),
                log("user", "withdrawer"),
                log("burn_amount", withdraw_amount_scaled),
                log("withdraw_amount", withdraw_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
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
        let cw20_contract_addr = HumanAddr::from("somecontract");
        let cw20_contract_canonical_addr = deps.api.canonical_address(&cw20_contract_addr).unwrap();
        let initial_available_liquidity = 12000000u128;

        let ma_token_addr = HumanAddr::from("matoken");

        deps.querier.set_cw20_balances(
            cw20_contract_addr.clone(),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(initial_available_liquidity),
            )],
        );
        deps.querier.set_cw20_balances(
            ma_token_addr.clone(),
            &[(HumanAddr::from("withdrawer"), Uint128(2000000u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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

        let market_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            cw20_contract_canonical_addr.as_slice(),
            &mock_market,
        );
        market_ma_tokens_state(&mut deps.storage)
            .save(
                deps.api
                    .canonical_address(&ma_token_addr)
                    .unwrap()
                    .as_slice(),
                &cw20_contract_canonical_addr.as_slice().to_vec(),
            )
            .unwrap();

        let withdrawer_addr = HumanAddr::from("withdrawer");

        let user = User::default();
        let mut users_bucket = users_state(&mut deps.storage);
        let withdrawer_canonical_addr = deps.api.canonical_address(&withdrawer_addr).unwrap();
        users_bucket
            .save(withdrawer_canonical_addr.as_slice(), &user)
            .unwrap();

        let msg = HandleMsg::Withdraw {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.clone(),
            },
            amount: Some(withdraw_amount),
        };

        let env = mars::testing::mock_env(
            "withdrawer",
            MockEnvParams {
                sent_funds: &[],
                block_time: mock_market.interests_last_updated + seconds_elapsed,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let market = markets_state_read(&deps.storage)
            .load(cw20_contract_canonical_addr.as_slice())
            .unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
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
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: ma_token_addr,
                    send: vec![],
                    msg: to_binary(&ma_token::msg::HandleMsg::Burn {
                        user: withdrawer_addr.clone(),
                        amount: withdraw_amount_scaled.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: cw20_contract_addr,
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: withdrawer_addr,
                        amount: withdraw_amount.into(),
                    })
                    .unwrap(),
                    send: vec![],
                }),
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "withdraw"),
                log("market", "somecontract"),
                log("user", "withdrawer"),
                log("burn_amount", withdraw_amount_scaled),
                log("withdraw_amount", withdraw_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
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

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
            liquidity_index: Decimal256::from_ratio(15, 10),
            ..Default::default()
        };

        deps.querier.set_cw20_balances(
            HumanAddr::from("matoken"),
            &[(HumanAddr::from("withdrawer"), Uint128(200u128))],
        );

        th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);

        let msg = HandleMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(Uint256::from(2000u128)),
        };

        let env = cosmwasm_std::testing::mock_env("withdrawer", &[]);
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Withdraw amount must be greater than 0 and less or equal user balance (asset: somecoin)");
    }

    #[test]
    fn test_withdraw_if_health_factor_not_met() {
        let initial_available_liquidity = 10000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "token3")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("token3"), Uint128(100u128))],
        );

        let withdrawer_addr = HumanAddr::from("withdrawer");
        let withdrawer_canonical_addr = deps.api.canonical_address(&withdrawer_addr).unwrap();

        // Initialize markets
        let ma_token_1_addr = HumanAddr::from("matoken1");
        let market_1 = Market {
            ma_token_address: deps.api.canonical_address(&ma_token_1_addr).unwrap(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(40, 100),
            maintenance_margin: Decimal256::from_ratio(60, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_2_addr = HumanAddr::from("matoken2");
        let market_2 = Market {
            ma_token_address: deps.api.canonical_address(&ma_token_2_addr).unwrap(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(50, 100),
            maintenance_margin: Decimal256::from_ratio(80, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let ma_token_3_addr = HumanAddr::from("matoken3");
        let market_3 = Market {
            ma_token_address: deps.api.canonical_address(&ma_token_3_addr).unwrap(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            max_loan_to_value: Decimal256::from_ratio(20, 100),
            maintenance_margin: Decimal256::from_ratio(40, 100),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let market_1_initial = th_init_market(&deps.api, &mut deps.storage, b"token1", &market_1);
        let market_2_initial = th_init_market(&deps.api, &mut deps.storage, b"token2", &market_2);
        let market_3_initial = th_init_market(&deps.api, &mut deps.storage, b"token3", &market_3);

        // Initialize user with market_1 and market_3 as collaterals
        // User borrows market_2
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_3_initial.index).unwrap();
        set_bit(&mut user.borrowed_assets, market_2_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(withdrawer_canonical_addr.as_slice(), &user)
            .unwrap();

        // Set the querier to return collateral balances (ma_token_1 and ma_token_3)
        let ma_token_1_balance_scaled = Uint128(100000);
        deps.querier.set_cw20_balances(
            ma_token_1_addr,
            &[(withdrawer_addr.clone(), ma_token_1_balance_scaled)],
        );
        let ma_token_3_balance_scaled = Uint128(600000);
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
        debts_asset_state(&mut deps.storage, b"token2")
            .save(withdrawer_canonical_addr.as_slice(), &debt)
            .unwrap();
        debts_asset_state(&mut deps.storage, b"token3")
            .save(withdrawer_canonical_addr.as_slice(), &uncollateralized_debt)
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

        let env = cosmwasm_std::testing::mock_env("withdrawer", &[]);

        // Calculate how much to withdraw to have health factor equal to one
        let how_much_to_withdraw = {
            let token_1_weighted_lt_in_uusd = Uint256::from(ma_token_1_balance_scaled)
                * get_updated_liquidity_index(&market_1_initial, env.block.time)
                * market_1_initial.maintenance_margin
                * Decimal256::from(token_1_exchange_rate);
            let token_3_weighted_lt_in_uusd = Uint256::from(ma_token_3_balance_scaled)
                * get_updated_liquidity_index(&market_3_initial, env.block.time)
                * market_3_initial.maintenance_margin
                * Decimal256::from(token_3_exchange_rate);
            let weighted_maintenance_margin_in_uusd =
                token_1_weighted_lt_in_uusd + token_3_weighted_lt_in_uusd;

            let total_collateralized_debt_in_uusd = token_2_debt_scaled
                * get_updated_borrow_index(&market_2_initial, env.block.time)
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
            let msg = HandleMsg::Withdraw {
                asset: Asset::Native {
                    denom: "token3".to_string(),
                },
                amount: Some(withdraw_amount),
            };
            let response = handle(&mut deps, env.clone(), msg);
            assert_generic_error_message(
                response,
                "User's health factor can't be less than 1 after withdraw",
            );
        }

        // Withdraw token3 with success
        // The withdraw amount needs to be a little bit smaller to have health factor greater than one
        {
            let withdraw_amount = how_much_to_withdraw - Uint256::from(10u128);
            let msg = HandleMsg::Withdraw {
                asset: Asset::Native {
                    denom: "token3".to_string(),
                },
                amount: Some(withdraw_amount),
            };
            let res = handle(&mut deps, env.clone(), msg).unwrap();

            let withdraw_amount_scaled =
                withdraw_amount / get_updated_liquidity_index(&market_3_initial, env.block.time);

            assert_eq!(
                res.messages,
                vec![
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("matoken3"),
                        send: vec![],
                        msg: to_binary(&ma_token::msg::HandleMsg::Burn {
                            user: withdrawer_addr.clone(),
                            amount: withdraw_amount_scaled.into(),
                        })
                        .unwrap(),
                    }),
                    CosmosMsg::Bank(BankMsg::Send {
                        from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        to_address: withdrawer_addr,
                        amount: vec![deduct_tax(
                            &deps,
                            Coin {
                                denom: String::from("token3"),
                                amount: withdraw_amount.into(),
                            }
                        )
                        .unwrap()],
                    }),
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
            &[(String::from("somecoin"), Uint128(100u128))],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
            HumanAddr::from("matoken"),
            &[(
                HumanAddr::from("withdrawer"),
                withdrawer_balance_scaled.into(),
            )],
        );

        let market_initial =
            th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);
        market_ma_tokens_state(&mut deps.storage)
            .save(
                deps.api
                    .canonical_address(&HumanAddr::from("matoken"))
                    .unwrap()
                    .as_slice(),
                &(b"somecoin".to_vec()),
            )
            .unwrap();

        // Mark the market as collateral for the user
        let withdrawer_addr = HumanAddr::from("withdrawer");
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        let withdrawer_canonical_addr = deps.api.canonical_address(&withdrawer_addr).unwrap();
        users_bucket
            .save(withdrawer_canonical_addr.as_slice(), &user)
            .unwrap();
        // Check if user has set bit for collateral
        assert!(get_bit(user.collateral_assets, market_initial.index).unwrap());

        let msg = HandleMsg::Withdraw {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: None,
        };

        let env = mars::testing::mock_env(
            "withdrawer",
            MockEnvParams {
                sent_funds: &[],
                block_time: mock_market.interests_last_updated + seconds_elapsed,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let market = markets_state_read(&deps.storage).load(b"somecoin").unwrap();

        let withdrawer_balance = withdrawer_balance_scaled
            * get_updated_liquidity_index(
                &market_initial,
                market_initial.interests_last_updated + seconds_elapsed,
            );

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
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
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("matoken"),
                    send: vec![],
                    msg: to_binary(&ma_token::msg::HandleMsg::Burn {
                        user: withdrawer_addr.clone(),
                        amount: withdrawer_balance_scaled.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: withdrawer_addr,
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: String::from("somecoin"),
                            amount: withdrawer_balance.into(),
                        }
                    )
                    .unwrap()],
                }),
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "withdraw"),
                log("market", "somecoin"),
                log("user", "withdrawer"),
                log("burn_amount", withdrawer_balance_scaled),
                log("withdraw_amount", withdrawer_balance),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
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
        let user = users_state_read(&deps.storage)
            .load(&withdrawer_canonical_addr.as_slice())
            .unwrap();
        assert!(!get_bit(user.collateral_assets, market_initial.index).unwrap());
    }

    #[test]
    fn test_borrow_and_repay() {
        // NOTE: available liquidity stays fixed as the test environment does not get changes in
        // contract balances on subsequent calls. They would change from call to call in practice
        let available_liquidity_cw20 = 1000000000u128; // cw20
        let available_liquidity_native = 2000000000u128; // native
        let mut deps = th_setup(&[coin(available_liquidity_native, "borrowedcoinnative")]);

        let cw20_contract_addr = HumanAddr::from("borrowedcoincw20");
        let cw20_contract_addr_canonical = deps.api.canonical_address(&cw20_contract_addr).unwrap();
        deps.querier.set_cw20_balances(
            cw20_contract_addr.clone(),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(available_liquidity_cw20),
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
            &[(String::from("borrowedcoinnative"), Uint128(100u128))],
        );

        let mock_market_1 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken1"))
                .unwrap(),
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
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken2"))
                .unwrap(),
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken3"))
                .unwrap(),
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
        let market_1_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_market_1,
        );
        // should get index 1
        let market_2_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoinnative",
            &mock_market_2,
        );
        // should get index 2
        let market_collateral = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin",
            &mock_market_3,
        );

        let borrower_addr = HumanAddr::from("borrower");
        let borrower_canonical_addr = deps.api.canonical_address(&borrower_addr).unwrap();

        // Set user as having the market_collateral deposited
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, market_collateral.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(borrower_canonical_addr.as_slice(), &user)
            .unwrap();

        // Set the querier to return a certain collateral balance
        let deposit_coin_address = HumanAddr::from("matoken3");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(borrower_addr.clone(), Uint128(10000))],
        );

        // TODO: probably some variables (ie: borrow_amount, expected_params) that are repeated
        // in all calls could be enclosed in local scopes somehow)
        // *
        // Borrow cw20 token
        // *
        let block_time = mock_market_1.interests_last_updated + 10000u64;
        let borrow_amount = 2400u128;

        let msg = HandleMsg::Borrow {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.clone(),
            },
            amount: Uint256::from(borrow_amount),
        };

        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        let res = handle(&mut deps, env, msg).unwrap();

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps,
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
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_addr.clone(),
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: borrower_addr.clone(),
                    amount: borrow_amount.into(),
                })
                .unwrap(),
                send: vec![],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("market", "borrowedcoincw20"),
                log("user", "borrower"),
                log("amount", borrow_amount),
                log("borrow_index", expected_params_cw20.borrow_index),
                log("liquidity_index", expected_params_cw20.liquidity_index),
                log("borrow_rate", expected_params_cw20.borrow_rate),
                log("liquidity_rate", expected_params_cw20.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt = debts_asset_state_read(&deps.storage, cw20_contract_addr_canonical.as_slice())
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let expected_debt_scaled_1_after_borrow =
            Uint256::from(borrow_amount) / expected_params_cw20.borrow_index;

        let market_1_after_borrow = markets_state_read(&deps.storage)
            .load(cw20_contract_addr_canonical.as_slice())
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

        let msg = HandleMsg::Borrow {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.clone(),
            },
            amount: Uint256::from(borrow_amount),
        };

        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        handle(&mut deps, env, msg).unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps,
            &market_1_after_borrow,
            block_time,
            available_liquidity_cw20,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );
        let debt = debts_asset_state_read(&deps.storage, cw20_contract_addr_canonical.as_slice())
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let market_1_after_borrow_again = markets_state_read(&deps.storage)
            .load(cw20_contract_addr_canonical.as_slice())
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
        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: String::from("borrowedcoinnative"),
            },
            amount: Uint256::from(borrow_amount),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_native = th_get_expected_indices_and_rates(
            &deps,
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
            vec![CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: HumanAddr::from("borrower"),
                amount: vec![deduct_tax(
                    &deps,
                    Coin {
                        denom: String::from("borrowedcoinnative"),
                        amount: borrow_amount.into(),
                    }
                )
                .unwrap()],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("market", "borrowedcoinnative"),
                log("user", "borrower"),
                log("amount", borrow_amount),
                log("borrow_index", expected_params_native.borrow_index),
                log("liquidity_index", expected_params_native.liquidity_index),
                log("borrow_rate", expected_params_native.borrow_rate),
                log("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoinnative")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let market_2_after_borrow_2 = markets_state_read(&deps.storage)
            .load(b"borrowedcoinnative")
            .unwrap();

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

        let env = cosmwasm_std::testing::mock_env("borrower", &[]);
        let msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: String::from("borrowedcoinnative"),
            },
            amount: Uint256::from(83968_u128),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "borrow amount exceeds maximum allowed given current collateral value",
        );

        // *
        // Repay zero native debt(should fail)
        // *
        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "Repay amount must be greater than 0 borrowedcoinnative",
        );

        // *
        // Repay some native debt
        // *
        let repay_amount = 2000u128;
        let block_time = market_2_after_borrow_2.interests_last_updated + 8000u64;
        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[coin(repay_amount, "borrowedcoinnative")],
                block_time,
                ..Default::default()
            },
        );
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_params_native = th_get_expected_indices_and_rates(
            &deps,
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
            res.log,
            vec![
                log("action", "repay"),
                log("market", "borrowedcoinnative"),
                log("user", "borrower"),
                log("amount", repay_amount),
                log("borrow_index", expected_params_native.borrow_index),
                log("liquidity_index", expected_params_native.liquidity_index),
                log("borrow_rate", expected_params_native.borrow_rate),
                log("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoinnative")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let market_2_after_repay_some_2 = markets_state_read(&deps.storage)
            .load(b"borrowedcoinnative")
            .unwrap();
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
            &deps,
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

        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[coin(repay_amount, "borrowedcoinnative")],
                block_time,
                ..Default::default()
            },
        );
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(res.messages, vec![]);
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("market", "borrowedcoinnative"),
                log("user", "borrower"),
                log("amount", repay_amount),
                log("borrow_index", expected_params_native.borrow_index),
                log("liquidity_index", expected_params_native.liquidity_index),
                log("borrow_rate", expected_params_native.borrow_rate),
                log("liquidity_rate", expected_params_native.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoinnative")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let market_2_after_repay_all_2 = markets_state_read(&deps.storage)
            .load(b"borrowedcoinnative")
            .unwrap();

        assert_eq!(Uint256::zero(), debt2.amount_scaled);
        assert_eq!(
            Uint256::zero(),
            market_2_after_repay_all_2.debt_total_scaled
        );

        // *
        // Repay more native debt (should fail)
        // *
        let env = cosmwasm_std::testing::mock_env("borrower", &[coin(2000, "borrowedcoinnative")]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Cannot repay 0 debt");

        // *
        // Repay all cw20 debt (and then some)
        // *
        let block_time = market_2_after_repay_all_2.interests_last_updated + 5000u64;
        let repay_amount = 4800u128;

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps,
            &market_1_after_borrow_again,
            block_time,
            available_liquidity_cw20,
            TestUtilizationDeltas {
                less_debt: repay_amount,
                ..Default::default()
            },
        );

        let env = mars::testing::mock_env(
            "borrowedcoincw20",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::RepayCw20 {}).unwrap()),
            sender: borrower_addr.clone(),
            amount: Uint128(repay_amount),
        });

        let res = handle(&mut deps, env, msg).unwrap();

        let expected_repay_amount_scaled =
            Uint256::from(repay_amount) / expected_params_cw20.borrow_index;
        let expected_refund_amount: u128 = ((expected_repay_amount_scaled
            - expected_debt_scaled_1_after_borrow_again)
            * expected_params_cw20.borrow_index)
            .into();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_addr,
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: borrower_addr,
                    amount: expected_refund_amount.into(),
                })
                .unwrap(),
                send: vec![],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("market", "borrowedcoincw20"),
                log("user", "borrower"),
                log("amount", Uint128(repay_amount - expected_refund_amount)),
                log("borrow_index", expected_params_cw20.borrow_index),
                log("liquidity_index", expected_params_cw20.liquidity_index),
                log("borrow_rate", expected_params_cw20.borrow_rate),
                log("liquidity_rate", expected_params_cw20.liquidity_rate),
            ]
        );
        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(!get_bit(user.borrowed_assets, 0).unwrap());
        assert!(!get_bit(user.borrowed_assets, 1).unwrap());

        let debt1 = debts_asset_state_read(&deps.storage, cw20_contract_addr_canonical.as_slice())
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let market_1_after_repay_1 = markets_state_read(&deps.storage)
            .load(cw20_contract_addr_canonical.as_slice())
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

        let borrower_addr = HumanAddr::from("borrower");
        let borrower_canonical_addr = deps.api.canonical_address(&borrower_addr).unwrap();
        let ltv = Decimal256::from_ratio(7, 10);

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
        let market = th_init_market(&deps.api, &mut deps.storage, b"uusd", &mock_market);

        // Set tax data for uusd
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("uusd"), Uint128(100u128))],
        );

        // Set user as having the market_collateral deposited
        let deposit_amount = 110000u64;
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(borrower_canonical_addr.as_slice(), &user)
            .unwrap();

        // Set the querier to return collateral balance
        let deposit_coin_address = HumanAddr::from("matoken");
        deps.querier.set_cw20_balances(
            deposit_coin_address,
            &[(borrower_addr, Uint128::from(deposit_amount))],
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
        let msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "uusd".to_string(),
            },
            amount: max_to_borrow + Uint256::from(1u128),
        };
        let mut env = mars::testing::mock_env("borrower", MockEnvParams::default());
        env.block.time = new_block_time;
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "borrow amount exceeds maximum allowed given current collateral value",
        );

        let valid_amount = Uint256::from(deposit_amount) * ltv - Uint256::from(1000u128);
        let msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "uusd".to_string(),
            },
            amount: valid_amount,
        };
        let env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                block_time,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &market,
            block_time,
            initial_liquidity,
            TestUtilizationDeltas {
                less_liquidity: valid_amount.into(),
                more_debt: valid_amount.into(),
                ..Default::default()
            },
        );

        let market_after_borrow = markets_state_read(&deps.storage).load(b"uusd").unwrap();

        let user = users_state_read(&deps.storage)
            .load(borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());

        let debt = debts_asset_state_read(&deps.storage, b"uusd")
            .load(&borrower_canonical_addr.as_slice())
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
            &[(String::from("depositedcoin2"), Uint128(100u128))],
        );

        let cw20_contract_addr = HumanAddr::from("depositedcoin1");
        let cw20_contract_addr_canonical = deps.api.canonical_address(&cw20_contract_addr).unwrap();
        deps.querier.set_cw20_balances(
            cw20_contract_addr,
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(available_liquidity_1),
            )],
        );

        let exchange_rate_1 = Decimal::one();
        let exchange_rate_2 = Decimal::from_ratio(15u128, 4u128);
        let exchange_rate_3 = Decimal::one();

        let exchange_rates = &[(String::from("depositedcoin2"), exchange_rate_2)];
        deps.querier
            .set_native_exchange_rates(String::from("uusd"), &exchange_rates[..]);

        let mock_market_1 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken1"))
                .unwrap(),
            max_loan_to_value: Decimal256::from_ratio(8, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken2"))
                .unwrap(),
            max_loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_market_3 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken3"))
                .unwrap(),
            max_loan_to_value: Decimal256::from_ratio(4, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let market_1_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_market_1,
        );
        // should get index 1
        let market_2_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin2",
            &mock_market_2,
        );
        // should get index 2
        let market_3_initial =
            th_init_market(&deps.api, &mut deps.storage, b"uusd", &mock_market_3);

        let borrower_canonical_addr = deps
            .api
            .canonical_address(&HumanAddr::from("borrower"))
            .unwrap();

        // Set user as having all the markets as collateral
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, market_3_initial.index).unwrap();

        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(borrower_canonical_addr.as_slice(), &user)
            .unwrap();

        let ma_token_address_1 = HumanAddr::from("matoken1");
        let ma_token_address_2 = HumanAddr::from("matoken2");
        let ma_token_address_3 = HumanAddr::from("matoken3");

        let balance_1 = Uint128(4_000_000);
        let balance_2 = Uint128(7_000_000);
        let balance_3 = Uint128(3_000_000);

        let borrower_addr = HumanAddr(String::from("borrower"));

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
        let borrow_msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            amount: exceeding_borrow_amount,
        };
        let env = cosmwasm_std::testing::mock_env("borrower", &[]);
        let response = handle(&mut deps, env, borrow_msg);
        assert_generic_error_message(
            response,
            "borrow amount exceeds maximum allowed given current collateral value",
        );

        // borrow permissible amount given current collateral, should succeed
        let borrow_msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            amount: permissible_borrow_amount,
        };
        let env = cosmwasm_std::testing::mock_env("borrower", &[]);
        handle(&mut deps, env, borrow_msg).unwrap();
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
            &[(String::from("collateral"), Uint128(100u128))],
        );

        let debt_contract_addr = HumanAddr::from("debt");
        let debt_contract_addr_canonical = deps.api.canonical_address(&debt_contract_addr).unwrap();
        let user_address = HumanAddr::from("user");
        let user_canonical_addr = deps.api.canonical_address(&user_address).unwrap();
        let _collateral_address = HumanAddr::from("collateral");
        let liquidator_address = HumanAddr::from("liquidator");

        let collateral_max_ltv = Decimal256::from_ratio(5, 10);
        let collateral_maintenance_margin = Decimal256::from_ratio(6, 10);
        let collateral_liquidation_bonus = Decimal256::from_ratio(1, 10);
        let collateral_price = Decimal::from_ratio(2_u128, 1_u128);
        let debt_price = Decimal::from_ratio(1_u128, 1_u128);
        let user_collateral_balance = Uint128(2_000_000);
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

        config_state(&mut deps.storage)
            .update(|mut config| {
                config.close_factor = close_factor;
                Ok(config)
            })
            .unwrap();

        deps.querier.set_cw20_balances(
            debt_contract_addr.clone(),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(available_liquidity_debt),
            )],
        );

        // initialize collateral and debt markets
        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[("collateral".to_string(), collateral_price)],
        );

        let collateral_market_ma_token_human_addr = HumanAddr::from("ma_collateral");
        let collateral_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&collateral_market_ma_token_human_addr)
                .unwrap(),
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

        let collateral_market_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"collateral",
            &collateral_market,
        );

        let debt_market_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            debt_contract_addr_canonical.as_slice(),
            &debt_market,
        );

        let mut expected_user_debt_scaled = user_debt / debt_market_initial.liquidity_index;

        // Set user as having collateral and debt in respective markets
        {
            let mut user = User::default();
            set_bit(&mut user.collateral_assets, collateral_market_initial.index).unwrap();
            set_bit(&mut user.borrowed_assets, debt_market_initial.index).unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            users_bucket
                .save(user_canonical_addr.as_slice(), &user)
                .unwrap();
        }

        // trying to liquidate user with zero collateral balance should fail
        {
            deps.querier.set_cw20_balances(
                collateral_market_ma_token_human_addr,
                &[(user_address.clone(), Uint128::zero())],
            );

            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: true,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: first_debt_to_repay.into(),
            });

            let env = cosmwasm_std::testing::mock_env(debt_contract_addr.clone(), &[]);
            let response = handle(&mut deps, env, liquidate_msg);
            assert_generic_error_message(
                response,
                "user has no balance in specified collateral asset to be liquidated",
            );
        }

        // Set the querier to return positive collateral balance
        deps.querier.set_cw20_balances(
            HumanAddr::from("ma_collateral"),
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
            debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
                .save(user_canonical_addr.as_slice(), &debt)
                .unwrap();
            debts_asset_state(&mut deps.storage, b"uncollateralized_debt")
                .save(user_canonical_addr.as_slice(), &uncollateralized_debt)
                .unwrap();

            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: true,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: first_debt_to_repay.into(),
            });

            let env = cosmwasm_std::testing::mock_env(debt_contract_addr.clone(), &[]);
            let response = handle(&mut deps, env, liquidate_msg);
            assert_generic_error_message(response, "User has no outstanding debt in the specified debt asset and thus cannot be liquidated");
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
            debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
                .save(user_canonical_addr.as_slice(), &debt)
                .unwrap();
            debts_asset_state(&mut deps.storage, b"uncollateralized_debt")
                .save(user_canonical_addr.as_slice(), &uncollateralized_debt)
                .unwrap();
        }

        //  trying to liquidate without sending funds should fail
        {
            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: true,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: Uint128::zero(),
            });

            let env = cosmwasm_std::testing::mock_env(debt_contract_addr.clone(), &[]);
            let response = handle(&mut deps, env, liquidate_msg);
            assert_generic_error_message(
                response,
                "Must send more than 0 debt in order to liquidate",
            );
        }

        // Perform first successful liquidation receiving ma_token in return
        {
            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: true,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: first_debt_to_repay.into(),
            });

            let collateral_market_before = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_before = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            let block_time = first_block_time;
            let env = mars::testing::mock_env(
                "debt",
                MockEnvParams {
                    block_time,
                    ..Default::default()
                },
            );
            let res = handle(&mut deps, env.clone(), liquidate_msg).unwrap();

            // get expected indices and rates for debt market
            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps,
                &debt_market_initial,
                block_time,
                available_liquidity_debt,
                TestUtilizationDeltas {
                    less_debt: first_debt_to_repay.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_after = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            // TODO: not multiplying by collateral because it is a cw20 and Decimal::one
            // is the default price. Set a different price when implementing the oracle
            let expected_liquidated_collateral_amount = first_debt_to_repay
                * (Decimal256::one() + collateral_liquidation_bonus)
                / Decimal256::from(collateral_price);

            let expected_liquidated_collateral_amount_scaled = expected_liquidated_collateral_amount
                / get_updated_liquidity_index(&collateral_market_after, env.block.time);

            assert_eq!(
                res.messages,
                vec![CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("ma_collateral"),
                    msg: to_binary(&mars::ma_token::msg::HandleMsg::TransferOnLiquidation {
                        sender: user_address.clone(),
                        recipient: liquidator_address.clone(),
                        amount: expected_liquidated_collateral_amount_scaled.into(),
                    })
                    .unwrap(),
                    send: vec![],
                }),]
            );

            mars::testing::assert_eq_vec(
                res.log,
                vec![
                    log("action", "liquidate"),
                    log("collateral_market", "collateral"),
                    log("debt_market", debt_contract_addr.as_str()),
                    log("user", user_address.as_str()),
                    log("liquidator", liquidator_address.as_str()),
                    log(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount,
                    ),
                    log("debt_amount_repaid", first_debt_to_repay),
                    log("refund_amount", 0),
                    log("borrow_index", expected_debt_rates.borrow_index),
                    log("liquidity_index", expected_debt_rates.liquidity_index),
                    log("borrow_rate", expected_debt_rates.borrow_rate),
                    log("liquidity_rate", expected_debt_rates.liquidity_rate),
                ],
            );

            // check user still has deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = users_state_read(&deps.storage)
                .load(user_canonical_addr.as_slice())
                .unwrap();
            assert!(get_bit(user.collateral_assets, collateral_market_before.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_before.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let debt =
                debts_asset_state_read(&deps.storage, debt_contract_addr_canonical.as_slice())
                    .load(&user_canonical_addr.as_slice())
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
            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: false,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: second_debt_to_repay.into(),
            });

            let collateral_market_before = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_before = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            let block_time = second_block_time;
            let env = mars::testing::mock_env(
                "debt",
                MockEnvParams {
                    block_time,
                    ..Default::default()
                },
            );
            let res = handle(&mut deps, env, liquidate_msg).unwrap();

            // get expected indices and rates for debt and collateral markets
            let expected_debt_indices = th_get_expected_indices(&debt_market_before, block_time);
            let user_debt_asset_total_debt =
                expected_user_debt_scaled * expected_debt_indices.borrow;
            // Since debt is being over_repayed, we expect to max out the liquidatable debt
            let expected_less_debt = user_debt_asset_total_debt * close_factor;

            let expected_refund_amount = second_debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps,
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
                &deps,
                &collateral_market_before,
                block_time,
                available_liquidity_collateral, //this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: expected_liquidated_collateral_amount.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_after = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            let expected_liquidated_collateral_amount_scaled =
                expected_liquidated_collateral_amount / expected_collateral_rates.liquidity_index;

            assert_eq!(
                res.messages,
                vec![
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("ma_collateral"),
                        msg: to_binary(&mars::ma_token::msg::HandleMsg::Burn {
                            user: user_address.clone(),
                            amount: expected_liquidated_collateral_amount_scaled.into(),
                        })
                        .unwrap(),
                        send: vec![],
                    }),
                    CosmosMsg::Bank(BankMsg::Send {
                        from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        to_address: liquidator_address.clone(),
                        amount: vec![deduct_tax(
                            &deps,
                            Coin {
                                denom: String::from("collateral"),
                                amount: expected_liquidated_collateral_amount.into(),
                            }
                        )
                        .unwrap()],
                    }),
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("debt"),
                        msg: to_binary(&Cw20HandleMsg::Transfer {
                            recipient: liquidator_address.clone(),
                            amount: expected_refund_amount.into(),
                        })
                        .unwrap(),
                        send: vec![],
                    }),
                ]
            );

            mars::testing::assert_eq_vec(
                vec![
                    log("action", "liquidate"),
                    log("collateral_market", "collateral"),
                    log("debt_market", debt_contract_addr.as_str()),
                    log("user", user_address.as_str()),
                    log("liquidator", liquidator_address.as_str()),
                    log(
                        "collateral_amount_liquidated",
                        expected_liquidated_collateral_amount,
                    ),
                    log("debt_amount_repaid", expected_less_debt),
                    log("refund_amount", expected_refund_amount),
                    log("borrow_index", expected_debt_rates.borrow_index),
                    log("liquidity_index", expected_debt_rates.liquidity_index),
                    log("borrow_rate", expected_debt_rates.borrow_rate),
                    log("liquidity_rate", expected_debt_rates.liquidity_rate),
                    log("borrow_index", expected_collateral_rates.borrow_index),
                    log("liquidity_index", expected_collateral_rates.liquidity_index),
                    log("borrow_rate", expected_collateral_rates.borrow_rate),
                    log("liquidity_rate", expected_collateral_rates.liquidity_rate),
                ],
                res.log,
            );

            // check user still has deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = users_state_read(&deps.storage)
                .load(user_canonical_addr.as_slice())
                .unwrap();
            assert!(get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled = expected_less_debt / expected_debt_rates.borrow_index;
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt =
                debts_asset_state_read(&deps.storage, debt_contract_addr_canonical.as_slice())
                    .load(&user_canonical_addr.as_slice())
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
                HumanAddr::from("ma_collateral"),
                &[(user_address.clone(), user_collateral_balance_scaled.into())],
            );

            // set user to have positive debt amount in debt asset
            let debt = Debt {
                amount_scaled: expected_user_debt_scaled,
                uncollateralized: false,
            };
            debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
                .save(user_canonical_addr.as_slice(), &debt)
                .unwrap();

            let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
                msg: Some(
                    to_binary(&ReceiveMsg::LiquidateCw20 {
                        collateral_asset: Asset::Native {
                            denom: "collateral".to_string(),
                        },
                        debt_asset_address: debt_contract_addr.clone(),
                        user_address: user_address.clone(),
                        receive_ma_token: false,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: debt_to_repay.into(),
            });

            let collateral_market_before = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_before = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            let block_time = second_block_time;
            let env = mars::testing::mock_env(
                "debt",
                MockEnvParams {
                    block_time,
                    ..Default::default()
                },
            );
            let res = handle(&mut deps, env, liquidate_msg).unwrap();

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
                &deps,
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
                &deps,
                &collateral_market_before,
                block_time,
                available_liquidity_collateral, // this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: user_collateral_balance.into(),
                    ..Default::default()
                },
            );

            let collateral_market_after = markets_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_market_after = markets_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            // NOTE: expected_liquidated_collateral_amount_scaled should be equal user_collateral_balance_scaled
            // but there are rounding errors
            let expected_liquidated_collateral_amount_scaled =
                user_collateral_balance / expected_collateral_rates.liquidity_index;

            assert_eq!(
                res.messages,
                vec![
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("ma_collateral"),
                        msg: to_binary(&mars::ma_token::msg::HandleMsg::Burn {
                            user: user_address.clone(),
                            amount: expected_liquidated_collateral_amount_scaled.into(),
                        })
                        .unwrap(),
                        send: vec![],
                    }),
                    CosmosMsg::Bank(BankMsg::Send {
                        from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        to_address: liquidator_address.clone(),
                        amount: vec![deduct_tax(
                            &deps,
                            Coin {
                                denom: String::from("collateral"),
                                amount: user_collateral_balance.into(),
                            }
                        )
                        .unwrap()],
                    }),
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("debt"),
                        msg: to_binary(&Cw20HandleMsg::Transfer {
                            recipient: liquidator_address.clone(),
                            amount: expected_refund_amount.into(),
                        })
                        .unwrap(),
                        send: vec![],
                    }),
                ]
            );

            mars::testing::assert_eq_vec(
                vec![
                    log("action", "liquidate"),
                    log("collateral_market", "collateral"),
                    log("debt_market", debt_contract_addr.as_str()),
                    log("user", user_address.as_str()),
                    log("liquidator", liquidator_address.as_str()),
                    log("collateral_amount_liquidated", user_collateral_balance),
                    log("debt_amount_repaid", expected_less_debt),
                    log("refund_amount", expected_refund_amount),
                    log("borrow_index", expected_debt_rates.borrow_index),
                    log("liquidity_index", expected_debt_rates.liquidity_index),
                    log("borrow_rate", expected_debt_rates.borrow_rate),
                    log("liquidity_rate", expected_debt_rates.liquidity_rate),
                    log("borrow_index", expected_collateral_rates.borrow_index),
                    log("liquidity_index", expected_collateral_rates.liquidity_index),
                    log("borrow_rate", expected_collateral_rates.borrow_rate),
                    log("liquidity_rate", expected_collateral_rates.liquidity_rate),
                ],
                res.log,
            );

            // check user doesn't have deposited collateral asset and
            // still has outstanding debt in debt asset
            let user = users_state_read(&deps.storage)
                .load(user_canonical_addr.as_slice())
                .unwrap();
            assert!(!get_bit(user.collateral_assets, collateral_market_initial.index).unwrap());
            assert!(get_bit(user.borrowed_assets, debt_market_initial.index).unwrap());

            // check user's debt decreased by the appropriate amount
            let expected_less_debt_scaled = expected_less_debt / expected_debt_rates.borrow_index;
            expected_user_debt_scaled = expected_user_debt_scaled - expected_less_debt_scaled;

            let debt =
                debts_asset_state_read(&deps.storage, debt_contract_addr_canonical.as_slice())
                    .load(&user_canonical_addr.as_slice())
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

        let debt_contract_addr = HumanAddr::from("debt");
        let debt_contract_addr_canonical = deps.api.canonical_address(&debt_contract_addr).unwrap();
        deps.querier.set_cw20_balances(
            debt_contract_addr.clone(),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(available_liquidity_debt),
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
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("collateral"))
                .unwrap(),
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
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("debt"))
                .unwrap(),
            max_loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::from(20_000_000u64),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };

        // initialize markets
        let collateral_market_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"collateral",
            &collateral_market,
        );

        let debt_market_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            debt_contract_addr_canonical.as_slice(),
            &debt_market,
        );

        // test health factor check
        let healthy_user_address = HumanAddr::from("healthy_user");
        let healthy_user_canonical_addr =
            deps.api.canonical_address(&healthy_user_address).unwrap();

        // Set user as having collateral and debt in respective markets
        let mut healthy_user = User::default();

        set_bit(
            &mut healthy_user.collateral_assets,
            collateral_market_initial.index,
        )
        .unwrap();
        set_bit(&mut healthy_user.borrowed_assets, debt_market_initial.index).unwrap();

        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(healthy_user_canonical_addr.as_slice(), &healthy_user)
            .unwrap();

        // set initial collateral and debt balances for user
        let collateral_address = HumanAddr::from("collateral");
        let healthy_user_collateral_balance = Uint128(10_000_000);

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
        debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
            .save(healthy_user_canonical_addr.as_slice(), &healthy_user_debt)
            .unwrap();
        debts_asset_state(&mut deps.storage, b"uncollateralized_debt")
            .save(
                healthy_user_canonical_addr.as_slice(),
                &uncollateralized_debt,
            )
            .unwrap();

        // perform liquidation (should fail because health factor is > 1)
        let liquidator_address = HumanAddr::from("liquidator");
        let debt_to_cover = Uint256::from(1_000_000u64);

        let liquidate_msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::LiquidateCw20 {
                    collateral_asset: Asset::Native {
                        denom: "collateral".to_string(),
                    },
                    debt_asset_address: debt_contract_addr.clone(),
                    user_address: healthy_user_address,
                    receive_ma_token: true,
                })
                .unwrap(),
            ),
            sender: liquidator_address,
            amount: debt_to_cover.into(),
        });

        let env = cosmwasm_std::testing::mock_env(debt_contract_addr, &[]);
        let response = handle(&mut deps, env, liquidate_msg);
        assert_generic_error_message(
            response,
            "User's health factor is not less than 1 and thus cannot be liquidated",
        );
    }

    #[test]
    fn test_finalize_liquidity_token_transfer() {
        // Setup
        let mut deps = th_setup(&[]);
        let env_matoken = cosmwasm_std::testing::mock_env(HumanAddr::from("masomecoin"), &[]);

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("masomecoin"))
                .unwrap(),
            liquidity_index: Decimal256::one(),
            maintenance_margin: Decimal256::from_ratio(5, 10),
            ..Default::default()
        };
        let market = th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);
        let debt_mock_market = Market {
            borrow_index: Decimal256::one(),
            ..Default::default()
        };
        let debt_market =
            th_init_market(&deps.api, &mut deps.storage, b"debtcoin", &debt_mock_market);

        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[
                ("somecoin".to_string(), Decimal::from_ratio(1u128, 2u128)),
                ("debtcoin".to_string(), Decimal::from_ratio(2u128, 1u128)),
            ],
        );

        let (sender_address, sender_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "fromaddr");
        let (recipient_address, recipient_canonical_address) =
            mars::testing::get_test_addresses(&deps.api, "toaddr");

        deps.querier.set_cw20_balances(
            HumanAddr::from("masomecoin"),
            &[(sender_address.clone(), Uint128(500_000))],
        );

        {
            let mut sender_user = User::default();
            set_bit(&mut sender_user.collateral_assets, market.index).unwrap();
            users_state(&mut deps.storage)
                .save(sender_canonical_address.as_slice(), &sender_user)
                .unwrap();
        }

        // Finalize transfer with sender not borrowing passes
        {
            let msg = HandleMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
                sender_previous_balance: Uint128(1_000_000),
                recipient_previous_balance: Uint128(0),
                amount: Uint128(500_000),
            };

            handle(&mut deps, env_matoken.clone(), msg).unwrap();

            let users_bucket = users_state_read(&deps.storage);
            let sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            let recipient_user = users_bucket
                .load(recipient_canonical_address.as_slice())
                .unwrap();
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
            debts_asset_state(&mut deps.storage, b"debtcoin")
                .save(sender_canonical_address.as_slice(), &debt)
                .unwrap();
            debts_asset_state(&mut deps.storage, b"uncollateralized_debt")
                .save(sender_canonical_address.as_slice(), &uncollateralized_debt)
                .unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            let mut sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_market.index).unwrap();
            users_bucket
                .save(sender_canonical_address.as_slice(), &sender_user)
                .unwrap();
        }

        {
            let msg = HandleMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
                sender_previous_balance: Uint128(1_000_000),
                recipient_previous_balance: Uint128(0),
                amount: Uint128(500_000),
            };

            let response = handle(&mut deps, env_matoken.clone(), msg);
            assert_generic_error_message(response, "Cannot make token transfer if it results in a health factor lower than 1 for the sender");
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
            debts_asset_state(&mut deps.storage, b"debtcoin")
                .save(sender_canonical_address.as_slice(), &debt)
                .unwrap();
            debts_asset_state(&mut deps.storage, b"uncollateralized_debt")
                .save(sender_canonical_address.as_slice(), &uncollateralized_debt)
                .unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            let mut sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_market.index).unwrap();
            users_bucket
                .save(sender_canonical_address.as_slice(), &sender_user)
                .unwrap();
        }

        {
            let msg = HandleMsg::FinalizeLiquidityTokenTransfer {
                sender_address: sender_address.clone(),
                recipient_address: recipient_address.clone(),
                sender_previous_balance: Uint128(500_000),
                recipient_previous_balance: Uint128(500_000),
                amount: Uint128(500_000),
            };

            handle(&mut deps, env_matoken, msg).unwrap();

            let users_bucket = users_state_read(&deps.storage);
            let sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            let recipient_user = users_bucket
                .load(recipient_canonical_address.as_slice())
                .unwrap();
            // Should set deposited to false as: previous_balance - amount = 0
            assert!(!get_bit(sender_user.collateral_assets, market.index).unwrap());
            assert!(get_bit(recipient_user.collateral_assets, market.index).unwrap());
        }

        // Calling this with other token fails
        {
            let msg = HandleMsg::FinalizeLiquidityTokenTransfer {
                sender_address,
                recipient_address,
                sender_previous_balance: Uint128(500_000),
                recipient_previous_balance: Uint128(500_000),
                amount: Uint128(500_000),
            };
            let env = cosmwasm_std::testing::mock_env(HumanAddr::from("othertoken"), &[]);

            let res_error = handle(&mut deps, env, msg).unwrap_err();
            match res_error {
                StdError::NotFound { .. } => {}
                e => panic!("Unexpected error: {}", e),
            }
        }
    }

    #[test]
    fn test_uncollateralized_loan_limits() {
        let available_liquidity = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity, "somecoin")]);

        // Set tax data
        deps.querier.set_native_tax(
            Decimal::from_ratio(1u128, 100u128),
            &[(String::from("somecoin"), Uint128(100u128))],
        );

        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
        let market_initial =
            th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);

        let borrower_addr = HumanAddr::from("borrower");
        let borrower_canonical_addr = deps.api.canonical_address(&borrower_addr).unwrap();

        let mut block_time = mock_market.interests_last_updated + 10000u64;
        let initial_uncollateralized_loan_limit = Uint128::from(2400_u128);

        // Update uncollateralized loan limit
        let update_limit_msg = HandleMsg::UpdateUncollateralizedLoanLimit {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            user_address: borrower_addr.clone(),
            new_limit: initial_uncollateralized_loan_limit,
        };

        // update limit as unauthorized user, should fail
        let update_limit_env = mars::testing::mock_env(
            "random",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let res_error = handle(&mut deps, update_limit_env, update_limit_msg.clone()).unwrap_err();
        assert_eq!(res_error, StdError::unauthorized());

        // Update borrower limit as owner
        let update_limit_env = mars::testing::mock_env(
            "owner",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        handle(&mut deps, update_limit_env, update_limit_msg).unwrap();

        // check user's limit has been updated to the appropriate amount
        let limit = uncollateralized_loan_limits_read(&deps.storage, b"somecoin")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert_eq!(limit, initial_uncollateralized_loan_limit);

        // check user's uncollateralized debt flag is true (limit > 0)
        let debt = debts_asset_state_read(&deps.storage, b"somecoin")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(debt.uncollateralized);

        // Borrow asset
        block_time += 1000_u64;
        let initial_borrow_amount =
            initial_uncollateralized_loan_limit.multiply_ratio(1_u64, 2_u64);
        let borrow_msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(initial_borrow_amount),
        };
        let borrow_env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, borrow_env, borrow_msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
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
            vec![CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: borrower_addr.clone(),
                amount: vec![deduct_tax(
                    &deps,
                    Coin {
                        denom: String::from("somecoin"),
                        amount: initial_borrow_amount,
                    }
                )
                .unwrap()],
            })]
        );

        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("market", "somecoin"),
                log("user", "borrower"),
                log("amount", initial_borrow_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        // Check debt
        let user = users_state_read(&deps.storage)
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(get_bit(user.borrowed_assets, 0).unwrap());

        let debt = debts_asset_state_read(&deps.storage, b"somecoin")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let expected_debt_scaled_after_borrow =
            Uint256::from(initial_borrow_amount) / expected_params.borrow_index;

        assert_eq!(expected_debt_scaled_after_borrow, debt.amount_scaled);

        // Borrow an amount less than initial limit but exceeding current limit
        let remaining_limit =
            (initial_uncollateralized_loan_limit - initial_borrow_amount).unwrap();
        let exceeding_limit = remaining_limit + Uint128::from(100_u64);

        block_time += 1000_u64;
        let borrow_msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(exceeding_limit),
        };
        let borrow_env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, borrow_env, borrow_msg);
        assert_generic_error_message(
            response,
            "borrow amount exceeds uncollateralized loan limit given existing debt",
        );

        // Borrow a valid amount given uncollateralized loan limit
        block_time += 1000_u64;
        let borrow_msg = HandleMsg::Borrow {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Uint256::from(remaining_limit),
        };
        let borrow_env = mars::testing::mock_env(
            "borrower",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        handle(&mut deps, borrow_env, borrow_msg).unwrap();

        // Set limit to zero
        let update_allowance_msg = HandleMsg::UpdateUncollateralizedLoanLimit {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            user_address: borrower_addr,
            new_limit: Uint128::zero(),
        };

        let allowance_env = mars::testing::mock_env(
            "owner",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        handle(&mut deps, allowance_env, update_allowance_msg).unwrap();

        // check user's allowance is zero
        let allowance = uncollateralized_loan_limits_read(&deps.storage, b"somecoin")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert_eq!(allowance, Uint128::zero());

        // check user's uncollateralized debt flag is false (limit == 0)
        let debt = debts_asset_state_read(&deps.storage, b"somecoin")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        assert!(!debt.uncollateralized);
    }

    #[test]
    fn test_update_asset_collateral() {
        let mut deps = th_setup(&[]);

        let user_addr = HumanAddr(String::from("user"));
        let user_canonical_addr = deps.api.canonical_address(&user_addr).unwrap();

        let ma_token_address_1 = HumanAddr::from("matoken1");
        let mock_market_1 = Market {
            ma_token_address: deps.api.canonical_address(&ma_token_address_1).unwrap(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_market_2 = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken2"))
                .unwrap(),
            ..Default::default()
        };
        let cw20_contract_addr = HumanAddr::from("depositedcoin1");
        let cw20_contract_addr_canonical = deps.api.canonical_address(&cw20_contract_addr).unwrap();

        // Should get index 0
        let market_1_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_market_1,
        );
        // Should get index 1
        let market_2_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin2",
            &mock_market_2,
        );

        // Set second asset as collateral
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(user_canonical_addr.as_slice(), &user)
            .unwrap();

        // Set the querier to return zero for the first asset
        deps.querier.set_cw20_balances(
            ma_token_address_1.clone(),
            &[(user_addr.clone(), Uint128::zero())],
        );

        // Enable first market index which is currently disabled as collateral and ma-token balance is 0
        let update_msg = HandleMsg::UpdateUserCollateralAssetStatus {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.clone(),
            },
            enable: true,
        };
        let env = cosmwasm_std::testing::mock_env("user", &[]);
        let response = handle(&mut deps, env.clone(), update_msg.clone());
        assert_generic_error_message(
            response,
            &format!(
                "User address {} has no balance in specified collateral asset {}",
                user_addr.as_str(),
                String::from(cw20_contract_addr.as_str())
            ),
        );

        let user = users_state(&mut deps.storage)
            .load(user_canonical_addr.as_slice())
            .unwrap();
        let market_1_collateral = get_bit(user.collateral_assets, market_1_initial.index).unwrap();
        // Balance for first asset is zero so don't update bit
        assert!(!market_1_collateral);

        // Set the querier to return balance more than zero for the first asset
        deps.querier
            .set_cw20_balances(ma_token_address_1, &[(user_addr, Uint128(100_000))]);

        // Enable first market index which is currently disabled as collateral and ma-token balance is more than 0
        let _res = handle(&mut deps, env.clone(), update_msg).unwrap();
        let user = users_state(&mut deps.storage)
            .load(user_canonical_addr.as_slice())
            .unwrap();
        let market_1_collateral = get_bit(user.collateral_assets, market_1_initial.index).unwrap();
        // Balance for first asset is more than zero so update bit
        assert!(market_1_collateral);

        // Disable second market index
        let update_msg = HandleMsg::UpdateUserCollateralAssetStatus {
            asset: Asset::Native {
                denom: "depositedcoin2".to_string(),
            },
            enable: false,
        };
        let _res = handle(&mut deps, env, update_msg).unwrap();
        let user = users_state(&mut deps.storage)
            .load(user_canonical_addr.as_slice())
            .unwrap();
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
            &[(String::from("somecoin"), Uint128(100u128))],
        );

        let asset = Asset::Native {
            denom: String::from("somecoin"),
        };
        let protocol_income_to_distribute = Uint256::from(1_000_000_u64);

        // initialize market with non-zero amount of protocol_income_to_distribute
        let mock_market = Market {
            ma_token_address: deps
                .api
                .canonical_address(&HumanAddr::from("matoken"))
                .unwrap(),
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
        let market_initial =
            th_init_market(&deps.api, &mut deps.storage, b"somecoin", &mock_market);

        let mut block_time = mock_market.interests_last_updated + 10000u64;

        // call function providing amount exceeding protocol_income_to_distribute, should fail
        let exceeding_amount = protocol_income_to_distribute + Uint256::from(1_000_u64);
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(exceeding_amount),
        };
        let env = mars::testing::mock_env(
            "anyone",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        let response = handle(&mut deps, env.clone(), distribute_income_msg);
        assert_generic_error_message(
            response,
            "amount specified exceeds market's income to be distributed",
        );

        // call function providing amount less than protocol_income_to_distribute
        let permissible_amount = Decimal256::from_ratio(1, 2) * protocol_income_to_distribute;
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset: asset.clone(),
            amount: Some(permissible_amount),
        };
        let res = handle(&mut deps, env.clone(), distribute_income_msg).unwrap();

        let config = config_state_read(&deps.storage).load().unwrap();
        let market_after_distribution =
            markets_state_read(&deps.storage).load(b"somecoin").unwrap();

        let expected_insurance_fund_amount = permissible_amount * config.insurance_fund_fee_share;
        let expected_treasury_amount = permissible_amount * config.treasury_fee_share;
        let expected_staking_amount =
            permissible_amount - (expected_insurance_fund_amount + expected_treasury_amount);

        let scaled_mint_amount =
            expected_treasury_amount / get_updated_liquidity_index(&market_initial, env.block.time);

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("insurance_fund"),
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_insurance_fund_amount.into(),
                        }
                    )
                    .unwrap()],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: deps
                        .api
                        .human_address(&market_initial.ma_token_address)
                        .unwrap(),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Mint {
                        recipient: HumanAddr::from("treasury"),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("staking"),
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_staking_amount.into(),
                        }
                    )
                    .unwrap()],
                })
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "distribute_protocol_income"),
                log("asset", "somecoin"),
                log("amount", permissible_amount),
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
        let env = mars::testing::mock_env(
            "anyone",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset,
            amount: None,
        };
        let res = handle(&mut deps, env.clone(), distribute_income_msg).unwrap();

        // verify messages are correct and protocol_income_to_distribute field is now zero
        let expected_insurance_amount =
            expected_remaining_income_to_be_distributed * config.insurance_fund_fee_share;
        let expected_treasury_amount =
            expected_remaining_income_to_be_distributed * config.treasury_fee_share;
        let expected_staking_amount = expected_remaining_income_to_be_distributed
            - (expected_insurance_amount + expected_treasury_amount);

        let scaled_mint_amount = expected_treasury_amount
            / get_updated_liquidity_index(&market_after_distribution, env.block.time);

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("insurance_fund"),
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_insurance_fund_amount.into(),
                        }
                    )
                    .unwrap()],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: deps
                        .api
                        .human_address(&market_initial.ma_token_address)
                        .unwrap(),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Mint {
                        recipient: HumanAddr::from("treasury"),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("staking"),
                    amount: vec![deduct_tax(
                        &deps,
                        Coin {
                            denom: "somecoin".to_string(),
                            amount: expected_staking_amount.into(),
                        }
                    )
                    .unwrap()],
                })
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "distribute_protocol_income"),
                log("asset", "somecoin"),
                log("amount", expected_remaining_income_to_be_distributed),
            ]
        );

        let market_after_second_distribution =
            markets_state_read(&deps.storage).load(b"somecoin").unwrap();
        assert_eq!(
            market_after_second_distribution.protocol_income_to_distribute,
            Uint256::zero()
        );
    }

    #[test]
    fn test_query_collateral() {
        let mut deps = th_setup(&[]);

        let user_addr = HumanAddr(String::from("user"));
        let user_canonical_addr = deps.api.canonical_address(&user_addr).unwrap();

        // Setup first market containing a CW20 asset
        let cw20_contract_addr_1 = HumanAddr::from("depositedcoin1");
        deps.querier
            .set_cw20_symbol(cw20_contract_addr_1.clone(), "DP1".to_string());
        let market_1_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            deps.api
                .canonical_address(&cw20_contract_addr_1)
                .unwrap()
                .as_slice(),
            &Market {
                asset_type: AssetType::Cw20,
                ..Default::default()
            },
        );

        // Setup second market containing a native asset
        let market_2_initial = th_init_market(
            &deps.api,
            &mut deps.storage,
            String::from("uusd").as_bytes(),
            &Market {
                ..Default::default()
            },
        );

        // Set second market as collateral
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, market_2_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(user_canonical_addr.as_slice(), &user)
            .unwrap();

        // Assert markets correctly return collateral status
        let res = query_collateral(&deps, user_addr.clone()).unwrap();
        assert_eq!(res.collateral[0].denom, String::from("DP1"));
        assert!(!res.collateral[0].enabled);
        assert_eq!(res.collateral[1].denom, String::from("uusd"));
        assert!(res.collateral[1].enabled);

        // Set first market as collateral
        set_bit(&mut user.collateral_assets, market_1_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(user_canonical_addr.as_slice(), &user)
            .unwrap();

        // Assert markets correctly return collateral status
        let res = query_collateral(&deps, user_addr).unwrap();
        assert_eq!(res.collateral[0].denom, String::from("DP1"));
        assert!(res.collateral[0].enabled);
        assert_eq!(res.collateral[1].denom, String::from("uusd"));
        assert!(res.collateral[1].enabled);
    }

    // TEST HELPERS

    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        let config = CreateOrUpdateConfig {
            owner: Some(HumanAddr::from("owner")),
            address_provider_address: Some(HumanAddr::from("address_provider")),
            insurance_fund_fee_share: Some(Decimal256::from_ratio(5, 10)),
            treasury_fee_share: Some(Decimal256::from_ratio(3, 10)),
            ma_token_code_id: Some(1u64),
            close_factor: Some(Decimal256::from_ratio(1, 2)),
        };
        let msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        init(&mut deps, env, msg).unwrap();

        deps
    }

    impl Default for Market {
        fn default() -> Self {
            Market {
                index: 0,
                ma_token_address: Default::default(),
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

    fn th_init_market<S: Storage, A: Api>(
        _api: &A,
        storage: &mut S,
        key: &[u8],
        market: &Market,
    ) -> Market {
        let mut index = 0;

        money_market_state(storage)
            .update(|mut mm: RedBank| -> StdResult<RedBank> {
                index = mm.market_count;
                mm.market_count += 1;
                Ok(mm)
            })
            .unwrap();

        let mut market_bucket = markets_state(storage);

        let new_market = Market {
            index,
            ..market.clone()
        };

        market_bucket.save(key, &new_market).unwrap();

        market_references_state(storage)
            .save(
                &index.to_be_bytes(),
                &MarketReferences {
                    reference: key.to_vec(),
                },
            )
            .unwrap();

        market_ma_tokens_state(storage)
            .save(new_market.ma_token_address.as_slice(), &key.to_vec())
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
    fn th_get_expected_indices_and_rates<S: Storage, A: Api, Q: Querier>(
        deps: &Extern<S, A, Q>,
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

        let config = config_state_read(&deps.storage).load().unwrap();

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
