use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, LogAttribute, MigrateResponse, MigrateResult, Order,
    Querier, QueryRequest, StdError, StdResult, Storage, Uint128, WasmMsg, WasmQuery,
};

use cw20::{BalanceResponse, Cw20HandleMsg, Cw20QueryMsg, Cw20ReceiveMsg, MinterResponse};
use mars::ma_token;

use crate::msg::{
    ConfigResponse, DebtInfo, DebtResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg, ReceiveMsg,
    ReserveInfo, ReserveResponse, ReservesListResponse,
};
use crate::state::{
    config_state, config_state_read, debts_asset_state, debts_asset_state_read,
    reserve_denoms_state, reserve_denoms_state_read, reserves_state, reserves_state_read,
    users_state, users_state_read, Config, Debt, Reserve, ReserveDenoms, User,
};
use terra_cosmwasm::{ExchangeRatesResponse, TerraQuerier};

// CONSTANTS

const SECONDS_PER_YEAR: u64 = 31536000u64;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        ma_token_code_id: msg.ma_token_code_id,
        reserve_count: 0,
    };

    config_state(&mut deps.storage).save(&config)?;

    Ok(InitResponse::default())
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Receive(cw20_msg) => receive_cw20(deps, env, cw20_msg),
        HandleMsg::InitAsset {
            denom,
            borrow_slope,
        } => init_asset(deps, env, denom, borrow_slope),
        HandleMsg::InitAssetTokenCallback { id } => init_asset_token_callback(deps, env, id),
        HandleMsg::DepositNative { denom } => deposit_native(deps, env, denom),
        HandleMsg::BorrowNative { denom, amount } => borrow_native(deps, env, denom, amount),
        HandleMsg::RepayNative { denom } => repay_native(deps, env, denom),
    }
}

/// cw20 receive implementation
pub fn receive_cw20<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<HandleResponse> {
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            ReceiveMsg::Redeem { id } => {
                let reserve = reserves_state_read(&deps.storage).load(id.as_bytes())?;
                if deps.api.canonical_address(&env.message.sender)? != reserve.ma_token_address {
                    return Err(StdError::unauthorized());
                }

                // TODO: if cw20s are added, then this needs some extra handling
                redeem_native(
                    deps,
                    env,
                    id,
                    reserve,
                    cw20_msg.sender,
                    Uint256::from(cw20_msg.amount),
                )
            }
        }
    } else {
        Err(StdError::generic_err("Invalid Cw20RecieveMsg"))
    }
}

/// Burns sent maAsset in exchange of underlying asset
pub fn redeem_native<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
    mut reserve: Reserve,
    to: HumanAddr,
    burn_amount: Uint256,
) -> StdResult<HandleResponse> {
    reserve_update_market_indices(&env, &mut reserve);
    reserve_update_interest_rates(&deps.querier, &env, &denom, &mut reserve, burn_amount)?;
    reserves_state(&mut deps.storage).save(&denom.as_bytes(), &reserve)?;

    // Redeem amount is computed after interest rates so that the updated index is used
    let redeem_amount = burn_amount * reserve.liquidity_index;

    let balance = deps.querier.query_balance(&env.contract.address, &denom)?;
    if redeem_amount > Uint256::from(balance.amount) {
        return Err(StdError::generic_err(
            "Redeem amount exceeds contract balance",
        ));
    }

    let mut log = vec![
        log("action", "redeem"),
        log("reserve", denom.clone()),
        log("user", to.clone()),
        log("burn_amount", burn_amount),
        log("redeem_amount", redeem_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &reserve);

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: burn_amount.into(),
                })?,
            }),
            CosmosMsg::Bank(BankMsg::Send {
                from_address: env.contract.address,
                to_address: to.clone(),
                amount: vec![Coin {
                    denom: denom.clone(),
                    amount: redeem_amount.into(),
                }],
            }),
        ],
        log,
        data: None,
    })
}

/// Initialize asset so it can be deposited and borrowed.
/// A new maToken should be created which callbacks this contract in order to be registered
pub fn init_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
    borrow_slope: Decimal256,
) -> StdResult<HandleResponse> {
    // Get config
    let mut config = config_state_read(&deps.storage).load()?;

    // Only owner can do this
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    // create only if it doesn't exist
    let mut reserves = reserves_state(&mut deps.storage);
    match reserves.may_load(denom.as_bytes()) {
        Ok(None) => {
            // create asset reserve
            reserves.save(
                denom.as_bytes(),
                &Reserve {
                    index: config.reserve_count,
                    ma_token_address: CanonicalAddr::default(),

                    borrow_index: Decimal256::one(),
                    liquidity_index: Decimal256::one(),
                    borrow_rate: Decimal256::zero(),
                    liquidity_rate: Decimal256::zero(),

                    borrow_slope,

                    loan_to_value: Decimal256::from_ratio(8, 10),

                    interests_last_updated: env.block.time,
                    debt_total_scaled: Uint256::zero(),
                },
            )?;

            // save index to denom mapping
            reserve_denoms_state(&mut deps.storage).save(
                &config.reserve_count.to_be_bytes(),
                &ReserveDenoms {
                    denom: denom.clone(),
                },
            )?;

            // increment reserve count
            config.reserve_count += 1;
            config_state(&mut deps.storage).save(&config)?;
        }
        Ok(Some(_)) => return Err(StdError::generic_err("Asset already initialized")),
        Err(err) => return Err(err),
    }

    // Prepare response, should instantiate an maToken
    // and use the Register hook
    Ok(HandleResponse {
        log: vec![],
        data: None,
        messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id: config.ma_token_code_id,
            msg: to_binary(&ma_token::msg::InitMsg {
                name: format!("mars {} debt token", denom),
                symbol: format!("ma{}", denom),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: HumanAddr::from(env.contract.address.as_str()),
                    cap: None,
                }),
                init_hook: Some(ma_token::msg::InitHook {
                    msg: to_binary(&HandleMsg::InitAssetTokenCallback { id: denom })?,
                    contract_addr: env.contract.address,
                }),
            })?,
            send: vec![],
            label: None,
        })],
    })
}

