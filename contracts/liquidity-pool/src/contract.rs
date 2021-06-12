use std::str;

use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, LogAttribute, MigrateResponse, MigrateResult, Order,
    Querier, StdError, StdResult, Storage, Uint128, WasmMsg,
};
use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use terra_cosmwasm::TerraQuerier;

use mars::cw20_token;
use mars::helpers::{cw20_get_balance, cw20_get_symbol, unwrap_or};
use mars::liquidity_pool::msg::{
    Asset, AssetType, ConfigResponse, CreateOrUpdateConfig, DebtInfo, DebtResponse, HandleMsg,
    InitMsg, InitOrUpdateAssetParams, MigrateMsg, QueryMsg, ReceiveMsg, ReserveInfo,
    ReserveResponse, ReservesListResponse,
};

use crate::state::{
    config_state, config_state_read, debts_asset_state, debts_asset_state_read, money_market_state,
    money_market_state_read, reserve_ma_tokens_state, reserve_ma_tokens_state_read,
    reserve_references_state, reserve_references_state_read, reserves_state, reserves_state_read,
    uncollateralized_loan_limits, uncollateralized_loan_limits_read, users_state, users_state_read,
    Config, Debt, MoneyMarket, Reserve, ReserveReferences, User,
};

// CONSTANTS

const SECONDS_PER_YEAR: u64 = 31536000u64;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        treasury_contract_address,
        insurance_fund_contract_address,
        staking_contract_address,
        insurance_fund_fee_share,
        treasury_fee_share,
        ma_token_code_id,
        close_factor,
    } = msg.config;

    // All fields should be available
    let available = treasury_contract_address.is_some()
        && insurance_fund_contract_address.is_some()
        && staking_contract_address.is_some()
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
        owner: deps.api.canonical_address(&env.message.sender)?,
        treasury_contract_address: deps
            .api
            .canonical_address(&treasury_contract_address.unwrap())?,
        insurance_fund_contract_address: deps
            .api
            .canonical_address(&insurance_fund_contract_address.unwrap())?,
        staking_contract_address: deps
            .api
            .canonical_address(&staking_contract_address.unwrap())?,
        ma_token_code_id: ma_token_code_id.unwrap(),
        close_factor: close_factor.unwrap(),
        insurance_fund_fee_share: insurance_fund_fee_share.unwrap(),
        treasury_fee_share: treasury_fee_share.unwrap(),
    };
    config.validate()?;

    config_state(&mut deps.storage).save(&config)?;

    money_market_state(&mut deps.storage).save(&MoneyMarket { reserve_count: 0 })?;

    Ok(InitResponse::default())
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::UpdateConfig { owner, config } => handle_update_config(deps, env, owner, config),
        HandleMsg::Receive(cw20_msg) => receive_cw20(deps, env, cw20_msg),
        HandleMsg::InitAsset {
            asset,
            asset_params,
        } => handle_init_asset(deps, env, asset, asset_params),
        HandleMsg::UpdateAsset {
            asset,
            asset_params,
        } => handle_update_asset(deps, env, asset, asset_params),
        HandleMsg::InitAssetTokenCallback { reference } => {
            init_asset_token_callback(deps, env, reference)
        }
        HandleMsg::DepositNative { denom } => {
            let deposit_amount = get_denom_amount_from_coins(&env.message.sent_funds, &denom);
            let depositor_address = env.message.sender.clone();
            handle_deposit(
                deps,
                env,
                depositor_address,
                denom.as_bytes(),
                denom.as_str(),
                deposit_amount,
            )
        }
        HandleMsg::Borrow { asset, amount } => handle_borrow(deps, env, asset, amount),
        HandleMsg::RepayNative { denom } => {
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
        HandleMsg::LiquidateNative {
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
        HandleMsg::FinalizeLiquidityTokenTransfer {
            sender_address,
            recipient_address,
            sender_previous_balance,
            recipient_previous_balance,
            amount,
        } => handle_finalize_liquidity_token_transfer(
            deps,
            env,
            sender_address,
            recipient_address,
            sender_previous_balance,
            recipient_previous_balance,
            amount,
        ),
        HandleMsg::UpdateUncollateralizedLoanLimit {
            user_address,
            asset,
            new_limit,
        } => handle_update_uncollateralized_loan_limit(deps, env, user_address, asset, new_limit),
        HandleMsg::UpdateUserCollateralAssetStatus { asset, enable } => {
            handle_update_user_collateral_asset_status(deps, env, asset, enable)
        }

        HandleMsg::DistributeProtocolIncome { asset, amount } => {
            handle_distribute_protocol_income(deps, env, asset, amount)
        }
    }
}

/// Update config
pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    new_config: CreateOrUpdateConfig,
) -> StdResult<HandleResponse> {
    let mut config = config_state_read(&deps.storage).load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        treasury_contract_address,
        insurance_fund_contract_address,
        staking_contract_address,
        insurance_fund_fee_share,
        treasury_fee_share,
        ma_token_code_id,
        close_factor,
    } = new_config;

    // Update config
    config.owner = unwrap_or(deps.api, owner, config.owner)?;
    config.treasury_contract_address = unwrap_or(
        deps.api,
        treasury_contract_address,
        config.treasury_contract_address,
    )?;
    config.insurance_fund_contract_address = unwrap_or(
        deps.api,
        insurance_fund_contract_address,
        config.insurance_fund_contract_address,
    )?;
    config.staking_contract_address = unwrap_or(
        deps.api,
        staking_contract_address,
        config.staking_contract_address,
    )?;
    config.ma_token_code_id = ma_token_code_id.unwrap_or(config.ma_token_code_id);
    config.close_factor = close_factor.unwrap_or(config.close_factor);
    config.insurance_fund_fee_share =
        insurance_fund_fee_share.unwrap_or(config.insurance_fund_fee_share);
    config.treasury_fee_share = treasury_fee_share.unwrap_or(config.treasury_fee_share);

    // Validate config
    config.validate()?;

    config_state(&mut deps.storage).save(&config)?;

    Ok(HandleResponse::default())
}

/// cw20 receive implementation
pub fn receive_cw20<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<HandleResponse> {
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            ReceiveMsg::Redeem {} => {
                let ma_contract_canonical_addr = deps.api.canonical_address(&env.message.sender)?;
                let reference = match reserve_ma_tokens_state_read(&deps.storage)
                    .load(ma_contract_canonical_addr.as_slice())
                {
                    Ok(res) => res,
                    Err(_) => return Err(StdError::unauthorized()),
                };

                handle_redeem(
                    deps,
                    env,
                    reference.as_slice(),
                    cw20_msg.sender,
                    Uint256::from(cw20_msg.amount),
                )
            }
            ReceiveMsg::DepositCw20 {} => {
                let token_contract_address = env.message.sender.clone();
                handle_deposit(
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
pub fn handle_redeem<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset_reference: &[u8],
    redeemer_address: HumanAddr,
    burn_amount: Uint256,
) -> StdResult<HandleResponse> {
    // Sender must be the corresponding ma token contract
    let mut reserve = reserves_state_read(&deps.storage).load(asset_reference)?;
    if deps.api.canonical_address(&env.message.sender)? != reserve.ma_token_address {
        return Err(StdError::unauthorized());
    }
    reserve_apply_accumulated_interests(&env, &mut reserve);

    // Redeem amount is computed with the updated index
    let redeem_amount = burn_amount * reserve.liquidity_index;

    reserve_update_interest_rates(&deps, &env, asset_reference, &mut reserve, redeem_amount)?;
    reserves_state(&mut deps.storage).save(asset_reference, &reserve)?;

    // Check contract has sufficient balance to send back
    let (balance, asset_label) = match reserve.asset_type {
        AssetType::Native => {
            let asset_label = match str::from_utf8(asset_reference) {
                Ok(res) => res,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            };
            (
                deps.querier
                    .query_balance(&env.contract.address, &asset_label)?
                    .amount,
                String::from(asset_label),
            )
        }
        AssetType::Cw20 => {
            let cw20_contract_addr = deps
                .api
                .human_address(&CanonicalAddr::from(asset_reference))?;
            (
                cw20_get_balance(
                    &deps.querier,
                    cw20_contract_addr.clone(),
                    env.contract.address.clone(),
                )?,
                String::from(cw20_contract_addr.as_str()),
            )
        }
    };
    if redeem_amount > Uint256::from(balance) {
        return Err(StdError::generic_err(
            "Redeem amount exceeds contract balance",
        ));
    }

    let mut log = vec![
        log("action", "redeem"),
        log("reserve", asset_label.as_str()),
        log("user", redeemer_address.as_str()),
        log("burn_amount", burn_amount),
        log("redeem_amount", redeem_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &reserve);

    let mut messages = vec![CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
        send: vec![],
        msg: to_binary(&Cw20HandleMsg::Burn {
            amount: burn_amount.into(),
        })?,
    })];

    let redeem_msg = match reserve.asset_type {
        AssetType::Native => build_send_native_asset_msg(
            env.contract.address,
            redeemer_address,
            asset_label.as_str(),
            redeem_amount,
        ),
        AssetType::Cw20 => {
            let token_contract_addr = deps
                .api
                .human_address(&CanonicalAddr::from(asset_reference))?;
            build_send_cw20_token_msg(redeemer_address, token_contract_addr, redeem_amount)?
        }
    };

    messages.push(redeem_msg);

    Ok(HandleResponse {
        messages,
        log,
        data: None,
    })
}

/// Initialize asset if not exist.
/// Initialization requires that all params are provided and there is no asset in state.
pub fn handle_init_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset: Asset,
    asset_params: InitOrUpdateAssetParams,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    let sender_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    if sender_canonical_address != config.owner {
        return Err(StdError::unauthorized());
    }

    let mut money_market = money_market_state_read(&deps.storage).load()?;

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(deps, &asset)?;
    let mut reserves = reserves_state(&mut deps.storage);
    let reserve_option = reserves.may_load(asset_reference.as_slice())?;
    match reserve_option {
        None => {
            let reserve_idx = money_market.reserve_count;
            let new_reserve =
                Reserve::create(env.block.time, reserve_idx, asset_type, asset_params)?;

            // Save new reserve
            reserves.save(asset_reference.as_slice(), &new_reserve)?;

            // Save index to reference mapping
            reserve_references_state(&mut deps.storage).save(
                &reserve_idx.to_be_bytes(),
                &ReserveReferences {
                    reference: asset_reference.to_vec(),
                },
            )?;

            // Increment reserve count
            money_market.reserve_count += 1;
            money_market_state(&mut deps.storage).save(&money_market)?;

            let symbol = match asset {
                Asset::Native { denom } => denom,
                Asset::Cw20 { contract_addr } => cw20_get_symbol(&deps.querier, contract_addr)?,
            };

            // Prepare response, should instantiate an maToken
            // and use the Register hook.
            // A new maToken should be created which callbacks this contract in order to be registered.
            Ok(HandleResponse {
                log: vec![log("action", "init_asset"), log("asset", asset_label)],
                data: None,
                messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                    code_id: config.ma_token_code_id,
                    msg: to_binary(&cw20_token::msg::InitMsg {
                        name: format!("mars {} debt token", symbol),
                        symbol: format!("ma{}", symbol),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: HumanAddr::from(env.contract.address.as_str()),
                            cap: None,
                        }),
                        init_hook: Some(cw20_token::msg::InitHook {
                            msg: to_binary(&HandleMsg::InitAssetTokenCallback {
                                reference: asset_reference,
                            })?,
                            contract_addr: env.contract.address,
                        }),
                    })?,
                    send: vec![],
                    label: None,
                })],
            })
        }
        Some(_) => Err(StdError::generic_err("Asset already initialized")),
    }
}

/// Update asset with new params.
pub fn handle_update_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset: Asset,
    asset_params: InitOrUpdateAssetParams,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    let sender_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    if sender_canonical_address != config.owner {
        return Err(StdError::unauthorized());
    }

    let (asset_label, asset_reference, _asset_type) = asset_get_attributes(deps, &asset)?;
    let mut reserves = reserves_state(&mut deps.storage);
    let reserve_option = reserves.may_load(asset_reference.as_slice())?;
    match reserve_option {
        Some(reserve) => {
            let updated_reserve = reserve.update_with(asset_params)?;

            // Save updated reserve
            reserves.save(asset_reference.as_slice(), &updated_reserve)?;

            Ok(HandleResponse {
                log: vec![log("action", "update_asset"), log("asset", asset_label)],
                data: None,
                messages: vec![],
            })
        }
        None => Err(StdError::generic_err("Asset not initialized")),
    }
}

pub fn init_asset_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    reference: Vec<u8>,
) -> StdResult<HandleResponse> {
    let mut state = reserves_state(&mut deps.storage);
    let mut reserve = state.load(reference.as_slice())?;

    if reserve.ma_token_address == CanonicalAddr::default() {
        let ma_contract_canonical_addr = deps.api.canonical_address(&env.message.sender)?;

        reserve.ma_token_address = ma_contract_canonical_addr.clone();
        state.save(reference.as_slice(), &reserve)?;

        // save ma token contract to reference mapping
        reserve_ma_tokens_state(&mut deps.storage)
            .save(ma_contract_canonical_addr.as_slice(), &reference)?;

        Ok(HandleResponse::default())
    } else {
        // Can do this only once
        Err(StdError::unauthorized())
    }
}