pub fn init_asset_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    id: String,
) -> StdResult<HandleResponse> {
    let mut state = reserves_state(&mut deps.storage);
    let mut reserve = state.load(&id.as_bytes())?;

    if reserve.ma_token_address == CanonicalAddr::default() {
        reserve.ma_token_address = deps.api.canonical_address(&env.message.sender)?;
        state.save(&id.as_bytes(), &reserve)?;
        Ok(HandleResponse {
            messages: vec![],
            log: vec![
                log("action", "init_asset"),
                log("asset", &id),
                log("ma_token_address", &env.message.sender),
            ],
            data: None,
        })
    } else {
        // Can do this only once
        Err(StdError::unauthorized())
    }
}

/// Handle the deposit of native tokens and mint corresponding debt tokens
pub fn deposit_native<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
) -> StdResult<HandleResponse> {
    let mut reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;

    // Get deposit amount
    // TODO: assumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    let deposit_amount = get_denom_amount_from_coins(&env.message.sent_funds, &denom);

    // Cannot deposit zero amount
    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            denom,
        )));
    }

    let depositer_addr = deps.api.canonical_address(&env.message.sender)?;
    let mut user: User = match users_state_read(&deps.storage).may_load(depositer_addr.as_slice()) {
        Ok(Some(user)) => user,
        Ok(None) => User {
            borrowed_assets: Uint128::zero(),
            deposited_assets: Uint128::zero(),
        },
        Err(error) => return Err(error),
    };

    let has_deposited_asset = get_bit(user.deposited_assets, reserve.index)?;
    if !has_deposited_asset {
        set_bit(&mut user.deposited_assets, reserve.index)?;
        users_state(&mut deps.storage).save(depositer_addr.as_slice(), &user)?;
    }

    reserve_update_market_indices(&env, &mut reserve);
    reserve_update_interest_rates(&deps.querier, &env, &denom, &mut reserve, Uint256::zero())?;
    reserves_state(&mut deps.storage).save(denom.as_bytes(), &reserve)?;

    if reserve.liquidity_index.is_zero() {
        return Err(StdError::generic_err("Cannot have 0 as liquidity index"));
    }
    let mint_amount = deposit_amount / reserve.liquidity_index;

    let mut log = vec![
        log("action", "deposit"),
        log("reserve", denom),
        log("user", env.message.sender.clone()),
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
                recipient: env.message.sender,
                amount: mint_amount.into(),
            })?,
        })],
    })
}