/// Handle deposits and mint corresponding debt tokens
pub fn handle_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    depositor_address: HumanAddr,
    asset_reference: &[u8],
    asset_label: &str,
    deposit_amount: Uint256,
) -> StdResult<HandleResponse> {
    let mut reserve = reserves_state_read(&deps.storage).load(asset_reference)?;

    // Cannot deposit zero amount
    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            asset_label,
        )));
    }

    let depositor_canonical_addr = deps.api.canonical_address(&depositor_address)?;
    let mut user = users_state_read(&deps.storage)
        .may_load(depositor_canonical_addr.as_slice())?
        .unwrap_or_default();

    let has_deposited_asset = get_bit(user.collateral_assets, reserve.index)?;
    if !has_deposited_asset {
        set_bit(&mut user.collateral_assets, reserve.index)?;
        users_state(&mut deps.storage).save(depositor_canonical_addr.as_slice(), &user)?;
    }

    reserve_apply_accumulated_interests(&env, &mut reserve);
    reserve_update_interest_rates(&deps, &env, asset_reference, &mut reserve, Uint256::zero())?;
    reserves_state(&mut deps.storage).save(asset_reference, &reserve)?;

    if reserve.liquidity_index.is_zero() {
        return Err(StdError::generic_err("Cannot have 0 as liquidity index"));
    }
    let mint_amount = deposit_amount / reserve.liquidity_index;

    let mut log = vec![
        log("action", "deposit"),
        log("reserve", asset_label),
        log("user", depositor_address.as_str()),
        log("amount", deposit_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &reserve);

    Ok(HandleResponse {
        data: None,
        log,
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: depositor_address,
                amount: mint_amount.into(),
            })?,
        })],
    })
}

/// Add debt for the borrower and send the borrowed funds
pub fn handle_borrow<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset: Asset,
    borrow_amount: Uint256,
) -> StdResult<HandleResponse> {
    let borrower_address = env.message.sender.clone();

    let (asset_label, asset_reference, asset_type) = asset_get_attributes(deps, &asset)?;

    // Cannot borrow zero amount
    if borrow_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Borrow amount must be greater than 0 {}",
            asset_label,
        )));
    }

    let money_market = money_market_state_read(&deps.storage).load()?;
    let mut borrow_reserve =
        match reserves_state_read(&deps.storage).load(asset_reference.as_slice()) {
            Ok(borrow_reserve) => borrow_reserve,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "no borrow reserve exists with asset reference: {}",
                    String::from_utf8(asset_reference).expect("Found invalid UTF-8")
                )));
            }
        };
    let borrower_canonical_addr = deps.api.canonical_address(&borrower_address)?;

    let uncollateralized_loan_limits_bucket =
        uncollateralized_loan_limits_read(&deps.storage, asset_reference.as_slice());
    let uncollateralized_loan_limit = uncollateralized_loan_limits_bucket
        .may_load(borrower_canonical_addr.as_slice())?
        .unwrap_or_else(Uint128::zero);

    let mut user: User =
        match users_state_read(&deps.storage).may_load(borrower_canonical_addr.as_slice())? {
            Some(user) => user,
            None => {
                if uncollateralized_loan_limit.is_zero() {
                    return Err(StdError::generic_err("address has no collateral deposited"));
                }
                // If User has some uncollateralized_loan_limit, then we don't require an existing debt position and initialize a new one.
                User::default()
            }
        };

    // TODO: Check the contract has enough funds to safely lend them

    if uncollateralized_loan_limit.is_zero() {
        // Collateralized loan: validate user has enough collateral if they have no uncollateralized loan limit
        let mut native_asset_prices_to_query: Vec<String> = match asset {
            Asset::Native { .. } if asset_label != "uusd" => vec![asset_label.clone()],
            _ => vec![],
        };

        // Get debt and ltv values for user's position
        // Vec<(reference, debt_amount, max_borrow, asset_type)>
        let user_balances = user_get_balances(
            &deps,
            &money_market,
            &user,
            &borrower_canonical_addr,
            |collateral, reserve| collateral * reserve.loan_to_value,
            &mut native_asset_prices_to_query,
            env.block.time,
        )?;

        let asset_prices = get_native_asset_prices(&deps.querier, &native_asset_prices_to_query)?;

        let mut total_debt_in_uusd = Uint256::zero();
        let mut max_borrow_in_uusd = Decimal256::zero();

        for (asset_label, debt, max_borrow, asset_type) in user_balances {
            let asset_price = asset_get_price(asset_label.as_str(), &asset_prices, &asset_type)?;

            total_debt_in_uusd += debt * asset_price;
            max_borrow_in_uusd += max_borrow * asset_price;
        }

        let borrow_asset_price = asset_get_price(asset_label.as_str(), &asset_prices, &asset_type)?;
        let borrow_amount_in_uusd = borrow_amount * borrow_asset_price;

        if Decimal256::from_uint256(total_debt_in_uusd + borrow_amount_in_uusd) > max_borrow_in_uusd
        {
            return Err(StdError::generic_err(
                "borrow amount exceeds maximum allowed given current collateral value",
            ));
        }
    } else {
        // Uncollateralized loan: check borrow amount plus debt does not exceed uncollateralized loan limit
        let debts_asset_bucket = debts_asset_state(&mut deps.storage, asset_reference.as_slice());
        let borrower_debt: Debt =
            match debts_asset_bucket.may_load(borrower_canonical_addr.as_slice()) {
                Ok(Some(debt)) => debt,
                Ok(None) => Debt {
                    amount_scaled: Uint256::zero(),
                },
                Err(error) => return Err(error),
            };

        let asset_reserve = reserves_state_read(&deps.storage).load(asset_reference.as_slice())?;
        let debt_amount = borrower_debt.amount_scaled * asset_reserve.borrow_index;
        if borrow_amount + debt_amount > Uint256::from(uncollateralized_loan_limit) {
            return Err(StdError::generic_err(
                "borrow amount exceeds uncollateralized loan limit given existing debt",
            ));
        }
    }

    reserve_apply_accumulated_interests(&env, &mut borrow_reserve);

    // Set borrowing asset for user
    let is_borrowing_asset = get_bit(user.borrowed_assets, borrow_reserve.index)?;
    if !is_borrowing_asset {
        set_bit(&mut user.borrowed_assets, borrow_reserve.index)?;
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket.save(borrower_canonical_addr.as_slice(), &user)?;
    }

    // Set new debt
    let mut debts_asset_bucket = debts_asset_state(&mut deps.storage, asset_reference.as_slice());
    let mut debt: Debt = match debts_asset_bucket.may_load(borrower_canonical_addr.as_slice()) {
        Ok(Some(debt)) => debt,
        Ok(None) => Debt {
            amount_scaled: Uint256::zero(),
        },
        Err(error) => return Err(error),
    };
    let borrow_amount_scaled = borrow_amount / borrow_reserve.borrow_index;
    debt.amount_scaled += borrow_amount_scaled;
    debts_asset_bucket.save(borrower_canonical_addr.as_slice(), &debt)?;

    borrow_reserve.debt_total_scaled += borrow_amount_scaled;

    reserve_update_interest_rates(
        &deps,
        &env,
        asset_reference.as_slice(),
        &mut borrow_reserve,
        borrow_amount,
    )?;
    reserves_state(&mut deps.storage).save(&asset_reference.as_slice(), &borrow_reserve)?;

    let mut log = vec![
        log("action", "borrow"),
        log("reserve", asset_label.as_str()),
        log("user", borrower_address.as_str()),
        log("amount", borrow_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &borrow_reserve);

    // Send borrow amount to borrower
    let send_msg =
        build_send_asset_msg(env.contract.address, borrower_address, asset, borrow_amount)?;

    Ok(HandleResponse {
        data: None,
        log,
        messages: vec![send_msg],
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
    let mut reserve = reserves_state_read(&deps.storage).load(asset_reference)?;

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
    let mut debts_asset_bucket = debts_asset_state(&mut deps.storage, asset_reference);
    let mut debt = debts_asset_bucket.load(repayer_canonical_address.as_slice())?;

    if debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("Cannot repay 0 debt"));
    }

    reserve_apply_accumulated_interests(&env, &mut reserve);

    let mut repay_amount_scaled = repay_amount / reserve.borrow_index;

    let mut messages: Vec<CosmosMsg> = vec![];
    let mut refund_amount = Uint256::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        // TODO: Should we log this?
        refund_amount = (repay_amount_scaled - debt.amount_scaled) * reserve.borrow_index;
        let refund_msg = match asset_type {
            AssetType::Native => build_send_native_asset_msg(
                env.contract.address.clone(),
                repayer_address.clone(),
                asset_label,
                refund_amount,
            ),
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
    debts_asset_bucket.save(repayer_canonical_address.as_slice(), &debt)?;

    if repay_amount_scaled > reserve.debt_total_scaled {
        return Err(StdError::generic_err(
            "Amount to repay is greater than total debt",
        ));
    }
    reserve.debt_total_scaled = reserve.debt_total_scaled - repay_amount_scaled;
    reserve_update_interest_rates(&deps, &env, asset_reference, &mut reserve, Uint256::zero())?;
    reserves_state(&mut deps.storage).save(asset_reference, &reserve)?;

    if debt.amount_scaled == Uint256::zero() {
        // Remove asset from borrowed assets
        let mut users_bucket = users_state(&mut deps.storage);
        let mut user = users_bucket.load(repayer_canonical_address.as_slice())?;
        unset_bit(&mut user.borrowed_assets, reserve.index)?;
        users_bucket.save(repayer_canonical_address.as_slice(), &user)?;
    }

    let mut log = vec![
        log("action", "repay"),
        log("reserve", asset_label),
        log("user", repayer_address),
        log("amount", repay_amount - refund_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &reserve);

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

    let mut collateral_reserve =
        reserves_state_read(&deps.storage).load(collateral_asset_reference.as_slice())?;

    // check if user has available collateral in specified collateral asset to be liquidated
    let collateral_ma_address = deps
        .api
        .human_address(&collateral_reserve.ma_token_address)?;
    let user_collateral_balance = collateral_reserve.liquidity_index
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
    let (user_health_status, native_asset_prices) = user_get_health_status(
        &deps,
        &money_market,
        &user,
        &user_canonical_address,
        env.block.time,
    )?;

    let health_factor = match user_health_status {
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

    let mut debt_reserve =
        reserves_state_read(&deps.storage).load(debt_asset_reference.as_slice())?;

    // 3. Compute debt to repay and collateral to liquidate
    let collateral_price = asset_get_price(
        collateral_asset_label.as_str(),
        &native_asset_prices,
        &collateral_reserve.asset_type,
    )?;
    let debt_price = asset_get_price(
        debt_asset_label.as_str(),
        &native_asset_prices,
        &debt_reserve.asset_type,
    )?;

    reserve_apply_accumulated_interests(&env, &mut debt_reserve);

    let user_debt_asset_total_debt = user_debt.amount_scaled * debt_reserve.borrow_index;

    let (debt_amount_to_repay, collateral_amount_to_liquidate, refund_amount) =
        liquidation_compute_amounts(
            collateral_price,
            debt_price,
            config.close_factor,
            user_collateral_balance,
            collateral_reserve.liquidation_bonus,
            user_debt_asset_total_debt,
            sent_debt_asset_amount,
        );

    let mut messages: Vec<CosmosMsg> = vec![];
    // 4. Update collateral positions and reserve depending on whether the liquidator elects to
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
            get_bit(liquidator.collateral_assets, collateral_reserve.index)?;
        if !liquidator_is_using_as_collateral {
            set_bit(&mut liquidator.collateral_assets, collateral_reserve.index)?;
            users_state(&mut deps.storage)
                .save(liquidator_canonical_addr.as_slice(), &liquidator)?;
        }

        let collateral_amount_to_liquidate_scaled =
            collateral_amount_to_liquidate / collateral_reserve.liquidity_index;
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
            Asset::Native { denom } => {
                Uint256::from(
                    deps.querier
                        .query_balance(env.contract.address.clone(), denom.as_str())?
                        .amount,
                ) * collateral_reserve.liquidity_index
            }
            Asset::Cw20 {
                contract_addr: token_addr,
            } => {
                Uint256::from(cw20_get_balance(
                    &deps.querier,
                    token_addr,
                    env.contract.address.clone(),
                )?) * collateral_reserve.liquidity_index
            }
        };
        if contract_collateral_balance < collateral_amount_to_liquidate {
            return Err(StdError::generic_err(
                "contract does not have enough collateral liquidity to send back underlying asset",
            ));
        }

        // apply update collateral interest as liquidity is reduced
        reserve_apply_accumulated_interests(&env, &mut collateral_reserve);
        reserve_update_interest_rates(
            &deps,
            &env,
            collateral_asset_reference.as_slice(),
            &mut collateral_reserve,
            collateral_amount_to_liquidate,
        )?;

        let collateral_amount_to_liquidate_scaled =
            collateral_amount_to_liquidate / collateral_reserve.liquidity_index;

        let burn_ma_tokens_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: collateral_ma_address,
            msg: to_binary(&mars::ma_token::msg::HandleMsg::Burn {
                user: user_address.clone(),
                amount: collateral_amount_to_liquidate_scaled.into(),
            })?,
            send: vec![],
        });

        let send_underlying_asset_msg = build_send_asset_msg(
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
        unset_bit(&mut user.collateral_assets, collateral_reserve.index)?;
        users_state(&mut deps.storage).save(user_canonical_address.as_slice(), &user)?;
    }

    // 5. Update debt reserve and positions

    let debt_amount_to_repay_scaled = debt_amount_to_repay / debt_reserve.borrow_index;

    // update user and reserve debt
    let mut debts_asset_bucket =
        debts_asset_state(&mut deps.storage, debt_asset_reference.as_slice());
    let mut debt = debts_asset_bucket.load(user_canonical_address.as_slice())?;
    // NOTE: Should be > 0 as amount to repay is capped by the close factor
    debt.amount_scaled = debt.amount_scaled - debt_amount_to_repay_scaled;
    debts_asset_bucket.save(user_canonical_address.as_slice(), &debt)?;
    debt_reserve.debt_total_scaled = debt_reserve.debt_total_scaled - debt_amount_to_repay_scaled;

    reserve_update_interest_rates(
        deps,
        &env,
        debt_asset_reference.as_slice(),
        &mut debt_reserve,
        refund_amount,
    )?;

    // save reserves
    reserves_state(&mut deps.storage).save(&debt_asset_reference.as_slice(), &debt_reserve)?;
    reserves_state(&mut deps.storage)
        .save(&collateral_asset_reference.as_slice(), &collateral_reserve)?;

    // 6. Build response
    // refund sent amount in excess of actual debt amount to liquidate
    if refund_amount > Uint256::zero() {
        let refund_msg = build_send_asset_msg(
            env.contract.address,
            liquidator_address.clone(),
            debt_asset,
            refund_amount,
        )?;
        messages.push(refund_msg);
    }

    let mut log = vec![
        log("action", "liquidate"),
        log("collateral_reserve", collateral_asset_label),
        log("debt_reserve", debt_asset_label),
        log("user", user_address.as_str()),
        log("liquidator", liquidator_address.as_str()),
        log(
            "collateral_amount_liquidated",
            collateral_amount_to_liquidate,
        ),
        log("debt_amount_repaid", debt_amount_to_repay),
        log("refund_amount", refund_amount),
    ];

    // TODO: we should distinguish between collateral and reserve values in some way
    append_indices_and_rates_to_logs(&mut log, &debt_reserve);
    if !receive_ma_token {
        append_indices_and_rates_to_logs(&mut log, &collateral_reserve);
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
pub fn handle_finalize_liquidity_token_transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    from_address: HumanAddr,
    to_address: HumanAddr,
    from_previous_balance: Uint128,
    to_previous_balance: Uint128,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    // Get liquidity token reserve
    let reserve_reference = reserve_ma_tokens_state_read(&deps.storage)
        .load(deps.api.canonical_address(&env.message.sender)?.as_slice())?;
    let reserve = reserves_state_read(&deps.storage).load(&reserve_reference)?;

    // Check user health factor is above 1
    // TODO: this assumes new balances are already in state as this call will be made
    // after a transfer call on an ma_asset. Double check this is the case when doing
    // integration tests. If it's not we would need to pass the updated balances to
    // the health factor somehow
    let from_canonical_address = deps.api.canonical_address(&from_address)?;
    let money_market = money_market_state_read(&deps.storage).load()?;
    let mut from_user = users_state_read(&deps.storage).load(from_canonical_address.as_slice())?;
    let (user_health_status, _) = user_get_health_status(
        &deps,
        &money_market,
        &from_user,
        &from_canonical_address,
        env.block.time,
    )?;
    if let UserHealthStatus::Borrowing(health_factor) = user_health_status {
        if health_factor < Decimal256::one() {
            return Err(StdError::generic_err("Cannot make token transfer if it results in a helth factor lower than 1 for the sender"));
        }
    }

    // Update users's positions
    // TODO: Should this and all collateral positions changes be logged? how?
    if from_address != to_address {
        if (from_previous_balance - amount)? == Uint128::zero() {
            unset_bit(&mut from_user.collateral_assets, reserve.index)?;
            users_state(&mut deps.storage).save(from_canonical_address.as_slice(), &from_user)?;
        }

        if (to_previous_balance == Uint128::zero()) && (amount != Uint128::zero()) {
            let to_canonical_address = deps.api.canonical_address(&to_address)?;
            let mut users_bucket = users_state(&mut deps.storage);
            let mut to_user = users_bucket
                .may_load(&to_canonical_address.as_slice())?
                .unwrap_or_default();
            set_bit(&mut to_user.collateral_assets, reserve.index)?;
            users_bucket.save(to_canonical_address.as_slice(), &to_user)?;
        }
    }

    Ok(HandleResponse::default())
}

/// Update uncollateralized loan limit by a given amount in uusd
pub fn handle_update_uncollateralized_loan_limit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    user_address: HumanAddr,
    asset: Asset,
    new_limit: Uint128,
) -> StdResult<HandleResponse> {
    // Get config
    let config = config_state_read(&deps.storage).load()?;

    // Only owner can do this
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    let (asset_label, asset_reference, _) = asset_get_attributes(deps, &asset)?;
    let user_canonical_address = deps.api.canonical_address(&user_address)?;

    let mut uncollateralized_loan_limits_bucket =
        uncollateralized_loan_limits(&mut deps.storage, asset_reference.as_slice());

    uncollateralized_loan_limits_bucket.save(user_canonical_address.as_slice(), &new_limit)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "update_uncollateralized_loan_limit"),
            log("user", user_address),
            log("asset", asset_label),
            log("new_allowance", new_limit),
        ],
        data: None,
    })
}

/// Update (enable / disable) collateral asset for specific user
pub fn handle_update_user_collateral_asset_status<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    asset: Asset,
    enable: bool,
) -> StdResult<HandleResponse> {
    let user_address = env.message.sender;
    let user_canonical_address = deps.api.canonical_address(&user_address)?;
    let mut user = users_state_read(&deps.storage)
        .may_load(user_canonical_address.as_slice())
        .map(|u| u.unwrap_or_default())?;

    let (collateral_asset_label, collateral_asset_reference, _) =
        asset_get_attributes(deps, &asset)?;
    let collateral_reserve =
        reserves_state_read(&deps.storage).load(collateral_asset_reference.as_slice())?;
    let has_collateral_asset = get_bit(user.collateral_assets, collateral_reserve.index)?;
    if !has_collateral_asset && enable {
        let collateral_ma_address = deps
            .api
            .human_address(&collateral_reserve.ma_token_address)?;
        let user_collateral_balance =
            cw20_get_balance(&deps.querier, collateral_ma_address, user_address.clone())?;
        if user_collateral_balance > Uint128::zero() {
            // enable collateral asset
            set_bit(&mut user.collateral_assets, collateral_reserve.index)?;
            users_state(&mut deps.storage).save(user_canonical_address.as_slice(), &user)?;
        } else {
            return Err(StdError::generic_err(format!(
                "User address {} has no balance in specified collateral asset {}",
                user_address.as_str(),
                collateral_asset_label
            )));
        }
    } else if has_collateral_asset && !enable {
        // disable collateral asset
        unset_bit(&mut user.collateral_assets, collateral_reserve.index)?;
        users_state(&mut deps.storage).save(user_canonical_address.as_slice(), &user)?;
    }

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "update_user_collateral_asset_status"),
            log("user", user_address),
            log("asset", collateral_asset_label),
            log("has_collateral", has_collateral_asset),
            log("enable", enable),
        ],
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

    // Only owner can do this
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    let (asset_label, asset_reference, _) = asset_get_attributes(deps, &asset)?;
    let mut reserve = reserves_state_read(&deps.storage).load(asset_reference.as_slice())?;

    let amount_to_distribute = match amount {
        Some(amount) => amount,
        None => reserve.protocol_income_to_distribute,
    };

    if amount_to_distribute > reserve.protocol_income_to_distribute {
        return Err(StdError::generic_err(
            "amount specified exceeds reserve's income to be distributed",
        ));
    }

    reserve.protocol_income_to_distribute =
        reserve.protocol_income_to_distribute - amount_to_distribute;
    reserves_state(&mut deps.storage).save(&asset_reference.as_slice(), &reserve)?;

    let mut messages = vec![];

    let insurance_fund_address = deps
        .api
        .human_address(&config.insurance_fund_contract_address)?;
    let treasury_fund_address = deps.api.human_address(&config.treasury_contract_address)?;
    let staking_contract_address = deps.api.human_address(&config.staking_contract_address)?;

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
            env.contract.address.clone(),
            insurance_fund_address,
            asset.clone(),
            insurance_fund_amount,
        )?;
        messages.push(insurance_fund_msg);
    }

    if !treasury_amount.is_zero() {
        let scaled_mint_amount = treasury_amount / reserve.liquidity_index;
        let treasury_fund_msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: treasury_fund_address,
                amount: scaled_mint_amount.into(),
            })?,
        });
        messages.push(treasury_fund_msg);
    }

    if !staking_amount.is_zero() {
        let staking_msg = build_send_asset_msg(
            env.contract.address,
            staking_contract_address,
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
        QueryMsg::Reserve { asset } => to_binary(&query_reserve(deps, asset)?),
        QueryMsg::ReservesList {} => to_binary(&query_reserves_list(deps)?),
        QueryMsg::Debt { address } => to_binary(&query_debt(deps, address)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    let money_market = money_market_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        treasury_contract_address: deps.api.human_address(&config.treasury_contract_address)?,
        insurance_fund_contract_address: deps
            .api
            .human_address(&config.insurance_fund_contract_address)?,
        insurance_fund_fee_share: config.insurance_fund_fee_share,
        treasury_fee_share: config.treasury_fee_share,
        ma_token_code_id: config.ma_token_code_id,
        // TODO do we need this in ConfigResponse?
        reserve_count: money_market.reserve_count,
        close_factor: config.close_factor,
    })
}

fn query_reserve<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: Asset,
) -> StdResult<ReserveResponse> {
    let reserve = match asset {
        Asset::Native { denom } => {
            match reserves_state_read(&deps.storage).load(denom.as_bytes()) {
                Ok(reserve) => reserve,
                Err(_) => {
                    return Err(StdError::generic_err(format!(
                        "failed to load reserve for: {}",
                        denom
                    )))
                }
            }
        }
        Asset::Cw20 { contract_addr } => {
            let cw20_canonical_address = deps.api.canonical_address(&contract_addr)?;
            match reserves_state_read(&deps.storage).load(cw20_canonical_address.as_slice()) {
                Ok(reserve) => reserve,
                Err(_) => {
                    return Err(StdError::generic_err(format!(
                        "failed to load reserve for: {}",
                        contract_addr
                    )))
                }
            }
        }
    };

    Ok(ReserveResponse {
        ma_token_address: deps.api.human_address(&reserve.ma_token_address)?,
        borrow_index: reserve.borrow_index,
        liquidity_index: reserve.liquidity_index,
        borrow_rate: reserve.borrow_rate,
        liquidity_rate: reserve.liquidity_rate,
        borrow_slope: reserve.borrow_slope,
        loan_to_value: reserve.loan_to_value,
        interests_last_updated: reserve.interests_last_updated,
        debt_total_scaled: reserve.debt_total_scaled,
        asset_type: reserve.asset_type,
        liquidation_threshold: reserve.liquidation_threshold,
        liquidation_bonus: reserve.liquidation_bonus,
    })
}

fn query_reserves_list<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ReservesListResponse> {
    let reserves = reserves_state_read(&deps.storage);

    let reserves_list: StdResult<Vec<_>> = reserves
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;

            let denom = match v.asset_type {
                AssetType::Native => match String::from_utf8(k) {
                    Ok(denom) => denom,
                    Err(_) => {
                        return Err(StdError::generic_err("failed to encode key into string"))
                    }
                },
                AssetType::Cw20 => {
                    let cw20_contract_address =
                        match deps.api.human_address(&CanonicalAddr::from(k)) {
                            Ok(cw20_contract_address) => cw20_contract_address,
                            Err(_) => {
                                return Err(StdError::generic_err(
                                    "failed to encode key into canonical address",
                                ))
                            }
                        };

                    match cw20_get_symbol(&deps.querier, cw20_contract_address.clone()) {
                        Ok(symbol) => symbol,
                        Err(_) => {
                            return Err(StdError::generic_err(format!(
                                "failed to get symbol from cw20 contract address: {}",
                                cw20_contract_address
                            )));
                        }
                    }
                }
            };

            Ok(ReserveInfo {
                denom,
                ma_token_address: deps.api.human_address(&v.ma_token_address)?,
            })
        })
        .collect();

    Ok(ReservesListResponse {
        reserves_list: reserves_list?,
    })
}