/// Add debt for the borrower and send the borrowed funds
pub fn borrow_native<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
    borrow_amount: Uint256,
) -> StdResult<HandleResponse> {
    // Cannot borrow zero amount
    if borrow_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Borrow amount must be greater than 0 {}",
            denom,
        )));
    }

    let config = config_state_read(&deps.storage).load()?;
    let mut borrow_reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;
    let borrower_addr = deps.api.canonical_address(&env.message.sender)?;
    let mut user: User = match users_state_read(&deps.storage).may_load(borrower_addr.as_slice()) {
        Ok(Some(user)) => user,
        Ok(None) => return Err(StdError::generic_err("address has no collateral deposited")),
        Err(error) => return Err(error),
    };

    // TODO: Check the contract has enough funds to safely lend them

    // Validate user has enough collateral
    let mut denoms_to_query: Vec<String> = vec![denom.clone()];
    let mut user_balances: Vec<(String, Uint256, Decimal256)> = vec![]; // (denom, debt_amount, max_borrow)
    for i in 0..config.reserve_count {
        let user_is_using_as_collateral = get_bit(user.deposited_assets, i)?;
        let user_is_borrowing = get_bit(user.borrowed_assets, i)?;
        if user_is_using_as_collateral || user_is_borrowing {
            let denom = reserve_denoms_state_read(&deps.storage)
                .load(&i.to_be_bytes())?
                .denom;
            let asset_reserve = reserves_state_read(&deps.storage).load(&denom.as_bytes())?;

            let mut debt = Uint256::zero();
            let mut max_borrow = Decimal256::zero();

            if user_is_using_as_collateral {
                // query asset balance (ma_token contract gives back a scaled value)
                let asset_balance: BalanceResponse =
                    deps.querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                        contract_addr: deps.api.human_address(&asset_reserve.ma_token_address)?,
                        msg: to_binary(&Cw20QueryMsg::Balance {
                            address: deps.api.human_address(&borrower_addr)?,
                        })?,
                    }))?;

                let collateral =
                    Uint256::from(asset_balance.balance) * asset_reserve.liquidity_index;
                max_borrow = Decimal256::from_uint256(collateral) * asset_reserve.loan_to_value;
            }

            if user_is_borrowing {
                // query debt
                let debts_asset_bucket = debts_asset_state(&mut deps.storage, denom.as_bytes());
                let borrower_debt: Debt = debts_asset_bucket.load(borrower_addr.as_slice())?;
                debt = borrower_debt.amount_scaled * asset_reserve.borrow_index;
            }

            user_balances.push((denom.clone(), debt, max_borrow));
            denoms_to_query.push(denom.clone());
        }
    }

    let querier = TerraQuerier::new(&deps.querier);
    let denoms_to_query: Vec<&str> = denoms_to_query.iter().map(AsRef::as_ref).collect(); // type conversion
    let exchange_rates: ExchangeRatesResponse =
        querier.query_exchange_rates("uusd", denoms_to_query)?;
    let mut total_debt_in_uusd = Uint256::zero();
    let mut max_borrow_in_uusd = Decimal256::zero();
    for (denom, debt, max_borrow) in user_balances {
        for rate in &exchange_rates.exchange_rates {
            if rate.quote_denom == denom {
                let exchange_rate = Decimal256::from(rate.exchange_rate);
                total_debt_in_uusd += debt * exchange_rate;
                max_borrow_in_uusd += max_borrow * exchange_rate;
                break;
            }
        }
    }

    let borrow_amount_rate = exchange_rates
        .exchange_rates
        .iter()
        .find(|e| e.quote_denom == denom)
        .unwrap();
    let borrow_amount_in_uusd = borrow_amount * Decimal256::from(borrow_amount_rate.exchange_rate);

    if Decimal256::from_uint256(total_debt_in_uusd + borrow_amount_in_uusd) > max_borrow_in_uusd {
        return Err(StdError::generic_err(
            "borrow amount exceeds maximum allowed given current collateral value",
        ));
    }

    reserve_update_market_indices(&env, &mut borrow_reserve);

    // Set borrowing asset for user
    let is_borrowing_asset = get_bit(user.borrowed_assets, borrow_reserve.index)?;
    if !is_borrowing_asset {
        set_bit(&mut user.borrowed_assets, borrow_reserve.index)?;
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket.save(borrower_addr.as_slice(), &user)?;
    }

    // Set new debt
    let mut debts_asset_bucket = debts_asset_state(&mut deps.storage, denom.as_bytes());
    let mut debt: Debt = match debts_asset_bucket.may_load(borrower_addr.as_slice()) {
        Ok(Some(debt)) => debt,
        Ok(None) => Debt {
            amount_scaled: Uint256::zero(),
        },
        Err(error) => return Err(error),
    };
    let borrow_amount_scaled = borrow_amount / borrow_reserve.borrow_index;
    debt.amount_scaled += borrow_amount_scaled;
    debts_asset_bucket.save(borrower_addr.as_slice(), &debt)?;

    borrow_reserve.debt_total_scaled += borrow_amount_scaled;

    reserve_update_interest_rates(
        &deps.querier,
        &env,
        &denom,
        &mut borrow_reserve,
        borrow_amount,
    )?;
    reserves_state(&mut deps.storage).save(&denom.as_bytes(), &borrow_reserve)?;

    let mut log = vec![
        log("action", "borrow"),
        log("reserve", denom.clone()),
        log("user", env.message.sender.clone()),
        log("amount", borrow_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &borrow_reserve);

    Ok(HandleResponse {
        data: None,
        log,
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: env.contract.address,
            to_address: env.message.sender,
            amount: vec![Coin {
                denom,
                amount: borrow_amount.into(),
            }],
        })],
    })
}

/// Handle the repay of native tokens. Refund extra funds if they exist
pub fn repay_native<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
) -> StdResult<HandleResponse> {
    // TODO: assumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    let mut reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;

    // Get repay amount
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    let repay_amount = get_denom_amount_from_coins(&env.message.sent_funds, &denom);

    // Cannot repay zero amount
    if repay_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Repay amount must be greater than 0 {}",
            denom,
        )));
    }

    // TODO: Interest rate update and computing goes somewhere around here
    let borrower_addr = deps.api.canonical_address(&env.message.sender)?;

    // Check new debt
    let mut debts_asset_bucket = debts_asset_state(&mut deps.storage, denom.as_bytes());
    let mut debt = debts_asset_bucket.load(borrower_addr.as_slice())?;

    if debt.amount_scaled.is_zero() {
        return Err(StdError::generic_err("Cannot repay 0 debt"));
    }

    reserve_update_market_indices(&env, &mut reserve);

    let mut repay_amount_scaled = repay_amount / reserve.borrow_index;

    let mut messages = vec![];
    let mut refund_amount = Uint256::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        // TODO: Should we log this?
        refund_amount = (repay_amount_scaled - debt.amount_scaled) * reserve.borrow_index;
        messages.push(CosmosMsg::Bank(BankMsg::Send {
            from_address: env.contract.address.clone(),
            to_address: env.message.sender.clone(),
            amount: vec![Coin {
                denom: denom.clone(),
                amount: refund_amount.into(),
            }],
        }));
        repay_amount_scaled = debt.amount_scaled;
    }

    debt.amount_scaled = debt.amount_scaled - repay_amount_scaled;
    debts_asset_bucket.save(borrower_addr.as_slice(), &debt)?;

    if repay_amount_scaled > reserve.debt_total_scaled {
        return Err(StdError::generic_err(
            "Amount to repay is greater than total debt",
        ));
    }
    reserve.debt_total_scaled = reserve.debt_total_scaled - repay_amount_scaled;
    reserve_update_interest_rates(&deps.querier, &env, &denom, &mut reserve, Uint256::zero())?;
    reserves_state(&mut deps.storage).save(denom.as_bytes(), &reserve)?;

    if debt.amount_scaled == Uint256::zero() {
        // Remove asset from borrowed assets
        let mut users_bucket = users_state(&mut deps.storage);
        let mut user = users_bucket.load(borrower_addr.as_slice())?;
        unset_bit(&mut user.borrowed_assets, reserve.index)?;
        users_bucket.save(borrower_addr.as_slice(), &user)?;
    }

    let mut log = vec![
        log("action", "repay"),
        log("reserve", denom),
        log("user", env.message.sender.clone()),
        log("amount", repay_amount - refund_amount),
    ];

    append_indices_and_rates_to_logs(&mut log, &reserve);

    Ok(HandleResponse {
        data: None,
        log,
        messages,
    })
}