fn query_debt<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<DebtResponse> {
    let reserves = reserves_state_read(&deps.storage);
    let debtor_address = deps.api.canonical_address(&address)?;
    let users_bucket = users_state_read(&deps.storage);
    let user = users_bucket
        .may_load(debtor_address.as_slice())?
        .unwrap_or_default();

    let debts: StdResult<Vec<_>> = reserves
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;

            let denom = match v.asset_type {
                AssetType::Native => match String::from_utf8(k.clone()) {
                    Ok(denom) => denom,
                    Err(_) => {
                        return Err(StdError::generic_err("failed to encode key into string"))
                    }
                },
                AssetType::Cw20 => {
                    let cw20_contract_address =
                        match deps.api.human_address(&CanonicalAddr::from(k.clone())) {
                            Ok(cw20_contract_address) => cw20_contract_address,
                            Err(_) => {
                                return Err(StdError::generic_err(
                                    "failed to encode key into canonical address",
                                ))
                            }
                        };

                    match cw20_get_symbol(&deps.querier, cw20_contract_address.clone()) {
                        Ok(symbol) => symbol,
                        Err(_) => {
                            return Err(StdError::generic_err(format!(
                                "failed to get symbol from cw20 contract address: {}",
                                cw20_contract_address
                            )));
                        }
                    }
                }
            };

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

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// INTEREST

/// Updates reserve indices and protocol_income by applying current interest rates on the time between
/// last interest update and current block.
/// Note it does not save the reserve to the store (that is left to the caller)
pub fn reserve_apply_accumulated_interests(env: &Env, reserve: &mut Reserve) {
    let current_timestamp = env.block.time;
    // Since interest is updated on every change on scale debt, multiplying the scaled debt for each
    // of the indices and subtracting them returns the accrued borrow interest for the period since
    // when the indices were last updated and the current point in time.
    let previous_borrow_index = reserve.borrow_index;

    if reserve.interests_last_updated < current_timestamp {
        let time_elapsed = current_timestamp - reserve.interests_last_updated;

        if reserve.borrow_rate > Decimal256::zero() {
            reserve.borrow_index = calculate_applied_linear_interest_rate(
                reserve.borrow_index,
                reserve.borrow_rate,
                time_elapsed,
            );
        }
        if reserve.liquidity_rate > Decimal256::zero() {
            reserve.liquidity_index = calculate_applied_linear_interest_rate(
                reserve.liquidity_index,
                reserve.liquidity_rate,
                time_elapsed,
            );
        }
        reserve.interests_last_updated = current_timestamp;
    }

    let previous_debt_total = reserve.debt_total_scaled * previous_borrow_index;
    let new_debt_total = reserve.debt_total_scaled * reserve.borrow_index;

    let interest_accrued = if new_debt_total > previous_debt_total {
        new_debt_total - previous_debt_total
    } else {
        Uint256::zero()
    };

    let new_protocol_income_to_distribute = interest_accrued * reserve.reserve_factor;
    reserve.protocol_income_to_distribute += new_protocol_income_to_distribute;
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
/// Note it does not save the reserve to the store (that is left to the caller)
pub fn reserve_update_interest_rates<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    env: &Env,
    reference: &[u8],
    reserve: &mut Reserve,
    liquidity_taken: Uint256,
) -> StdResult<()> {
    let contract_balance_amount = match reserve.asset_type {
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
        (Decimal256::one() - config.treasury_fee_share) * reserve.protocol_income_to_distribute;
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
    let total_debt = Decimal256::from_uint256(reserve.debt_total_scaled) * reserve.borrow_index;
    let utilization_rate = if total_debt > Decimal256::zero() {
        total_debt / (available_liquidity + total_debt)
    } else {
        Decimal256::zero()
    };

    reserve.borrow_rate = reserve.borrow_slope * utilization_rate;
    // This operation should not underflow as reserve_factor is checked to be <= 1
    reserve.liquidity_rate =
        reserve.borrow_rate * utilization_rate * (Decimal256::one() - reserve.reserve_factor);

    Ok(())
}

fn append_indices_and_rates_to_logs(logs: &mut Vec<LogAttribute>, reserve: &Reserve) {
    let mut interest_logs = vec![
        log("borrow_index", reserve.borrow_index),
        log("liquidity_index", reserve.liquidity_index),
        log("borrow_rate", reserve.borrow_rate),
        log("liquidity_rate", reserve.liquidity_rate),
    ];
    logs.append(&mut interest_logs);
}

/// Goes through assets user has a position in and returns a vec containing the scaled debt
/// (denominated in the asset), a result from a specified computation for the current collateral
/// (denominated in asset) and some metadata to be used by the caller.
/// Also adds the price to native_assets_prices_to_query in case the prices in uusd need to
/// be retrieved by the caller later
fn user_get_balances<S, A, Q, F>(
    deps: &Extern<S, A, Q>,
    money_market: &MoneyMarket,
    user: &User,
    user_canonical_address: &CanonicalAddr,
    get_target_collateral_amount: F,
    native_asset_prices_to_query: &mut Vec<String>,
    block_time: u64,
) -> StdResult<Vec<(String, Uint256, Decimal256, AssetType)>>
where
    S: Storage,
    A: Api,
    Q: Querier,
    F: Fn(Decimal256, &Reserve) -> Decimal256,
{
    let mut ret: Vec<(String, Uint256, Decimal256, AssetType)> = vec![];

    for i in 0_u32..money_market.reserve_count {
        let user_is_using_as_collateral = get_bit(user.collateral_assets, i)?;
        let user_is_borrowing = get_bit(user.borrowed_assets, i)?;
        if !(user_is_using_as_collateral || user_is_borrowing) {
            continue;
        }

        let (asset_reference_vec, reserve) = reserve_get_from_index(&deps.storage, i)?;

        let target_collateral_amount = if user_is_using_as_collateral {
            // query asset balance (ma_token contract gives back a scaled value)
            let asset_balance = cw20_get_balance(
                &deps.querier,
                deps.api.human_address(&reserve.ma_token_address)?,
                deps.api.human_address(user_canonical_address)?,
            )?;

            let liquidity_index = if reserve.interests_last_updated < block_time {
                let time_elapsed = block_time - reserve.interests_last_updated;
                calculate_applied_linear_interest_rate(
                    reserve.liquidity_index,
                    reserve.liquidity_rate,
                    time_elapsed,
                )
            } else {
                reserve.liquidity_index
            };

            let collateral =
                Decimal256::from_uint256(Uint256::from(asset_balance)) * liquidity_index;
            get_target_collateral_amount(collateral, &reserve)
        } else {
            Decimal256::zero()
        };

        let debt_amount = if user_is_borrowing {
            // query debt
            let debts_asset_bucket = debts_asset_state_read(&deps.storage, &asset_reference_vec);
            let user_debt: Debt = debts_asset_bucket.load(user_canonical_address.as_slice())?;

            let borrow_index = if reserve.interests_last_updated < block_time {
                let time_elapsed = block_time - reserve.interests_last_updated;
                calculate_applied_linear_interest_rate(
                    reserve.borrow_index,
                    reserve.borrow_rate,
                    time_elapsed,
                )
            } else {
                reserve.borrow_index
            };

            user_debt.amount_scaled * borrow_index
        } else {
            Uint256::zero()
        };

        // get asset label
        let asset_label = match reserve.asset_type {
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
        if reserve.asset_type == AssetType::Native && asset_label != "uusd" {
            native_asset_prices_to_query.push(asset_label.clone());
        }

        ret.push((
            asset_label,
            debt_amount,
            target_collateral_amount,
            reserve.asset_type,
        ));
    }

    Ok(ret)
}

enum UserHealthStatus {
    NotBorrowing,
    Borrowing(Decimal256),
}

/// Computes user health status and returns it among the list of prices that were
/// used during the computation
fn user_get_health_status<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    money_market: &MoneyMarket,
    user: &User,
    user_canonical_address: &CanonicalAddr,
    block_time: u64,
) -> StdResult<(UserHealthStatus, Vec<(String, Decimal256)>)> {
    let mut native_asset_prices_to_query: Vec<String> = vec![];
    // Get debt and weighted_liquidation_threshold values for user's position
    // Vec<(reference, debt_amount, weighted_liquidation_threshold, asset_type)>
    let user_balances = user_get_balances(
        &deps,
        &money_market,
        user,
        user_canonical_address,
        |collateral, reserve| collateral * reserve.liquidation_threshold,
        &mut native_asset_prices_to_query,
        block_time,
    )?;
    let native_asset_prices =
        get_native_asset_prices(&deps.querier, &native_asset_prices_to_query)?;

    let mut total_debt_in_uusd = Decimal256::zero();
    let mut weighted_liquidation_threshold_sum = Decimal256::zero();

    // calculate user's health factor
    for (asset_label, debt_amount_in_asset, weighted_liquidation_threshold_in_asset, asset_type) in
        user_balances
    {
        let asset_price = asset_get_price(asset_label.as_str(), &native_asset_prices, &asset_type)?;

        let weighted_liquidation_threshold_in_uusd =
            asset_price * weighted_liquidation_threshold_in_asset;
        weighted_liquidation_threshold_sum += weighted_liquidation_threshold_in_uusd;

        let debt_balance_in_uusd = asset_price * debt_amount_in_asset;
        total_debt_in_uusd += Decimal256::from_uint256(debt_balance_in_uusd);
    }

    // ensure user's total debt across all reserves is not zero
    if total_debt_in_uusd.is_zero() {
        return Ok((UserHealthStatus::NotBorrowing, native_asset_prices));
    }

    let health_factor = weighted_liquidation_threshold_sum / total_debt_in_uusd;
    Ok((
        UserHealthStatus::Borrowing(health_factor),
        native_asset_prices,
    ))
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
    sender_address: HumanAddr,
    recipient_address: HumanAddr,
    asset: Asset,
    amount: Uint256,
) -> StdResult<CosmosMsg> {
    match asset {
        Asset::Native { denom } => Ok(build_send_native_asset_msg(
            sender_address,
            recipient_address,
            denom.as_str(),
            amount,
        )),
        Asset::Cw20 { contract_addr } => {
            build_send_cw20_token_msg(recipient_address, contract_addr, amount)
        }
    }
}

fn build_send_native_asset_msg(
    sender: HumanAddr,
    recipient: HumanAddr,
    denom: &str,
    amount: Uint256,
) -> CosmosMsg {
    CosmosMsg::Bank(BankMsg::Send {
        from_address: sender,
        to_address: recipient,
        amount: vec![Coin {
            denom: denom.to_string(),
            amount: amount.into(),
        }],
    })
}

fn build_send_cw20_token_msg(
    recipient: HumanAddr,
    token_contract_address: HumanAddr,
    amount: Uint256,
) -> StdResult<CosmosMsg> {
    Ok(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: token_contract_address,
        msg: to_binary(&Cw20HandleMsg::Transfer {
            recipient,
            amount: amount.into(),
        })?,
        send: vec![],
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

fn asset_get_attributes<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    asset: &Asset,
) -> StdResult<(String, Vec<u8>, AssetType)> {
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

fn reserve_get_from_index<S: Storage>(storage: &S, index: u32) -> StdResult<(Vec<u8>, Reserve)> {
    let asset_reference_vec =
        match reserve_references_state_read(storage).load(&index.to_be_bytes()) {
            Ok(asset_reference_vec) => asset_reference_vec,
            Err(_) => {
                return Err(StdError::generic_err(format!(
                    "no reserve reference exists with index: {}",
                    index
                )))
            }
        }
        .reference;
    match reserves_state_read(storage).load(&asset_reference_vec) {
        Ok(asset_reserve) => Ok((asset_reference_vec, asset_reserve)),
        Err(_) => Err(StdError::generic_err(format!(
            "no asset reserve exists with asset reference: {}",
            String::from_utf8(asset_reference_vec).expect("Found invalid UTF-8")
        ))),
    }
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{debts_asset_state_read, users_state_read};
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{coin, from_binary, Decimal, Extern};
    use mars::liquidity_pool::msg::HandleMsg::UpdateConfig;
    use mars::testing::{mock_dependencies, MarsMockQuerier, MockEnvParams};

    #[test]
    fn test_accumulated_index_calculation() {
        let index = Decimal256::from_ratio(1, 10);
        let rate = Decimal256::from_ratio(2, 10);
        let time_elapsed = 15768000; // half a year
        let accumulated = calculate_applied_linear_interest_rate(index, rate, time_elapsed);

        assert_eq!(accumulated, Decimal256::from_ratio(11, 100));
    }

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        // Config with base params valid (just update the rest)
        let base_config = CreateOrUpdateConfig {
            treasury_contract_address: Some(HumanAddr::from("treasury_contract")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_contract")),
            staking_contract_address: Some(HumanAddr::from("staking_contract")),
            ma_token_code_id: Some(10u64),
            insurance_fund_fee_share: None,
            treasury_fee_share: None,
            close_factor: None,
        };

        // *
        // init config with empty params
        // *
        let empty_config = CreateOrUpdateConfig {
            treasury_contract_address: None,
            insurance_fund_contract_address: None,
            staking_contract_address: None,
            insurance_fund_fee_share: None,
            treasury_fee_share: None,
            ma_token_code_id: None,
            close_factor: None,
        };
        let msg = InitMsg {
            config: empty_config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let res_error = init(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "All params should be available during initialization")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

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
        let res = init(&mut deps, env, msg);
        match res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "[close_factor, insurance_fund_fee_share, treasury_fee_share] should be less or equal 1. \
                Invalid params: [close_factor, insurance_fund_fee_share, treasury_fee_share]"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

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
        let res = init(&mut deps, env.clone(), exceeding_fees_msg);
        match res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

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
        assert_eq!(0, value.reserve_count);
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
            treasury_contract_address: Some(HumanAddr::from("treasury_contract")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_contract")),
            staking_contract_address: Some(HumanAddr::from("staking_contract")),
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
            owner: None,
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
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
            ..init_config.clone()
        };
        let msg = UpdateConfig {
            owner: None,
            config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let res = handle(&mut deps, env, msg);
        match res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "[close_factor, insurance_fund_fee_share, treasury_fee_share] should be less or equal 1. \
                Invalid params: [close_factor, insurance_fund_fee_share, treasury_fee_share]"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // update config with invalid fee share amounts
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(10, 10);
        let config = CreateOrUpdateConfig {
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: None,
            ..init_config
        };
        let exceeding_fees_msg = UpdateConfig {
            owner: None,
            config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let res = handle(&mut deps, env.clone(), exceeding_fees_msg);
        match res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "Invalid fee share amounts. Sum of insurance and treasury fee shares exceed one"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // update config with all new params
        // *
        insurance_fund_fee_share = Decimal256::from_ratio(5, 100);
        treasury_fee_share = Decimal256::from_ratio(3, 100);
        close_factor = Decimal256::from_ratio(1, 20);
        let config = CreateOrUpdateConfig {
            treasury_contract_address: Some(HumanAddr::from("treasury_addr")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_addr")),
            staking_contract_address: Some(HumanAddr::from("staking_addr")),
            ma_token_code_id: Some(40u64),
            insurance_fund_fee_share: Some(insurance_fund_fee_share),
            treasury_fee_share: Some(treasury_fee_share),
            close_factor: Some(close_factor),
        };
        let msg = UpdateConfig {
            owner: Some(HumanAddr::from("new_owner")),
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
            new_config.treasury_contract_address,
            deps.api
                .canonical_address(&config.treasury_contract_address.unwrap())
                .unwrap()
        );
        assert_eq!(
            new_config.insurance_fund_contract_address,
            deps.api
                .canonical_address(&config.insurance_fund_contract_address.unwrap())
                .unwrap()
        );
        assert_eq!(
            new_config.staking_contract_address,
            deps.api
                .canonical_address(&config.staking_contract_address.unwrap())
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
            treasury_contract_address: Some(HumanAddr::from("treasury_contract")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_fund")),
            staking_contract_address: Some(HumanAddr::from("staking_contract")),
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
            borrow_slope: Some(Decimal256::from_ratio(4, 100)),
            loan_to_value: Some(Decimal256::from_ratio(8, 10)),
            reserve_factor: Some(Decimal256::from_ratio(1, 100)),
            liquidation_threshold: Some(Decimal256::one()),
            liquidation_bonus: Some(Decimal256::zero()),
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
            loan_to_value: None,
            liquidation_threshold: None,
            liquidation_bonus: None,
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "All params should be available during initialization",)
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // init asset with some params greater than 1
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            loan_to_value: Some(Decimal256::from_ratio(110, 10)),
            reserve_factor: Some(Decimal256::from_ratio(120, 100)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "[loan_to_value, reserve_factor, liquidation_threshold, liquidation_bonus] should be less or equal 1. \
                Invalid params: [loan_to_value, reserve_factor]",
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // init asset where LTV >= liquidity threshold
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            loan_to_value: Some(Decimal256::from_ratio(5, 10)),
            liquidation_threshold: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = HandleMsg::InitAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "liquidation_threshold should be greater than loan_to_value. \
                    old_liquidation_threshold: 0, \
                    old_loan_to_value: 0, \
                    new_liquidation_threshold: 0.5, \
                    new_loan_to_value: 0.5",
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

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

        // should have asset reserve with Canonical default address
        let reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(CanonicalAddr::default(), reserve.ma_token_address);
        // should have 0 index
        assert_eq!(0, reserve.index);
        // should have asset_type Native
        assert_eq!(AssetType::Native, reserve.asset_type);

        // should store reference in reserve index
        let reserve_reference = reserve_references_state_read(&deps.storage)
            .load(&0_u32.to_be_bytes())
            .unwrap();
        assert_eq!(b"someasset", reserve_reference.reference.as_slice());

        // Should have reserve count of 1
        let money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(money_market.reserve_count, 1);

        // should instantiate a debt token
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 5u64,
                msg: to_binary(&cw20_token::msg::InitMsg {
                    name: String::from("mars someasset debt token"),
                    symbol: String::from("masomeasset"),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        cap: None,
                    }),
                    init_hook: Some(cw20_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitAssetTokenCallback {
                            reference: "someasset".into(),
                        })
                        .unwrap(),
                        contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    }),
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
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Asset already initialized",),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // callback comes back with created token
        // *
        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: "someasset".into(),
        };
        handle(&mut deps, env, msg).unwrap();

        // should have asset reserve with contract address
        let reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mtokencontract"))
                .unwrap(),
            reserve.ma_token_address
        );
        assert_eq!(Decimal256::one(), reserve.liquidity_index);

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

        let reserve = reserves_state_read(&deps.storage)
            .load(&cw20_canonical_addr.as_slice())
            .unwrap();
        // should have asset reserve with Canonical default address
        assert_eq!(CanonicalAddr::default(), reserve.ma_token_address);
        // should have index 1
        assert_eq!(1, reserve.index);
        // should have asset_type Cw20
        assert_eq!(AssetType::Cw20, reserve.asset_type);

        // should store reference in reserve index
        let reserve_reference = reserve_references_state_read(&deps.storage)
            .load(&1_u32.to_be_bytes())
            .unwrap();
        assert_eq!(
            cw20_canonical_addr.as_slice(),
            reserve_reference.reference.as_slice()
        );

        // should have an asset_type of cw20
        assert_eq!(AssetType::Cw20, reserve.asset_type);

        // Should have reserve count of 2
        let money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(2, money_market.reserve_count);

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

        // should have asset reserve with contract address
        let reserve = reserves_state_read(&deps.storage)
            .load(cw20_canonical_addr.as_slice())
            .unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mtokencontract"))
                .unwrap(),
            reserve.ma_token_address
        );
        assert_eq!(Decimal256::one(), reserve.liquidity_index);

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
            treasury_contract_address: Some(HumanAddr::from("treasury_contract")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_fund")),
            staking_contract_address: Some(HumanAddr::from("staking_contract")),
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
            borrow_slope: Some(Decimal256::from_ratio(4, 100)),
            loan_to_value: Some(Decimal256::from_ratio(50, 100)),
            reserve_factor: Some(Decimal256::from_ratio(1, 100)),
            liquidation_threshold: Some(Decimal256::from_ratio(80, 100)),
            liquidation_bonus: Some(Decimal256::from_ratio(10, 100)),
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
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Asset not initialized",),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

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
            liquidation_threshold: Some(Decimal256::from_ratio(110, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "[loan_to_value, reserve_factor, liquidation_threshold, liquidation_bonus] should be less or equal 1. \
                Invalid params: [liquidation_threshold]",
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // update asset where LTV >= liquidity threshold
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let invalid_asset_params = InitOrUpdateAssetParams {
            loan_to_value: Some(Decimal256::from_ratio(6, 10)),
            liquidation_threshold: Some(Decimal256::from_ratio(5, 10)),
            ..asset_params
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: invalid_asset_params,
        };
        let res_error = handle(&mut deps, env, msg);
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "liquidation_threshold should be greater than loan_to_value. \
                    old_liquidation_threshold: 0.8, \
                    old_loan_to_value: 0.5, \
                    new_liquidation_threshold: 0.5, \
                    new_loan_to_value: 0.6",
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // update asset with new params
        // *
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let asset_params = InitOrUpdateAssetParams {
            borrow_slope: Some(Decimal256::from_ratio(40, 100)),
            loan_to_value: Some(Decimal256::from_ratio(60, 100)),
            reserve_factor: Some(Decimal256::from_ratio(10, 100)),
            liquidation_threshold: Some(Decimal256::from_ratio(90, 100)),
            liquidation_bonus: Some(Decimal256::from_ratio(12, 100)),
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: asset_params.clone(),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let new_reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(0, new_reserve.index);
        assert_eq!(asset_params.borrow_slope.unwrap(), new_reserve.borrow_slope);
        assert_eq!(
            asset_params.loan_to_value.unwrap(),
            new_reserve.loan_to_value
        );
        assert_eq!(
            asset_params.reserve_factor.unwrap(),
            new_reserve.reserve_factor
        );
        assert_eq!(
            asset_params.liquidation_threshold.unwrap(),
            new_reserve.liquidation_threshold
        );
        assert_eq!(
            asset_params.liquidation_bonus.unwrap(),
            new_reserve.liquidation_bonus
        );

        let new_reserve_reference = reserve_references_state_read(&deps.storage)
            .load(&0_u32.to_be_bytes())
            .unwrap();
        assert_eq!(b"someasset", new_reserve_reference.reference.as_slice());

        let new_money_market = money_market_state_read(&deps.storage).load().unwrap();
        assert_eq!(new_money_market.reserve_count, 1);

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
            borrow_slope: None,
            loan_to_value: None,
            reserve_factor: None,
            liquidation_threshold: None,
            liquidation_bonus: None,
        };
        let msg = HandleMsg::UpdateAsset {
            asset: Asset::Native {
                denom: "someasset".to_string(),
            },
            asset_params: empty_asset_params,
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let new_reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(0, new_reserve.index);
        // should keep old params
        assert_eq!(asset_params.borrow_slope.unwrap(), new_reserve.borrow_slope);
        assert_eq!(
            asset_params.loan_to_value.unwrap(),
            new_reserve.loan_to_value
        );
        assert_eq!(
            asset_params.reserve_factor.unwrap(),
            new_reserve.reserve_factor
        );
        assert_eq!(
            asset_params.liquidation_threshold.unwrap(),
            new_reserve.liquidation_threshold
        );
        assert_eq!(
            asset_params.liquidation_bonus.unwrap(),
            new_reserve.liquidation_bonus
        );
    }

    #[test]
    fn test_init_asset_callback_cannot_be_called_on_its_own() {
        let mut deps = th_setup(&[]);

        let env = cosmwasm_std::testing::mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            reference: "uluna".into(),
        };
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(
            error_res,
            StdError::not_found("liquidity_pool::state::Reserve")
        );
    }

    #[test]
    fn test_deposit_native_asset() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "somecoin")]);
        let reserve_factor = Decimal256::from_ratio(1, 10);

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(11, 10),
            loan_to_value: Decimal256::one(),
            borrow_index: Decimal256::from_ratio(1, 1),
            borrow_slope: Decimal256::from_ratio(1, 10),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor,
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            ..Default::default()
        };
        let reserve = th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

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

        let expected_liquidity_index = calculate_applied_linear_interest_rate(
            Decimal256::from_ratio(11, 10),
            Decimal256::from_ratio(10, 100),
            100u64,
        );
        let expected_mint_amount =
            (Uint256::from(deposit_amount) / expected_liquidity_index).into();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &reserve,
            env.block.time,
            initial_liquidity,
            Default::default(),
        );

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
                log("reserve", "somecoin"),
                log("user", "depositor"),
                log("amount", deposit_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        let reserve = reserves_state_read(&deps.storage)
            .load(b"somecoin")
            .unwrap();
        // BR = U * Bslope = 0.5 * 0.01 = 0.05
        assert_eq!(reserve.borrow_rate, Decimal256::from_ratio(5, 100));
        // LR = BR * U = 0.05 * 0.5 * (1 - reserve_factor)= 0.025 * .9 = .0225
        assert_eq!(reserve.liquidity_rate, Decimal256::from_ratio(225, 10000));
        assert_eq!(reserve.liquidity_index, expected_liquidity_index);
        assert_eq!(reserve.borrow_index, Decimal256::from_ratio(1, 1));
        assert_eq!(
            reserve.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let env = cosmwasm_std::testing::mock_env("depositor", &[]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_deposit_cw20() {
        let initial_liquidity = 10_000_000;
        let mut deps = th_setup(&[]);

        let cw20_addr = HumanAddr::from("somecontract");
        let contract_addr_raw = deps.api.canonical_address(&cw20_addr).unwrap();

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(11, 10),
            loan_to_value: Decimal256::one(),
            borrow_index: Decimal256::from_ratio(1, 1),
            borrow_slope: Decimal256::from_ratio(1, 10),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(4, 100),
            debt_total_scaled: Uint256::from(10_000_000u128),
            interests_last_updated: 10_000_000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let reserve = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            contract_addr_raw.as_slice(),
            &mock_reserve,
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

        let expected_liquidity_index = calculate_applied_linear_interest_rate(
            Decimal256::from_ratio(11, 10),
            Decimal256::from_ratio(10, 100),
            100u64,
        );
        let expected_mint_amount: Uint256 =
            Uint256::from(deposit_amount) / expected_liquidity_index;

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &reserve,
            env.block.time,
            initial_liquidity,
            Default::default(),
        );

        let reserve = reserves_state_read(&deps.storage)
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
                log("reserve", cw20_addr),
                log("user", "depositor"),
                log("amount", deposit_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );
        assert_eq!(
            reserve.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );

        // empty deposit fails
        let env = cosmwasm_std::testing::mock_env("depositor", &[]);
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::DepositCw20 {}).unwrap()),
            sender: HumanAddr::from("depositor"),
            amount: Uint128(deposit_amount),
        });
        handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_cannot_deposit_if_no_reserve() {
        let mut deps = th_setup(&[]);

        let env = cosmwasm_std::testing::mock_env("depositer", &[coin(110000, "somecoin")]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_redeem_native() {
        // Redeem native token
        let initial_available_liquidity = 12000000u128;
        let mut deps = th_setup(&[coin(initial_available_liquidity, "somecoin")]);

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal256::from_ratio(2, 1),
            borrow_slope: Decimal256::from_ratio(1, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let burn_amount = 20000u128;
        let seconds_elapsed = 2000u64;

        let reserve_initial =
            th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);
        reserve_ma_tokens_state(&mut deps.storage)
            .save(
                deps.api
                    .canonical_address(&HumanAddr::from("matoken"))
                    .unwrap()
                    .as_slice(),
                &(b"somecoin".to_vec()),
            )
            .unwrap();

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Redeem {}).unwrap()),
            sender: HumanAddr::from("redeemer"),
            amount: Uint128(burn_amount),
        });

        let env = mars::testing::mock_env(
            "matoken",
            MockEnvParams {
                sent_funds: &[],
                block_time: mock_reserve.interests_last_updated + seconds_elapsed,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let reserve = reserves_state_read(&deps.storage)
            .load(b"somecoin")
            .unwrap();
        let unscaled_burn_amount: Uint128 =
            (Uint256::from(burn_amount) * reserve.liquidity_index).into();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &reserve_initial,
            mock_reserve.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: unscaled_burn_amount.into(),
                ..Default::default()
            },
        );

        let expected_asset_amount: Uint128 =
            (Uint256::from(burn_amount) * expected_params.liquidity_index).into();

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("matoken"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: Uint128(burn_amount),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("redeemer"),
                    amount: vec![Coin {
                        denom: String::from("somecoin"),
                        amount: expected_asset_amount,
                    }],
                }),
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "redeem"),
                log("reserve", "somecoin"),
                log("user", "redeemer"),
                log("burn_amount", burn_amount),
                log("redeem_amount", expected_asset_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        // BR = U * Bslope = 0.5 * 0.01 = 0.05
        assert_eq!(reserve.borrow_rate, expected_params.borrow_rate);
        assert_eq!(reserve.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(reserve.liquidity_index, expected_params.liquidity_index);
        assert_eq!(reserve.borrow_index, expected_params.borrow_index);
        assert_eq!(
            reserve.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn test_redeem_cw20() {
        // Redeem cw20 token
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
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                Uint128(initial_available_liquidity),
            )],
        );

        let initial_liquidity_index = Decimal256::from_ratio(15, 10);
        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: initial_liquidity_index,
            borrow_index: Decimal256::from_ratio(2, 1),
            borrow_slope: Decimal256::from_ratio(1, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(2, 100),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let burn_amount = 20000u128;
        let seconds_elapsed = 2000u64;

        let reserve_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            cw20_contract_canonical_addr.as_slice(),
            &mock_reserve,
        );
        reserve_ma_tokens_state(&mut deps.storage)
            .save(
                deps.api
                    .canonical_address(&ma_token_addr)
                    .unwrap()
                    .as_slice(),
                &cw20_contract_canonical_addr.as_slice().to_vec(),
            )
            .unwrap();

        let redeemer_addr = HumanAddr::from("redeemer");
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Redeem {}).unwrap()),
            sender: redeemer_addr.clone(),
            amount: Uint128(burn_amount),
        });

        let env = mars::testing::mock_env(
            "matoken",
            MockEnvParams {
                sent_funds: &[],
                block_time: mock_reserve.interests_last_updated + seconds_elapsed,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let reserve = reserves_state_read(&deps.storage)
            .load(cw20_contract_canonical_addr.as_slice())
            .unwrap();

        let unscaled_burn_amount: Uint128 =
            (Uint256::from(burn_amount) * reserve.liquidity_index).into();

        let expected_params = th_get_expected_indices_and_rates(
            &deps,
            &reserve_initial,
            mock_reserve.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: unscaled_burn_amount.into(),
                ..Default::default()
            },
        );

        let expected_asset_amount: Uint128 =
            (Uint256::from(burn_amount) * expected_params.liquidity_index).into();

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: ma_token_addr,
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: Uint128(burn_amount),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: cw20_contract_addr,
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: redeemer_addr,
                        amount: expected_asset_amount,
                    })
                    .unwrap(),
                    send: vec![],
                }),
            ]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "redeem"),
                log("reserve", "somecontract"),
                log("user", "redeemer"),
                log("burn_amount", burn_amount),
                log("redeem_amount", expected_asset_amount),
                log("borrow_index", expected_params.borrow_index),
                log("liquidity_index", expected_params.liquidity_index),
                log("borrow_rate", expected_params.borrow_rate),
                log("liquidity_rate", expected_params.liquidity_rate),
            ]
        );

        // BR = U * Bslope
        assert_eq!(reserve.borrow_rate, expected_params.borrow_rate);
        assert_eq!(reserve.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(reserve.liquidity_index, expected_params.liquidity_index);
        assert_eq!(reserve.borrow_index, expected_params.borrow_index);
        assert_eq!(
            reserve.protocol_income_to_distribute,
            expected_params.protocol_income_to_distribute
        );
    }

    #[test]
    fn redeem_cannot_exceed_balance() {
        let mut deps = th_setup(&[]);

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(15, 10),
            ..Default::default()
        };

        th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Redeem {}).unwrap()),
            sender: HumanAddr::from("redeemer"),
            amount: Uint128(2000),
        });

        let env = cosmwasm_std::testing::mock_env("matoken", &[]);
        handle(&mut deps, env, msg).unwrap_err();
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

        let mock_reserve_1 = MockReserve {
            ma_token_address: "matoken1",
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_slope: Decimal256::from_ratio(1, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_reserve_2 = MockReserve {
            ma_token_address: "matoken2",
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_reserve_3 = MockReserve {
            ma_token_address: "matoken3",
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::from_ratio(11, 10),
            loan_to_value: Decimal256::from_ratio(7, 10),
            borrow_slope: Decimal256::from_ratio(4, 10),
            borrow_rate: Decimal256::from_ratio(30, 100),
            reserve_factor: Decimal256::from_ratio(3, 100),
            liquidity_rate: Decimal256::from_ratio(20, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let reserve_1_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_reserve_1,
        );
        // should get index 1
        let reserve_2_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoinnative",
            &mock_reserve_2,
        );
        // should get index 2
        let reserve_collateral = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin",
            &mock_reserve_3,
        );

        let borrower_addr = HumanAddr::from("borrower");
        let borrower_canonical_addr = deps.api.canonical_address(&borrower_addr).unwrap();

        // Set user as having the reserve_collateral deposited
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, reserve_collateral.index).unwrap();
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
        let block_time = mock_reserve_1.interests_last_updated + 10000u64;
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
            &reserve_1_initial,
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
                log("reserve", "borrowedcoincw20"),
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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt = debts_asset_state_read(&deps.storage, cw20_contract_addr_canonical.as_slice())
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let expected_debt_scaled_1_after_borrow =
            Uint256::from(borrow_amount) / expected_params_cw20.borrow_index;

        let reserve_1_after_borrow = reserves_state_read(&deps.storage)
            .load(cw20_contract_addr_canonical.as_slice())
            .unwrap();

        assert_eq!(expected_debt_scaled_1_after_borrow, debt.amount_scaled);
        assert_eq!(
            expected_debt_scaled_1_after_borrow,
            reserve_1_after_borrow.debt_total_scaled
        );
        assert_eq!(
            expected_params_cw20.borrow_rate,
            reserve_1_after_borrow.borrow_rate
        );
        assert_eq!(
            expected_params_cw20.liquidity_rate,
            reserve_1_after_borrow.liquidity_rate
        );

        // *
        // Borrow cw20 token (again)
        // *
        let borrow_amount = 1200u128;
        let block_time = reserve_1_after_borrow.interests_last_updated + 20000u64;

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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps,
            &reserve_1_after_borrow,
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
        let reserve_1_after_borrow_again = reserves_state_read(&deps.storage)
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
            reserve_1_after_borrow_again.debt_total_scaled
        );
        assert_eq!(
            expected_params_cw20.borrow_rate,
            reserve_1_after_borrow_again.borrow_rate
        );
        assert_eq!(
            expected_params_cw20.liquidity_rate,
            reserve_1_after_borrow_again.liquidity_rate
        );

        // *
        // Borrow native coin
        // *

        let borrow_amount = 4000u128;
        let block_time = reserve_1_after_borrow_again.interests_last_updated + 3000u64;
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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_native = th_get_expected_indices_and_rates(
            &deps,
            &reserve_2_initial,
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
                amount: vec![Coin {
                    denom: String::from("borrowedcoinnative"),
                    amount: borrow_amount.into(),
                }],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("reserve", "borrowedcoinnative"),
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
        let reserve_2_after_borrow_2 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoinnative")
            .unwrap();

        let expected_debt_scaled_2_after_borrow_2 =
            Uint256::from(borrow_amount) / expected_params_native.borrow_index;
        assert_eq!(expected_debt_scaled_2_after_borrow_2, debt2.amount_scaled);
        assert_eq!(
            expected_debt_scaled_2_after_borrow_2,
            reserve_2_after_borrow_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_native.borrow_rate,
            reserve_2_after_borrow_2.borrow_rate
        );
        assert_eq!(
            expected_params_native.liquidity_rate,
            reserve_2_after_borrow_2.liquidity_rate
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
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        match res_error {
            StdError::GenericErr { msg, .. } => assert_eq!(
                "borrow amount exceeds maximum allowed given current collateral value",
                msg
            ),
            e => panic!("Unexpected error: {}", e),
        }

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
        handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay some native debt
        // *
        let repay_amount = 2000u128;
        let block_time = reserve_2_after_borrow_2.interests_last_updated + 8000u64;
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
            &reserve_2_after_borrow_2,
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
                log("reserve", "borrowedcoinnative"),
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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoinnative")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let reserve_2_after_repay_some_2 = reserves_state_read(&deps.storage)
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
            reserve_2_after_repay_some_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_native.borrow_rate,
            reserve_2_after_repay_some_2.borrow_rate
        );
        assert_eq!(
            expected_params_native.liquidity_rate,
            reserve_2_after_repay_some_2.liquidity_rate
        );

        // *
        // Repay all native debt
        // *
        let block_time = reserve_2_after_repay_some_2.interests_last_updated + 10000u64;
        // need this to compute the repay amount
        let expected_params_native = th_get_expected_indices_and_rates(
            &deps,
            &reserve_2_after_repay_some_2,
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
                log("reserve", "borrowedcoinnative"),
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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoinnative")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let reserve_2_after_repay_all_2 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoinnative")
            .unwrap();

        assert_eq!(Uint256::zero(), debt2.amount_scaled);
        assert_eq!(
            Uint256::zero(),
            reserve_2_after_repay_all_2.debt_total_scaled
        );

        // *
        // Repay more native debt (should fail)
        // *
        let env = cosmwasm_std::testing::mock_env("borrower", &[coin(2000, "borrowedcoinnative")]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoinnative"),
        };
        handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay all cw20 debt (and then some)
        // *
        let block_time = reserve_2_after_repay_all_2.interests_last_updated + 5000u64;
        let repay_amount = 4800u128;

        let expected_params_cw20 = th_get_expected_indices_and_rates(
            &deps,
            &reserve_1_after_borrow_again,
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
                log("reserve", "borrowedcoincw20"),
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
        assert_eq!(false, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt1 = debts_asset_state_read(&deps.storage, cw20_contract_addr_canonical.as_slice())
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();
        let reserve_1_after_repay_1 = reserves_state_read(&deps.storage)
            .load(cw20_contract_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(0_u128), debt1.amount_scaled);
        assert_eq!(
            Uint256::from(0_u128),
            reserve_1_after_repay_1.debt_total_scaled
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

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::one(),
            loan_to_value: ltv,
            borrow_index: Decimal256::one(),
            borrow_slope: Decimal256::one(),
            borrow_rate: Decimal256::one(),
            liquidity_rate: Decimal256::one(),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: block_time,
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let reserve = th_init_reserve(&deps.api, &mut deps.storage, b"uusd", &mock_reserve);

        // Set user as having the reserve_collateral deposited
        let deposit_amount = 110000u64;
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, reserve.index).unwrap();
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
        let time_elapsed = new_block_time - reserve.interests_last_updated;
        let liquidity_index = calculate_applied_linear_interest_rate(
            reserve.liquidity_index,
            reserve.liquidity_rate,
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
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        match res_error {
            StdError::GenericErr { msg, .. } => assert_eq!(
                "borrow amount exceeds maximum allowed given current collateral value",
                msg
            ),
            e => panic!("Unexpected error: {}", e),
        }

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
            &reserve,
            block_time,
            initial_liquidity,
            TestUtilizationDeltas {
                less_liquidity: valid_amount.into(),
                more_debt: valid_amount.into(),
                ..Default::default()
            },
        );

        let reserve_after_borrow = reserves_state_read(&deps.storage).load(b"uusd").unwrap();

        let user = users_state_read(&deps.storage)
            .load(borrower_canonical_addr.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());

        let debt = debts_asset_state_read(&deps.storage, b"uusd")
            .load(&borrower_canonical_addr.as_slice())
            .unwrap();

        assert_eq!(valid_amount, debt.amount_scaled);
        assert_eq!(
            reserve_after_borrow.protocol_income_to_distribute,
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

        let mock_reserve_1 = MockReserve {
            ma_token_address: "matoken1",
            loan_to_value: Decimal256::from_ratio(8, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_reserve_2 = MockReserve {
            ma_token_address: "matoken2",
            loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let mock_reserve_3 = MockReserve {
            ma_token_address: "matoken3",
            loan_to_value: Decimal256::from_ratio(4, 10),
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let reserve_1_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_reserve_1,
        );
        // should get index 1
        let reserve_2_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin2",
            &mock_reserve_2,
        );
        // should get index 2
        let reserve_3_initial =
            th_init_reserve(&deps.api, &mut deps.storage, b"uusd", &mock_reserve_3);

        let borrower_canonical_addr = deps
            .api
            .canonical_address(&HumanAddr::from("borrower"))
            .unwrap();

        // Set user as having all the reserves as collateral
        let mut user = User::default();

        set_bit(&mut user.collateral_assets, reserve_1_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, reserve_2_initial.index).unwrap();
        set_bit(&mut user.collateral_assets, reserve_3_initial.index).unwrap();

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

        let max_borrow_allowed_in_uusd = (reserve_1_initial.loan_to_value
            * Uint256::from(balance_1)
            * Decimal256::from(exchange_rate_1))
            + (reserve_2_initial.loan_to_value
                * Uint256::from(balance_2)
                * Decimal256::from(exchange_rate_2))
            + (reserve_3_initial.loan_to_value
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
        handle(&mut deps, env, borrow_msg).unwrap_err();

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

        let debt_contract_addr = HumanAddr::from("debt");
        let debt_contract_addr_canonical = deps.api.canonical_address(&debt_contract_addr).unwrap();
        let user_address = HumanAddr::from("user");
        let user_canonical_addr = deps.api.canonical_address(&user_address).unwrap();
        let collateral_address = HumanAddr::from("collateral");
        let liquidator_address = HumanAddr::from("liquidator");

        let collateral_max_ltv = Decimal256::from_ratio(5, 10);
        let collateral_liquidation_threshold = Decimal256::from_ratio(6, 10);
        let collateral_liquidation_bonus = Decimal256::from_ratio(1, 10);
        let collateral_price = Decimal::from_ratio(2_u128, 1_u128);
        let user_collateral_balance = Uint128(2_000_000);
        // TODO: As this is a cw20, it's price will be 1uusd, review this when oracle is
        // implemented.
        let user_debt = Uint256::from(3_000_000_u64); // ltv = 0.75
        let close_factor = Decimal256::from_ratio(1, 2);

        let first_debt_to_repay = Uint256::from(400_000_u64);
        let first_block_time = 15_000_000;

        let second_debt_to_repay = Uint256::from(10_000_000_u64);
        let second_block_time = 16_000_000;

        // Global debt for the debt reserve
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

        // initialize collateral and debt reserves
        deps.querier.set_native_exchange_rates(
            "uusd".to_string(),
            &[("collateral".to_string(), collateral_price)],
        );

        let collateral_reserve = MockReserve {
            ma_token_address: "ma_collateral",
            loan_to_value: collateral_max_ltv,
            liquidation_threshold: collateral_liquidation_threshold,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint256::from(800_000_000_u64),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            borrow_slope: Decimal256::from_ratio(1, 2),
            borrow_rate: Decimal256::from_ratio(2, 10),
            liquidity_rate: Decimal256::from_ratio(2, 10),
            reserve_factor: Decimal256::from_ratio(2, 100),
            asset_type: AssetType::Native,
            interests_last_updated: 0,
            ..Default::default()
        };

        let debt_reserve = MockReserve {
            loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: expected_global_debt_scaled,
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            borrow_slope: Decimal256::from_ratio(1, 3),
            borrow_rate: Decimal256::from_ratio(2, 10),
            liquidity_rate: Decimal256::from_ratio(2, 10),
            reserve_factor: Decimal256::from_ratio(3, 100),
            asset_type: AssetType::Cw20,
            interests_last_updated: 0,
            ..Default::default()
        };

        let collateral_reserve_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"collateral",
            &collateral_reserve,
        );

        let debt_reserve_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            debt_contract_addr_canonical.as_slice(),
            &debt_reserve,
        );

        let mut expected_user_debt_scaled = user_debt / debt_reserve_initial.liquidity_index;

        // Set user as having collateral and debt in respective reserves
        {
            let mut user = User::default();
            set_bit(
                &mut user.collateral_assets,
                collateral_reserve_initial.index,
            )
            .unwrap();
            set_bit(&mut user.borrowed_assets, debt_reserve_initial.index).unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            users_bucket
                .save(user_canonical_addr.as_slice(), &user)
                .unwrap();
        }

        // trying to liquidate user with zero collateral balance should fail
        {
            deps.querier.set_cw20_balances(
                collateral_address,
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
            handle(&mut deps, env, liquidate_msg).unwrap_err();
        }

        // Set the querier to return positive collateral balance
        deps.querier.set_cw20_balances(
            HumanAddr::from("ma_collateral"),
            &[(user_address.clone(), user_collateral_balance)],
        );

        // trying to liquidate user with zero outstanding debt should fail
        {
            let debt = Debt {
                amount_scaled: Uint256::zero(),
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
                        receive_ma_token: true,
                    })
                    .unwrap(),
                ),
                sender: liquidator_address.clone(),
                amount: first_debt_to_repay.into(),
            });

            let env = cosmwasm_std::testing::mock_env(debt_contract_addr.clone(), &[]);
            handle(&mut deps, env, liquidate_msg).unwrap_err();
        }

        // set user to have positive debt amount in debt asset
        {
            let debt = Debt {
                amount_scaled: expected_user_debt_scaled,
            };
            debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
                .save(user_canonical_addr.as_slice(), &debt)
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
            handle(&mut deps, env, liquidate_msg).unwrap_err();
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

            let collateral_reserve_before = reserves_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_reserve_before = reserves_state_read(&deps.storage)
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
            let res = handle(&mut deps, env, liquidate_msg).unwrap();

            // get expected indices and rates for debt reserve
            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps,
                &debt_reserve_initial,
                block_time,
                available_liquidity_debt,
                TestUtilizationDeltas {
                    less_debt: first_debt_to_repay.into(),
                    ..Default::default()
                },
            );

            let collateral_reserve_after = reserves_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_reserve_after = reserves_state_read(&deps.storage)
                .load(debt_contract_addr_canonical.as_slice())
                .unwrap();

            // TODO: not multiplying by collateral because it is a cw20 and Decimal::one
            // is the default price. Set a different price when implementing the oracle
            let expected_liquidated_collateral_amount = first_debt_to_repay
                * (Decimal256::one() + collateral_liquidation_bonus)
                / Decimal256::from(collateral_price);

            assert_eq!(
                res.messages,
                vec![CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("ma_collateral"),
                    msg: to_binary(&mars::ma_token::msg::HandleMsg::TransferOnLiquidation {
                        sender: user_address.clone(),
                        recipient: liquidator_address.clone(),
                        amount: expected_liquidated_collateral_amount.into(),
                    })
                    .unwrap(),
                    send: vec![],
                }),]
            );

            mars::testing::assert_eq_vec(
                res.log,
                vec![
                    log("action", "liquidate"),
                    log("collateral_reserve", "collateral"),
                    log("debt_reserve", debt_contract_addr.as_str()),
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
            assert_eq!(
                true,
                get_bit(user.collateral_assets, collateral_reserve_before.index).unwrap()
            );
            assert_eq!(
                true,
                get_bit(user.borrowed_assets, debt_reserve_before.index).unwrap()
            );

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
                debt_reserve_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                Uint256::zero(),
                collateral_reserve_after.protocol_income_to_distribute
            );
            assert_eq!(
                debt_reserve_before.protocol_income_to_distribute
                    + expected_debt_rates.protocol_income_to_distribute,
                debt_reserve_after.protocol_income_to_distribute
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

            let collateral_reserve_before = reserves_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_reserve_before = reserves_state_read(&deps.storage)
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

            // get expected indices and rates for debt and collateral reserves
            let expected_debt_indices = th_get_expected_indices(&debt_reserve_before, block_time);
            let user_debt_asset_total_debt =
                expected_user_debt_scaled * expected_debt_indices.borrow;
            // Since debt is being over_repayed, we expect to max out the liquidatable debt
            let expected_less_debt = user_debt_asset_total_debt * close_factor;

            let expected_refund_amount = second_debt_to_repay - expected_less_debt;

            let expected_debt_rates = th_get_expected_indices_and_rates(
                &deps,
                &debt_reserve_before,
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
                &collateral_reserve_before,
                block_time,
                available_liquidity_collateral, //this is the same as before as it comes from mocks
                TestUtilizationDeltas {
                    less_liquidity: expected_liquidated_collateral_amount.into(),
                    ..Default::default()
                },
            );

            let collateral_reserve_after = reserves_state_read(&deps.storage)
                .load(b"collateral")
                .unwrap();
            let debt_reserve_after = reserves_state_read(&deps.storage)
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
                        amount: vec![Coin {
                            denom: String::from("collateral"),
                            amount: expected_liquidated_collateral_amount.into(),
                        }],
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
                    log("collateral_reserve", "collateral"),
                    log("debt_reserve", debt_contract_addr.as_str()),
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
            assert_eq!(
                true,
                get_bit(user.collateral_assets, collateral_reserve_initial.index).unwrap()
            );
            assert_eq!(
                true,
                get_bit(user.borrowed_assets, debt_reserve_initial.index).unwrap()
            );

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
                debt_reserve_after.debt_total_scaled
            );

            // check correct accumulated protocol income to distribute
            assert_eq!(
                debt_reserve_before.protocol_income_to_distribute
                    + expected_debt_rates.protocol_income_to_distribute,
                debt_reserve_after.protocol_income_to_distribute
            );
            assert_eq!(
                expected_collateral_rates.protocol_income_to_distribute,
                collateral_reserve_after.protocol_income_to_distribute
            );
        }
    }

    #[test]
    fn test_liquidation_health_factor_check() {
        // initialize collateral and debt reserves
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
        let collateral_liquidation_threshold = Decimal256::from_ratio(7, 10);
        let collateral_liquidation_bonus = Decimal256::from_ratio(1, 10);

        let collateral_reserve = MockReserve {
            ma_token_address: "collateral",
            loan_to_value: collateral_ltv,
            liquidation_threshold: collateral_liquidation_threshold,
            liquidation_bonus: collateral_liquidation_bonus,
            debt_total_scaled: Uint256::zero(),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Native,
            ..Default::default()
        };
        let debt_reserve = MockReserve {
            ma_token_address: "debt",
            loan_to_value: Decimal256::from_ratio(6, 10),
            debt_total_scaled: Uint256::from(20_000_000u64),
            liquidity_index: Decimal256::one(),
            borrow_index: Decimal256::one(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };

        // initialize reserves
        let collateral_reserve_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"collateral",
            &collateral_reserve,
        );

        let debt_reserve_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            debt_contract_addr_canonical.as_slice(),
            &debt_reserve,
        );

        // test health factor check
        let healthy_user_address = HumanAddr::from("healthy_user");
        let healthy_user_canonical_addr =
            deps.api.canonical_address(&healthy_user_address).unwrap();

        // Set user as having collateral and debt in respective reserves
        let mut healthy_user = User::default();

        set_bit(
            &mut healthy_user.collateral_assets,
            collateral_reserve_initial.index,
        )
        .unwrap();
        set_bit(
            &mut healthy_user.borrowed_assets,
            debt_reserve_initial.index,
        )
        .unwrap();

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
            Uint256::from(healthy_user_collateral_balance) * collateral_liquidation_threshold;
        let healthy_user_debt = Debt {
            amount_scaled: healthy_user_debt_amount,
        };
        debts_asset_state(&mut deps.storage, debt_contract_addr_canonical.as_slice())
            .save(healthy_user_canonical_addr.as_slice(), &healthy_user_debt)
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
        handle(&mut deps, env, liquidate_msg).unwrap_err();
    }

    #[test]
    fn test_finalize_liquidity_token_transfer() {
        // Setup
        let mut deps = th_setup(&[]);
        let env_matoken = cosmwasm_std::testing::mock_env(HumanAddr::from("masomecoin"), &[]);

        let mock_reserve = MockReserve {
            ma_token_address: "masomecoin",
            liquidity_index: Decimal256::one(),
            liquidation_threshold: Decimal256::from_ratio(5, 10),
            ..Default::default()
        };
        let reserve = th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);
        let debt_mock_reserve = MockReserve {
            borrow_index: Decimal256::one(),
            ..Default::default()
        };
        let debt_reserve = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"debtcoin",
            &debt_mock_reserve,
        );

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
            set_bit(&mut sender_user.collateral_assets, reserve.index).unwrap();
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
            assert_eq!(
                get_bit(sender_user.collateral_assets, reserve.index).unwrap(),
                true
            );
            // Should create user and set deposited to true as previous balance is 0
            assert_eq!(
                get_bit(recipient_user.collateral_assets, reserve.index).unwrap(),
                true
            );
        }

        // Finalize transfer with health factor < 1 for sender doesn't go through
        {
            // set debt for user in order for health factor to be < 1
            let debt = Debt {
                amount_scaled: Uint256::from(500_000u128),
            };
            debts_asset_state(&mut deps.storage, b"debtcoin")
                .save(sender_canonical_address.as_slice(), &debt)
                .unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            let mut sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_reserve.index).unwrap();
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

            let res = handle(&mut deps, env_matoken.clone(), msg);

            match res.unwrap_err() {
                StdError::GenericErr { msg, .. } =>
                    assert_eq!("Cannot make token transfer if it results in a helth factor lower than 1 for the sender", msg),
                e => panic!("Unexpected error: {}", e),
            }
        }

        // Finalize transfer with health factor > 1 for goes through
        {
            // set debt for user in order for health factor to be > 1
            let debt = Debt {
                amount_scaled: Uint256::from(1_000u128),
            };
            debts_asset_state(&mut deps.storage, b"debtcoin")
                .save(sender_canonical_address.as_slice(), &debt)
                .unwrap();
            let mut users_bucket = users_state(&mut deps.storage);
            let mut sender_user = users_bucket
                .load(sender_canonical_address.as_slice())
                .unwrap();
            set_bit(&mut sender_user.borrowed_assets, debt_reserve.index).unwrap();
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
            assert_eq!(
                get_bit(sender_user.collateral_assets, reserve.index).unwrap(),
                false
            );
            assert_eq!(
                get_bit(recipient_user.collateral_assets, reserve.index).unwrap(),
                true
            );
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

            handle(&mut deps, env, msg).unwrap_err();
        }
    }

    #[test]
    fn test_uncollateralized_loan_limits() {
        let available_liquidity = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity, "somecoin")]);

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_slope: Decimal256::from_ratio(1, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            reserve_factor: Decimal256::from_ratio(1, 10),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            asset_type: AssetType::Native,
            ..Default::default()
        };

        // should get index 0
        let reserve_initial =
            th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

        let borrower_addr = HumanAddr::from("borrower");
        let borrower_canonical_addr = deps.api.canonical_address(&borrower_addr).unwrap();

        let mut block_time = mock_reserve.interests_last_updated + 10000u64;
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
        handle(&mut deps, update_limit_env, update_limit_msg.clone()).unwrap_err();

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
            &reserve_initial,
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
                amount: vec![Coin {
                    denom: String::from("somecoin"),
                    amount: initial_borrow_amount,
                }],
            })]
        );

        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("reserve", "somecoin"),
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
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());

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
        handle(&mut deps, borrow_env, borrow_msg).unwrap_err();

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
    }

    #[test]
    fn test_update_asset_collateral() {
        let mut deps = th_setup(&[]);

        let user_addr = HumanAddr(String::from("user"));
        let user_canonical_addr = deps.api.canonical_address(&user_addr).unwrap();

        let ma_token_address_1 = HumanAddr::from("matoken1");
        let mock_reserve_1 = MockReserve {
            ma_token_address: ma_token_address_1.as_str(),
            asset_type: AssetType::Cw20,
            ..Default::default()
        };
        let mock_reserve_2 = MockReserve {
            ma_token_address: "matoken2",
            ..Default::default()
        };
        let cw20_contract_addr = HumanAddr::from("depositedcoin1");
        let cw20_contract_addr_canonical = deps.api.canonical_address(&cw20_contract_addr).unwrap();

        // Should get index 0
        let reserve_1_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            cw20_contract_addr_canonical.as_slice(),
            &mock_reserve_1,
        );
        // Should get index 1
        let reserve_2_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin2",
            &mock_reserve_2,
        );

        // Set second asset as collateral
        let mut user = User::default();
        set_bit(&mut user.collateral_assets, reserve_2_initial.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(user_canonical_addr.as_slice(), &user)
            .unwrap();

        // Set the querier to return zero for the first asset
        deps.querier.set_cw20_balances(
            ma_token_address_1.clone(),
            &[(user_addr.clone(), Uint128::zero())],
        );

        // Enable first reserve index which is currently disabled as collateral and ma-token balance is 0
        let update_msg = HandleMsg::UpdateUserCollateralAssetStatus {
            asset: Asset::Cw20 {
                contract_addr: cw20_contract_addr.clone(),
            },
            enable: true,
        };
        let env = cosmwasm_std::testing::mock_env("user", &[]);
        let res_error = handle(&mut deps, env.clone(), update_msg.clone());
        match res_error {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                format!(
                    "User address {} has no balance in specified collateral asset {}",
                    user_addr.as_str(),
                    String::from(cw20_contract_addr.as_str())
                )
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }
        let user = users_state(&mut deps.storage)
            .load(user_canonical_addr.as_slice())
            .unwrap();
        let reserve_1_collateral =
            get_bit(user.collateral_assets, reserve_1_initial.index).unwrap();
        // Balance for first asset is zero so don't update bit
        assert_eq!(reserve_1_collateral, false);

        // Set the querier to return balance more than zero for the first asset
        deps.querier
            .set_cw20_balances(ma_token_address_1, &[(user_addr, Uint128(100_000))]);

        // Enable first reserve index which is currently disabled as collateral and ma-token balance is more than 0
        let _res = handle(&mut deps, env.clone(), update_msg).unwrap();
        let user = users_state(&mut deps.storage)
            .load(user_canonical_addr.as_slice())
            .unwrap();
        let reserve_1_collateral =
            get_bit(user.collateral_assets, reserve_1_initial.index).unwrap();
        // Balance for first asset is more than zero so update bit
        assert_eq!(reserve_1_collateral, true);

        // Disable second reserve index
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
        let reserve_2_collateral =
            get_bit(user.collateral_assets, reserve_2_initial.index).unwrap();
        assert_eq!(reserve_2_collateral, false);
    }

    #[test]
    fn test_distribute_protocol_income() {
        // initialize contract with liquidity
        let available_liquidity = 2000000000u128;
        let mut deps = th_setup(&[coin(available_liquidity, "somecoin")]);

        let asset = Asset::Native {
            denom: String::from("somecoin"),
        };
        let protocol_income_to_distribute = Uint256::from(1_000_000_u64);

        // initialize reserve with non-zero amount of protocol_income_to_distribute
        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_slope: Decimal256::from_ratio(1, 10),
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
        let reserve_initial =
            th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

        let mut block_time = mock_reserve.interests_last_updated + 10000u64;

        // call function as non-owner
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset: asset.clone(),
            amount: None,
        };

        let random_env = mars::testing::mock_env(
            "random",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );
        handle(&mut deps, random_env, distribute_income_msg).unwrap_err();

        // call function providing amount exceeding protocol_income_to_distribute, should fail
        let exceeding_amount = protocol_income_to_distribute + Uint256::from(1_000_u64);
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset: Asset::Native {
                denom: "somecoin".to_string(),
            },
            amount: Some(exceeding_amount),
        };
        let owner_env = mars::testing::mock_env(
            "owner",
            MockEnvParams {
                sent_funds: &[],
                block_time,
                ..Default::default()
            },
        );

        handle(&mut deps, owner_env.clone(), distribute_income_msg).unwrap_err();

        // call function providing amount less than protocol_income_to_distribute
        let permissible_amount = Decimal256::from_ratio(1, 2) * protocol_income_to_distribute;
        let distribute_income_msg = HandleMsg::DistributeProtocolIncome {
            asset: asset.clone(),
            amount: Some(permissible_amount),
        };
        let res = handle(&mut deps, owner_env, distribute_income_msg).unwrap();

        let config = config_state_read(&deps.storage).load().unwrap();
        let reserve_after_distribution = reserves_state_read(&deps.storage)
            .load(b"somecoin")
            .unwrap();

        let expected_insurance_fund_amount = permissible_amount * config.insurance_fund_fee_share;
        let expected_treasury_amount = permissible_amount * config.treasury_fee_share;
        let expected_staking_amount =
            permissible_amount - (expected_insurance_fund_amount + expected_treasury_amount);

        let scaled_mint_amount = expected_treasury_amount / reserve_initial.liquidity_index;

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("insurance_contract"),
                    amount: vec![Coin {
                        denom: "somecoin".to_string(),
                        amount: expected_insurance_fund_amount.into(),
                    }],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: deps
                        .api
                        .human_address(&reserve_initial.ma_token_address)
                        .unwrap(),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Mint {
                        recipient: HumanAddr::from("treasury_contract"),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("staking_contract"),
                    amount: vec![Coin {
                        denom: "somecoin".to_string(),
                        amount: expected_staking_amount.into(),
                    }],
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
            reserve_after_distribution.protocol_income_to_distribute,
            expected_remaining_income_to_be_distributed
        );

        // call function without providing an amount, should send full remaining amount to contracts
        block_time += 1000;
        let owner_env = mars::testing::mock_env(
            "owner",
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
        let res = handle(&mut deps, owner_env, distribute_income_msg).unwrap();

        // verify messages are correct and protocol_income_to_distribute field is now zero
        let expected_insurance_amount =
            expected_remaining_income_to_be_distributed * config.insurance_fund_fee_share;
        let expected_treasury_amount =
            expected_remaining_income_to_be_distributed * config.treasury_fee_share;
        let expected_staking_amount = expected_remaining_income_to_be_distributed
            - (expected_insurance_amount + expected_treasury_amount);

        let scaled_mint_amount =
            expected_treasury_amount / reserve_after_distribution.liquidity_index;

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("insurance_contract"),
                    amount: vec![Coin {
                        denom: "somecoin".to_string(),
                        amount: expected_insurance_fund_amount.into(),
                    }],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: deps
                        .api
                        .human_address(&reserve_initial.ma_token_address)
                        .unwrap(),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Mint {
                        recipient: HumanAddr::from("treasury_contract"),
                        amount: scaled_mint_amount.into(),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("staking_contract"),
                    amount: vec![Coin {
                        denom: "somecoin".to_string(),
                        amount: expected_staking_amount.into(),
                    }],
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

        let reserve_after_second_distribution = reserves_state_read(&deps.storage)
            .load(b"somecoin")
            .unwrap();
        assert_eq!(
            reserve_after_second_distribution.protocol_income_to_distribute,
            Uint256::zero()
        );
    }

    // TEST HELPERS

    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        let config = CreateOrUpdateConfig {
            treasury_contract_address: Some(HumanAddr::from("treasury_contract")),
            insurance_fund_contract_address: Some(HumanAddr::from("insurance_contract")),
            staking_contract_address: Some(HumanAddr::from("staking_contract")),
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

    #[derive(Debug)]
    struct MockReserve<'a> {
        ma_token_address: &'a str,
        liquidity_index: Decimal256,
        borrow_index: Decimal256,

        borrow_rate: Decimal256,
        liquidity_rate: Decimal256,

        borrow_slope: Decimal256,
        loan_to_value: Decimal256,

        reserve_factor: Decimal256,

        interests_last_updated: u64,
        debt_total_scaled: Uint256,

        asset_type: AssetType,

        liquidation_threshold: Decimal256,
        liquidation_bonus: Decimal256,

        protocol_income_to_distribute: Uint256,
    }

    impl Default for MockReserve<'_> {
        fn default() -> Self {
            MockReserve {
                ma_token_address: "defaultmatoken",
                liquidity_index: Default::default(),
                borrow_index: Default::default(),
                borrow_rate: Default::default(),
                liquidity_rate: Default::default(),
                borrow_slope: Default::default(),
                loan_to_value: Default::default(),
                reserve_factor: Default::default(),
                interests_last_updated: 0,
                debt_total_scaled: Default::default(),
                asset_type: AssetType::Native,
                liquidation_threshold: Decimal256::one(),
                liquidation_bonus: Decimal256::zero(),
                protocol_income_to_distribute: Uint256::zero(),
            }
        }
    }

    fn th_init_reserve<S: Storage, A: Api>(
        api: &A,
        storage: &mut S,
        key: &[u8],
        reserve: &MockReserve,
    ) -> Reserve {
        let mut index = 0;

        money_market_state(storage)
            .update(|mut mm: MoneyMarket| -> StdResult<MoneyMarket> {
                index = mm.reserve_count;
                mm.reserve_count += 1;
                Ok(mm)
            })
            .unwrap();

        let ma_token_canonical_address = api
            .canonical_address(&HumanAddr::from(reserve.ma_token_address))
            .unwrap();

        let mut reserve_bucket = reserves_state(storage);
        let new_reserve = Reserve {
            ma_token_address: ma_token_canonical_address.clone(),
            index,
            borrow_index: reserve.borrow_index,
            liquidity_index: reserve.liquidity_index,
            borrow_rate: reserve.borrow_rate,
            liquidity_rate: reserve.liquidity_rate,
            borrow_slope: reserve.borrow_slope,
            loan_to_value: reserve.loan_to_value,
            reserve_factor: reserve.reserve_factor,
            interests_last_updated: reserve.interests_last_updated,
            debt_total_scaled: reserve.debt_total_scaled,
            asset_type: reserve.asset_type.clone(),
            liquidation_threshold: reserve.liquidation_threshold,
            liquidation_bonus: reserve.liquidation_bonus,
            protocol_income_to_distribute: reserve.protocol_income_to_distribute,
        };

        reserve_bucket.save(key, &new_reserve).unwrap();

        reserve_references_state(storage)
            .save(
                &index.to_be_bytes(),
                &ReserveReferences {
                    reference: key.to_vec(),
                },
            )
            .unwrap();

        reserve_ma_tokens_state(storage)
            .save(ma_token_canonical_address.as_slice(), &key.to_vec())
            .unwrap();

        new_reserve
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

    /// Takes a reserve before an action (ie: a borrow) among some test parameters
    /// used in that action and computes the expected indices and rates after that action.
    fn th_get_expected_indices_and_rates<S: Storage, A: Api, Q: Querier>(
        deps: &Extern<S, A, Q>,
        reserve: &Reserve,
        block_time: u64,
        initial_liquidity: u128,
        deltas: TestUtilizationDeltas,
    ) -> TestInterestResults {
        let expected_indices = th_get_expected_indices(reserve, block_time);

        // Compute protocol income to be distributed (using values up to the instant
        // before the contract call is made)
        let previous_borrow_index = reserve.borrow_index;
        let previous_debt_total = reserve.debt_total_scaled * previous_borrow_index;
        let current_debt_total = reserve.debt_total_scaled * expected_indices.borrow;
        let interest_accrued = if current_debt_total > previous_debt_total {
            current_debt_total - previous_debt_total
        } else {
            Uint256::zero()
        };
        let expected_protocol_income_to_distribute = interest_accrued * reserve.reserve_factor;

        // When borrowing, new computed index is used for scaled amount
        let more_debt_scaled = Uint256::from(deltas.more_debt) / expected_indices.borrow;
        // When repaying, new computed index is used for scaled amount
        let less_debt_scaled = Uint256::from(deltas.less_debt) / expected_indices.borrow;
        // NOTE: Don't panic here so that the total repay of debt can be simulated
        // when less debt is greater than outstanding debt
        let new_debt_total_scaled =
            if (reserve.debt_total_scaled + more_debt_scaled) > less_debt_scaled {
                reserve.debt_total_scaled + more_debt_scaled - less_debt_scaled
            } else {
                Uint256::zero()
            };
        let dec_debt_total =
            Decimal256::from_uint256(new_debt_total_scaled) * expected_indices.borrow;
        let total_protocol_income_to_distribute =
            reserve.protocol_income_to_distribute + expected_protocol_income_to_distribute;

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
        let expected_borrow_rate = expected_utilization_rate * reserve.borrow_slope;
        let expected_liquidity_rate = expected_borrow_rate
            * expected_utilization_rate
            * (Decimal256::one() - reserve.reserve_factor);

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

    fn th_get_expected_indices(reserve: &Reserve, block_time: u64) -> TestExpectedIndices {
        let seconds_elapsed = block_time - reserve.interests_last_updated;
        // market indices
        let expected_liquidity_index = calculate_applied_linear_interest_rate(
            reserve.liquidity_index,
            reserve.liquidity_rate,
            seconds_elapsed,
        );

        let expected_borrow_index = calculate_applied_linear_interest_rate(
            reserve.borrow_index,
            reserve.borrow_rate,
            seconds_elapsed,
        );

        TestExpectedIndices {
            liquidity: expected_liquidity_index,
            borrow: expected_borrow_index,
        }
    }
}