// QUERIES

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Reserve { denom } => to_binary(&query_reserve(deps, denom)?),
        QueryMsg::ReservesList {} => to_binary(&query_reserves_list(deps)?),
        QueryMsg::Debt { address } => to_binary(&query_debt(deps, address)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        ma_token_code_id: config.ma_token_code_id,
        reserve_count: config.reserve_count,
    })
}

fn query_reserve<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    denom: String,
) -> StdResult<ReserveResponse> {
    let reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;
    let ma_token_address = deps.api.human_address(&reserve.ma_token_address)?;
    Ok(ReserveResponse { ma_token_address })
}

fn query_reserves_list<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ReservesListResponse> {
    let reserves = reserves_state_read(&deps.storage);

    let reserves_list: StdResult<Vec<_>> = reserves
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = String::from_utf8(k);
            let denom = match denom {
                Ok(denom) => denom,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            };
            let ma_token_address = deps
                .api
                .human_address(&CanonicalAddr::from(v.ma_token_address))?;
            Ok(ReserveInfo {
                denom,
                ma_token_address,
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
    let user: User = match users_bucket.may_load(debtor_address.as_slice()) {
        Ok(Some(user)) => user,
        Ok(None) => User {
            borrowed_assets: Uint128::zero(),
            deposited_assets: Uint128::zero(),
        },
        Err(error) => return Err(error),
    };

    let debts: StdResult<Vec<_>> = reserves
        .range(None, None, Order::Ascending)
        .map(|item| {
            let (k, v) = item?;
            let denom = String::from_utf8(k);
            let denom = match denom {
                Ok(denom) => denom,
                Err(_) => return Err(StdError::generic_err("failed to encode denom into string")),
            };
            let is_borrowing_asset = get_bit(user.borrowed_assets, v.index)?;
            if is_borrowing_asset {
                let debts_asset_bucket = debts_asset_state_read(&deps.storage, denom.as_bytes());
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

/// Updates reserve indices by applying current interest rates on the time between last interest update
/// and current block. Note it does not save the reserve to the store (that is left to the caller)
pub fn reserve_update_market_indices(env: &Env, reserve: &mut Reserve) {
    let current_timestamp = env.block.time;

    if reserve.interests_last_updated < current_timestamp {
        let time_elapsed =
            Decimal256::from_uint256(current_timestamp - reserve.interests_last_updated);
        let seconds_per_year = Decimal256::from_uint256(SECONDS_PER_YEAR);

        if reserve.borrow_rate > Decimal256::zero() {
            let accumulated_interest =
                Decimal256::one() + reserve.borrow_rate * time_elapsed / seconds_per_year;
            reserve.borrow_index = reserve.borrow_index * accumulated_interest;
        }
        if reserve.liquidity_rate > Decimal256::zero() {
            let accumulated_interest =
                Decimal256::one() + reserve.liquidity_rate * time_elapsed / seconds_per_year;
            reserve.liquidity_index = reserve.liquidity_index * accumulated_interest;
        }
        reserve.interests_last_updated = current_timestamp;
    }
}

/// Update interest rates for current liquidity and debt levels
/// Note it does not save the reserve to the store (that is left to the caller)
pub fn reserve_update_interest_rates<Q: Querier>(
    querier: &Q,
    env: &Env,
    denom: &str,
    reserve: &mut Reserve,
    liquidity_taken: Uint256,
) -> StdResult<()> {
    // TODO: handle cw20
    let denom_coin = querier.query_balance(env.contract.address.clone(), denom)?;

    // TODO: Verify on integration tests that this balance includes the
    // amount sent by the user on deposits and repays(both for cw20 and native).
    // If it doesn't, we should include them on the available_liquidity
    let current_balance = Uint256::from(denom_coin.amount);
    if current_balance < liquidity_taken {
        return Err(StdError::generic_err(
            "Liquidity taken cannot be greater than available liquidity",
        ));
    }
    let available_liquidity = Decimal256::from_uint256(current_balance - liquidity_taken);
    let total_debt = Decimal256::from_uint256(reserve.debt_total_scaled) * reserve.borrow_index;
    let mut utilization_rate = Decimal256::zero();
    if total_debt > Decimal256::zero() {
        utilization_rate = total_debt / (available_liquidity + total_debt);
    }

    reserve.borrow_rate = reserve.borrow_slope * utilization_rate;
    reserve.liquidity_rate = reserve.borrow_rate * utilization_rate;

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

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_querier::{mock_dependencies, WasmMockQuerier};
    use crate::state::{debts_asset_state_read, users_state_read};
    use cosmwasm_std::testing::{mock_env, MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{
        coin, from_binary, BlockInfo, ContractInfo, Decimal, Env, Extern, MessageInfo,
    };

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 10u64,
        };
        let env = mock_env("owner", &[]);

        // we can just call .unwrap() to assert this was a success
        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(&deps, QueryMsg::Config {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(10, value.ma_token_code_id);
        assert_eq!(0, value.reserve_count);
    }

    #[test]
    fn test_init_native_asset() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 5u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // *
        // non owner is not authorized
        // *
        let env = mock_env("somebody", &[]);
        let msg = HandleMsg::InitAsset {
            denom: String::from("someasset"),
            borrow_slope: Decimal256::from_ratio(4, 100),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // owner is authorized
        // *
        let env = mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            denom: String::from("someasset"),
            borrow_slope: Decimal256::from_ratio(4, 100),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // should have asset reserve with Canonical default address
        let reserve = reserves_state_read(&deps.storage)
            .load(b"someasset")
            .unwrap();
        assert_eq!(CanonicalAddr::default(), reserve.ma_token_address);
        // should have 0 index
        assert_eq!(0, reserve.index);

        // Should have reserve count of 1
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(config.reserve_count, 1);

        // should instantiate a debt token
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 5u64,
                msg: to_binary(&ma_token::msg::InitMsg {
                    name: String::from("mars someasset debt token"),
                    symbol: String::from("masomeasset"),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        cap: None,
                    }),
                    init_hook: Some(ma_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitAssetTokenCallback {
                            id: String::from("someasset")
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

        // *
        // callback comes back with created token
        // *
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("someasset"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "init_asset"),
                log("asset", "someasset"),
                log("ma_token_address", "mtokencontract"),
            ]
        );

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
        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("someasset"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // calling with a different asset increments count
        // *
        let env = mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            denom: String::from("otherasset"),
            borrow_slope: Decimal256::from_ratio(4, 100),
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let reserve = reserves_state_read(&deps.storage)
            .load(b"otherasset")
            .unwrap();
        assert_eq!(1, reserve.index);

        // Should have reserve count of 2
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(2, config.reserve_count);
    }

    #[test]
    fn test_init_asset_callback_cannot_be_called_on_its_own() {
        let mut deps = th_setup(&[]);

        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("uluna"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_deposit_native_asset() {
        let initial_liquidity = 10000000;
        let mut deps = th_setup(&[coin(initial_liquidity, "somecoin")]);

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(11, 10),
            loan_to_value: Decimal256::one(),
            borrow_index: Decimal256::from_ratio(1, 1),
            borrow_slope: Decimal256::from_ratio(1, 10),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            ..Default::default()
        };
        let reserve = th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

        let deposit_amount = 110000;
        let env = th_mock_env(MockEnvParams {
            sender: "depositer",
            sent_funds: &[coin(deposit_amount, "somecoin")],
            block_time: Some(10000100),
        });
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let res = handle(&mut deps, env.clone(), msg).unwrap();

        // previous * (1 + rate * time / 31536000)
        let expected_accumulated_interest = Decimal256::one()
            + (Decimal256::from_ratio(10, 100) * Decimal256::from_uint256(100u64)
                / Decimal256::from_uint256(SECONDS_PER_YEAR));

        let expected_liquidity_index =
            Decimal256::from_ratio(11, 10) * expected_accumulated_interest;
        let expected_mint_amount =
            (Uint256::from(deposit_amount) / expected_liquidity_index).into();

        let expected_params = th_get_expected_indices_and_rates(
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
                    recipient: HumanAddr::from("depositer"),
                    amount: expected_mint_amount,
                })
                .unwrap(),
            }),]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "deposit"),
                log("reserve", "somecoin"),
                log("user", "depositer"),
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
        // LR = BR * U = 0.05 * 0.5 = 0.025
        assert_eq!(reserve.liquidity_rate, Decimal256::from_ratio(25, 1000));
        assert_eq!(reserve.liquidity_index, expected_liquidity_index);
        assert_eq!(reserve.borrow_index, Decimal256::from_ratio(1, 1));

        // empty deposit fails
        let env = mock_env("depositer", &[]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_cannot_deposit_if_no_reserve() {
        let mut deps = th_setup(&[]);

        let env = mock_env("depositer", &[coin(110000, "somecoin")]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_redeem_native() {
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
            debt_total_scaled: Uint256::from(10000000u128),
            interests_last_updated: 10000000,
            ..Default::default()
        };
        let burn_amount = 20000u128;
        let seconds_elapsed = 2000u64;

        let reserve = th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", &mock_reserve);

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::Redeem {
                    id: String::from("somecoin"),
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("redeemer"),
            amount: Uint128(burn_amount),
        });

        let env = th_mock_env(MockEnvParams {
            sender: "matoken",
            sent_funds: &[],
            block_time: Some(mock_reserve.interests_last_updated + seconds_elapsed),
        });
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_params = th_get_expected_indices_and_rates(
            &reserve,
            mock_reserve.interests_last_updated + seconds_elapsed,
            initial_available_liquidity,
            TestUtilizationDeltas {
                less_liquidity: burn_amount,
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
                    },],
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

        let reserve = reserves_state_read(&deps.storage)
            .load(b"somecoin")
            .unwrap();

        // BR = U * Bslope = 0.5 * 0.01 = 0.05
        assert_eq!(reserve.borrow_rate, expected_params.borrow_rate);
        assert_eq!(reserve.liquidity_rate, expected_params.liquidity_rate);
        assert_eq!(reserve.liquidity_index, expected_params.liquidity_index);
        assert_eq!(reserve.borrow_index, expected_params.borrow_index);
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
            msg: Some(
                to_binary(&ReceiveMsg::Redeem {
                    id: String::from("somecoin"),
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("redeemer"),
            amount: Uint128(2000),
        });

        let env = mock_env("matoken", &[]);
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_borrow_and_repay_native() {
        // NOTE: available liquidity stays fixed as the test environment does not get changes in
        // contract balances on subsequent calls. They would change from call to call in practice
        let available_liquidity_1 = 1000000000u128;
        let available_liquidity_2 = 2000000000u128;
        let mut deps = th_setup(&[
            coin(available_liquidity_1, "borrowedcoin1"),
            coin(available_liquidity_2, "borrowedcoin2"),
        ]);

        let exchange_rates = [
            (&String::from("borrowedcoin1"), &Decimal::one()),
            (&String::from("borrowedcoin2"), &Decimal::one()),
            (&String::from("depositedcoin"), &Decimal::one()),
        ];
        deps.querier
            .with_exchange_rates(&[(&String::from("uusd"), &exchange_rates)]);

        let mock_reserve_1 = MockReserve {
            ma_token_address: "matoken1",
            borrow_index: Decimal256::from_ratio(12, 10),
            liquidity_index: Decimal256::from_ratio(8, 10),
            borrow_slope: Decimal256::from_ratio(1, 10),
            borrow_rate: Decimal256::from_ratio(20, 100),
            liquidity_rate: Decimal256::from_ratio(10, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            ..Default::default()
        };
        let mock_reserve_2 = MockReserve {
            ma_token_address: "matoken2",
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::one(),
            ..Default::default()
        };
        let mock_reserve_3 = MockReserve {
            ma_token_address: "matoken3",
            borrow_index: Decimal256::one(),
            liquidity_index: Decimal256::from_ratio(11, 10),
            loan_to_value: Decimal256::from_ratio(7, 10),
            borrow_slope: Decimal256::from_ratio(4, 10),
            borrow_rate: Decimal256::from_ratio(30, 100),
            liquidity_rate: Decimal256::from_ratio(20, 100),
            debt_total_scaled: Uint256::zero(),
            interests_last_updated: 10000000,
            ..Default::default()
        };

        // should get index 0
        let reserve_1_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoin1",
            &mock_reserve_1,
        );
        // should get index 1
        let reserve_2_initial = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoin2",
            &mock_reserve_2,
        );
        // should get index 2
        let reserve_collateral = th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"depositedcoin",
            &mock_reserve_3,
        );

        let borrower_addr_canonical = deps
            .api
            .canonical_address(&HumanAddr::from("borrower"))
            .unwrap();

        // Set user as having the reserve_collateral deposited
        let mut user = User {
            borrowed_assets: Uint128::zero(),
            deposited_assets: Uint128::zero(),
        };
        set_bit(&mut user.deposited_assets, reserve_collateral.index).unwrap();
        let mut users_bucket = users_state(&mut deps.storage);
        users_bucket
            .save(borrower_addr_canonical.as_slice(), &user)
            .unwrap();

        // Set the querier to return a certain collateral balance
        deps.querier
            .with_balances(&[(&HumanAddr(String::from("borrower")), &Uint128(10000))]);

        // TODO: probably some variables (ie: borrow_amount, expected_params) that are repeated
        // in all calls could be enclosed in local scopes somehow)
        // *
        // Borrow coin 1
        // *
        let block_time = mock_reserve_1.interests_last_updated + 10000u64;
        let borrow_amount = 2400u128;
        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin1"),
            amount: Uint256::from(borrow_amount),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_params_1 = th_get_expected_indices_and_rates(
            &reserve_1_initial,
            block_time,
            available_liquidity_1,
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
                    denom: String::from("borrowedcoin1"),
                    amount: Uint128(borrow_amount),
                },],
            }),]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("reserve", "borrowedcoin1"),
                log("user", "borrower"),
                log("amount", borrow_amount),
                log("borrow_index", expected_params_1.borrow_index),
                log("liquidity_index", expected_params_1.liquidity_index),
                log("borrow_rate", expected_params_1.borrow_rate),
                log("liquidity_rate", expected_params_1.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let expected_debt_scaled_1_after_borrow =
            Uint256::from(borrow_amount) / expected_params_1.borrow_index;

        let reserve_1_after_borrow = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin1")
            .unwrap();

        assert_eq!(expected_debt_scaled_1_after_borrow, debt.amount_scaled);
        assert_eq!(
            expected_debt_scaled_1_after_borrow,
            reserve_1_after_borrow.debt_total_scaled
        );
        assert_eq!(
            expected_params_1.borrow_rate,
            reserve_1_after_borrow.borrow_rate
        );
        assert_eq!(
            expected_params_1.liquidity_rate,
            reserve_1_after_borrow.liquidity_rate
        );

        // *
        // Borrow coin 1 (again)
        // *
        let borrow_amount = 1200u128;
        let block_time = reserve_1_after_borrow.interests_last_updated + 20000u64;
        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin1"),
            amount: Uint256::from(borrow_amount),
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_1 = th_get_expected_indices_and_rates(
            &reserve_1_after_borrow,
            block_time,
            available_liquidity_1,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );
        let debt = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let reserve_1_after_borrow_again = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin1")
            .unwrap();

        let expected_debt_scaled_1_after_borrow_again = expected_debt_scaled_1_after_borrow
            + Uint256::from(borrow_amount) / expected_params_1.borrow_index;
        assert_eq!(
            expected_debt_scaled_1_after_borrow_again,
            debt.amount_scaled
        );
        assert_eq!(
            expected_debt_scaled_1_after_borrow_again,
            reserve_1_after_borrow_again.debt_total_scaled
        );
        assert_eq!(
            expected_params_1.borrow_rate,
            reserve_1_after_borrow_again.borrow_rate
        );
        assert_eq!(
            expected_params_1.liquidity_rate,
            reserve_1_after_borrow_again.liquidity_rate
        );

        // *
        // Borrow coin 2
        // *

        let borrow_amount = 4000u128;
        let block_time = reserve_1_after_borrow_again.interests_last_updated + 3000u64;
        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin2"),
            amount: Uint256::from(borrow_amount),
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());

        let expected_params_2 = th_get_expected_indices_and_rates(
            &reserve_2_initial,
            block_time,
            available_liquidity_2,
            TestUtilizationDeltas {
                less_liquidity: borrow_amount,
                more_debt: borrow_amount,
                ..Default::default()
            },
        );
        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let reserve_2_after_borrow_2 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin2")
            .unwrap();

        let expected_debt_scaled_2_after_borrow_2 =
            Uint256::from(borrow_amount) / expected_params_2.borrow_index;
        assert_eq!(expected_debt_scaled_2_after_borrow_2, debt2.amount_scaled);
        assert_eq!(
            expected_debt_scaled_2_after_borrow_2,
            reserve_2_after_borrow_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_2.borrow_rate,
            reserve_2_after_borrow_2.borrow_rate
        );
        assert_eq!(
            expected_params_2.liquidity_rate,
            reserve_2_after_borrow_2.liquidity_rate
        );

        // *
        // Borrow coin 2 again (should fail due to insufficient collateral)
        // *
        let env = mock_env("borrower", &[]);
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin2"),
            amount: Uint256::from(10000 as u128),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay zero debt 2 (should fail)
        // *
        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay some debt 2
        // *
        let repay_amount = 2000u128;
        let block_time = reserve_2_after_borrow_2.interests_last_updated + 8000u64;
        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[coin(repay_amount, "borrowedcoin2")],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_params_2 = th_get_expected_indices_and_rates(
            &reserve_2_after_borrow_2,
            block_time,
            available_liquidity_2,
            TestUtilizationDeltas {
                less_debt: repay_amount,
                ..Default::default()
            },
        );

        assert_eq!(res.messages, vec![],);
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("reserve", "borrowedcoin2"),
                log("user", "borrower"),
                log("amount", repay_amount),
                log("borrow_index", expected_params_2.borrow_index),
                log("liquidity_index", expected_params_2.liquidity_index),
                log("borrow_rate", expected_params_2.borrow_rate),
                log("liquidity_rate", expected_params_2.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let reserve_2_after_repay_some_2 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin2")
            .unwrap();
        let expected_debt_scaled_2_after_repay_some_2 = expected_debt_scaled_2_after_borrow_2
            - Uint256::from(repay_amount) / expected_params_2.borrow_index;
        assert_eq!(
            expected_debt_scaled_2_after_repay_some_2,
            debt2.amount_scaled
        );
        assert_eq!(
            expected_debt_scaled_2_after_repay_some_2,
            reserve_2_after_repay_some_2.debt_total_scaled
        );
        assert_eq!(
            expected_params_2.borrow_rate,
            reserve_2_after_repay_some_2.borrow_rate
        );
        assert_eq!(
            expected_params_2.liquidity_rate,
            reserve_2_after_repay_some_2.liquidity_rate
        );

        // *
        // Repay all debt 2
        // *
        let block_time = reserve_2_after_repay_some_2.interests_last_updated + 10000u64;
        // need this to compute the repay amount
        let expected_params_2 = th_get_expected_indices_and_rates(
            &reserve_2_after_repay_some_2,
            block_time,
            available_liquidity_2,
            TestUtilizationDeltas {
                less_debt: 9999999999999, // hack: Just do a big number to repay all debt,
                ..Default::default()
            },
        );
        // TODO: There's a rounding error when multiplying a dividing by a Decimal256
        // probably because intermediate result is cast to Uint256. doing everything in Decimal256
        // eliminates this but need to then find a way to cast it back to an integer
        let repay_amount: u128 =
            (expected_debt_scaled_2_after_repay_some_2 * expected_params_2.borrow_index).into();

        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[coin(repay_amount, "borrowedcoin2")],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(res.messages, vec![],);
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("reserve", "borrowedcoin2"),
                log("user", "borrower"),
                log("amount", repay_amount),
                log("borrow_index", expected_params_2.borrow_index),
                log("liquidity_index", expected_params_2.liquidity_index),
                log("borrow_rate", expected_params_2.borrow_rate),
                log("liquidity_rate", expected_params_2.liquidity_rate),
            ]
        );

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let reserve_2_after_repay_all_2 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin2")
            .unwrap();

        assert_eq!(Uint256::zero(), debt2.amount_scaled);
        assert_eq!(
            Uint256::zero(),
            reserve_2_after_repay_all_2.debt_total_scaled
        );

        // *
        // Repay more debt 2 (should fail)
        // *
        let env = mock_env("borrower", &[coin(2000, "borrowedcoin2")]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay all debt 1 (and then some)
        // *
        let block_time = reserve_2_after_repay_all_2.interests_last_updated + 5000u64;
        let repay_amount = 4800u128;

        let expected_params_1 = th_get_expected_indices_and_rates(
            &reserve_1_after_borrow_again,
            block_time,
            available_liquidity_1,
            TestUtilizationDeltas {
                less_debt: repay_amount,
                ..Default::default()
            },
        );

        let env = th_mock_env(MockEnvParams {
            sender: "borrower",
            sent_funds: &[coin(repay_amount, "borrowedcoin1")],
            block_time: Some(block_time),
        });
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin1"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_repay_amount_scaled =
            Uint256::from(repay_amount) / expected_params_1.borrow_index;
        let expected_refund_amount: u128 = ((expected_repay_amount_scaled
            - expected_debt_scaled_1_after_borrow_again)
            * expected_params_1.borrow_index)
            .into();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: HumanAddr::from("borrower"),
                amount: vec![Coin {
                    denom: String::from("borrowedcoin1"),
                    amount: Uint128(expected_refund_amount),
                },],
            }),],
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("reserve", "borrowedcoin1"),
                log("user", "borrower"),
                log("amount", Uint128(repay_amount - expected_refund_amount)),
                log("borrow_index", expected_params_1.borrow_index),
                log("liquidity_index", expected_params_1.liquidity_index),
                log("borrow_rate", expected_params_1.borrow_rate),
                log("liquidity_rate", expected_params_1.liquidity_rate),
            ]
        );
        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(false, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt1 = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        let reserve_1_after_repay_1 = reserves_state_read(&deps.storage)
            .load(b"borrowedcoin1")
            .unwrap();
        assert_eq!(Uint256::from(0 as u128), debt1.amount_scaled);
        assert_eq!(
            Uint256::from(0 as u128),
            reserve_1_after_repay_1.debt_total_scaled
        );
    }

    // TEST HELPERS

    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        let msg = InitMsg {
            ma_token_code_id: 1u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        deps
    }

    #[derive(Default)]
    struct MockEnvParams<'a> {
        sender: &'a str,
        sent_funds: &'a [Coin],
        block_time: Option<u64>,
    }

    /// A custom mock env to make it more cofigurable than the one provided by cosmwasm_std
    fn th_mock_env(mock_env: MockEnvParams) -> Env {
        let block_time = mock_env.block_time.unwrap_or(1_571_797_419);

        Env {
            block: BlockInfo {
                height: 12_345,
                time: block_time,
                chain_id: "cosmos-testnet-14002".to_string(),
            },
            message: MessageInfo {
                sender: HumanAddr::from(mock_env.sender),
                sent_funds: mock_env.sent_funds.to_vec(),
            },
            contract: ContractInfo {
                address: HumanAddr::from(MOCK_CONTRACT_ADDR),
            },
        }
    }

    #[derive(Default, Debug)]
    struct MockReserve<'a> {
        ma_token_address: &'a str,
        liquidity_index: Decimal256,
        borrow_index: Decimal256,

        borrow_rate: Decimal256,
        liquidity_rate: Decimal256,

        borrow_slope: Decimal256,
        loan_to_value: Decimal256,

        interests_last_updated: u64,
        debt_total_scaled: Uint256,
    }

    fn th_init_reserve<S: Storage, A: Api>(
        api: &A,
        storage: &mut S,
        key: &[u8],
        reserve: &MockReserve,
    ) -> Reserve {
        let mut index = 0;

        config_state(storage)
            .update(|mut c: Config| -> StdResult<Config> {
                index = c.reserve_count;
                c.reserve_count += 1;
                Ok(c)
            })
            .unwrap();

        let mut reserve_bucket = reserves_state(storage);
        let new_reserve = Reserve {
            ma_token_address: api
                .canonical_address(&HumanAddr::from(reserve.ma_token_address))
                .unwrap(),
            index,
            borrow_index: reserve.borrow_index,
            liquidity_index: reserve.liquidity_index,
            borrow_rate: reserve.borrow_rate,
            liquidity_rate: reserve.liquidity_rate,
            borrow_slope: reserve.borrow_slope,
            loan_to_value: reserve.loan_to_value,
            interests_last_updated: reserve.interests_last_updated,
            debt_total_scaled: reserve.debt_total_scaled,
        };

        reserve_bucket.save(key, &new_reserve).unwrap();

        reserve_denoms_state(storage)
            .save(
                &index.to_be_bytes(),
                &ReserveDenoms {
                    denom: String::from_utf8(key.to_vec()).unwrap(),
                },
            )
            .unwrap();

        new_reserve
    }

    #[derive(Default, Debug)]
    struct TestInterestResults {
        borrow_index: Decimal256,
        liquidity_index: Decimal256,
        borrow_rate: Decimal256,
        liquidity_rate: Decimal256,
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
    fn th_get_expected_indices_and_rates(
        reserve: &Reserve,
        block_time: u64,
        initial_liquidity: u128,
        deltas: TestUtilizationDeltas,
    ) -> TestInterestResults {
        let seconds_elapsed = block_time - reserve.interests_last_updated;

        // market indices
        let expected_accumulated_liquidity_interest = Decimal256::one()
            + (reserve.liquidity_rate * Decimal256::from_uint256(seconds_elapsed)
                / Decimal256::from_uint256(SECONDS_PER_YEAR));
        let expected_liquidity_index =
            reserve.liquidity_index * expected_accumulated_liquidity_interest;

        let expected_accumulated_borrow_interest = Decimal256::one()
            + (reserve.borrow_rate * Decimal256::from_uint256(seconds_elapsed)
                / Decimal256::from_uint256(SECONDS_PER_YEAR));
        let expected_borrow_index = reserve.borrow_index * expected_accumulated_borrow_interest;

        // When borrowing, new computed index is used for scaled amount
        let more_debt_scaled = Uint256::from(deltas.more_debt) / expected_borrow_index;
        // When repaying, new computed index is used for scaled amount
        let less_debt_scaled = Uint256::from(deltas.less_debt) / expected_borrow_index;
        let mut new_debt_total = Uint256::zero();
        // NOTE: Don't panick here so that the total repay of debt can be simulated
        // when less debt is greater than outstanding debt
        // TODO: Maybe split index and interest rate calculations and take the needed inputs
        // for each
        if (reserve.debt_total_scaled + more_debt_scaled) > less_debt_scaled {
            new_debt_total = reserve.debt_total_scaled + more_debt_scaled - less_debt_scaled;
        }
        let dec_debt_total = Decimal256::from_uint256(new_debt_total) * expected_borrow_index;
        let dec_liquidity_total =
            Decimal256::from_uint256(initial_liquidity - deltas.less_liquidity);
        let expected_utilization_rate = dec_debt_total / (dec_liquidity_total + dec_debt_total);

        // interest rates
        let expected_borrow_rate = expected_utilization_rate * reserve.borrow_slope;
        let expected_liquidity_rate = expected_borrow_rate * expected_utilization_rate;

        TestInterestResults {
            borrow_index: expected_borrow_index,
            liquidity_index: expected_liquidity_index,
            borrow_rate: expected_borrow_rate,
            liquidity_rate: expected_liquidity_rate,
        }
    }
}
