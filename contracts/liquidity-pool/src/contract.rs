use cosmwasm_bignumber::{Decimal256, Uint256};
use cosmwasm_std::{
    from_binary, log, to_binary, Api, BankMsg, Binary, CanonicalAddr, Coin, CosmosMsg, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Order, Querier,
    StdError, StdResult, Storage, Uint128, WasmMsg,
};

use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::ma_token;

use crate::msg::{
    ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg, ReceiveMsg, ReserveInfo,
    ReserveResponse, ReservesListResponse,
};
use crate::state::{
    config_state, config_state_read, debts_asset_state, reserves_state, reserves_state_read,
    users_state, Config, Debt, Reserve, User,
};

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
        HandleMsg::InitAsset { denom } => init_asset(deps, env, denom),
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
                redeem_native(deps, env, id, reserve, cw20_msg.sender, cw20_msg.amount)
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
    reserve: Reserve,
    to: HumanAddr,
    burn_amount: Uint128,
) -> StdResult<HandleResponse> {
    // TODO: Recompute interest rates
    // TODO: Check the withdraw can actually be made
    let redeem_amount = Uint256::from(burn_amount) * reserve.liquidity_index;

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&reserve.ma_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: burn_amount,
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
        log: vec![
            log("action", "redeem"),
            log("reserve", denom),
            log("user", to),
            log("burn_amount", burn_amount),
            log("redeem_amount", redeem_amount),
        ],
        data: None,
    })
}

/// Initialize asset so it can be deposited and borrowed.
/// A new maToken should be created which callbacks this contract in order to be registered
pub fn init_asset<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    denom: String,
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
                    ma_token_address: CanonicalAddr::default(),
                    liquidity_index: Decimal256::one(),
                    index: config.reserve_count,
                    borrow_index: Decimal256::one(),
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
    let reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;

    // Get deposit amount
    // TODO: asumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    let deposit_amount = env
        .message
        .sent_funds
        .iter()
        .find(|c| c.denom == denom)
        .map(|c| Uint256::from(c.amount))
        .unwrap_or_else(Uint256::zero);

    // Cannot deposit zero amount
    if deposit_amount.is_zero() {
        return Err(StdError::generic_err(format!(
            "Deposit amount must be greater than 0 {}",
            denom,
        )));
    }

    // TODO: Interest rate update and computing goes here

    let mint_amount = deposit_amount / reserve.liquidity_index;

    Ok(HandleResponse {
        data: None,
        log: vec![
            log("action", "deposit"),
            log("reserve", denom),
            log("user", env.message.sender.clone()),
            log("amount", deposit_amount),
        ],
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

    let reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;
    let mut users_bucket = users_state(&mut deps.storage);
    let borrower_addr = deps.api.canonical_address(&env.message.sender)?;
    let mut user: User = match users_bucket.may_load(borrower_addr.as_slice()) {
        Ok(Some(user)) => user,
        Ok(None) => User {
            borrowed_assets: Uint128(0),
        },
        Err(error) => return Err(error),
    };

    // TODO: Interest rate update and computing goes somwhere around here
    // TODO: Check the user can actually borrow (has enough collateral, contract has
    // enough funds to safely lend them)

    let is_borrowing_asset = get_bit(user.borrowed_assets, reserve.index)?;
    if !is_borrowing_asset {
        set_bit(&mut user.borrowed_assets, reserve.index)?;
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
    let debt_amount = borrow_amount / reserve.borrow_index;
    debt.amount_scaled += debt_amount;
    debts_asset_bucket.save(borrower_addr.as_slice(), &debt)?;

    Ok(HandleResponse {
        data: None,
        log: vec![
            log("action", "borrow"),
            log("reserve", denom.clone()),
            log("user", env.message.sender.clone()),
            log("amount", borrow_amount),
        ],
        messages: vec![CosmosMsg::Bank(BankMsg::Send {
            from_address: env.contract.address,
            to_address: env.message.sender,
            amount: vec![Coin {
                denom: denom,
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
    // TODO: asumes this will always be in 10^6 amounts (i.e: uluna, or uusd)
    // but double check that's the case
    let reserve = reserves_state_read(&deps.storage).load(denom.as_bytes())?;

    // Get repay amount
    // TODO: Evaluate refunding the rest of the coins sent (or failing if more
    // than one coin sent)
    let repay_amount = get_denom_amount_from_coins(env.message.sent_funds, &denom);

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

    let mut repay_amount_scaled = repay_amount / reserve.borrow_index;

    let mut messages = vec![];
    let mut refund_amount = Uint256::zero();
    if repay_amount_scaled > debt.amount_scaled {
        // refund any excess amounts
        // TODO: Should we log this?
        refund_amount = (repay_amount_scaled - debt.amount_scaled) * reserve.borrow_index;
        messages.push(CosmosMsg::Bank(BankMsg::Send {
            from_address: env.contract.address,
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

    if debt.amount_scaled == Uint256::zero() {
        // Remove asset from borrowed assets
        let mut users_bucket = users_state(&mut deps.storage);
        let mut user = users_bucket.load(borrower_addr.as_slice())?;
        unset_bit(&mut user.borrowed_assets, reserve.index)?;
        users_bucket.save(borrower_addr.as_slice(), &user)?;
    }

    Ok(HandleResponse {
        data: None,
        log: vec![
            log("action", "repay"),
            log("reserve", denom),
            log("user", env.message.sender.clone()),
            log("amount", repay_amount - refund_amount),
        ],
        messages: messages,
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

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// HELPERS
// native coins
fn get_denom_amount_from_coins(coins: Vec<Coin>, denom: &str) -> Uint256 {
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
    use crate::state::{debts_asset_state_read, users_state_read};
    use cosmwasm_std::testing::{
        mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR,
    };
    use cosmwasm_std::{coin, from_binary, Extern};

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
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // owner is authorized
        // *
        let env = mock_env("owner", &[]);
        let msg = HandleMsg::InitAsset {
            denom: String::from("someasset"),
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
        let mut deps = th_setup();

        let env = mock_env("mtokencontract", &[]);
        let msg = HandleMsg::InitAssetTokenCallback {
            id: String::from("uluna"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_deposit_native_asset() {
        let mut deps = th_setup();

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(11, 10),
            ..Default::default()
        };
        th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", mock_reserve);

        let env = mock_env("depositer", &[coin(110000, "somecoin")]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let res = handle(&mut deps, env, msg).unwrap();
        // mints coin_amount/liquidity_index
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("matoken"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("depositer"),
                    amount: Uint128(100000),
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
                log("amount", "110000"),
            ]
        );

        // empty deposit fails
        let env = mock_env("depositer", &[]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_cannot_deposit_if_no_reserve() {
        let mut deps = th_setup();

        let env = mock_env("depositer", &[coin(110000, "somecoin")]);
        let msg = HandleMsg::DepositNative {
            denom: String::from("somecoin"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();
    }

    #[test]
    fn test_redeem_native() {
        let mut deps = th_setup();

        let mock_reserve = MockReserve {
            ma_token_address: "matoken",
            liquidity_index: Decimal256::from_ratio(15, 10),
            ..Default::default()
        };

        th_init_reserve(&deps.api, &mut deps.storage, b"somecoin", mock_reserve);

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
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("matoken"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: Uint128(2000),
                    })
                    .unwrap(),
                }),
                CosmosMsg::Bank(BankMsg::Send {
                    from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    to_address: HumanAddr::from("redeemer"),
                    amount: vec![Coin {
                        denom: String::from("somecoin"),
                        amount: Uint128(3000),
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
                log("burn_amount", 2000),
                log("redeem_amount", 3000),
            ]
        );
    }

    #[test]
    fn test_borrow_and_repay_native() {
        let mut deps = th_setup();

        let mock_reserve_1 = MockReserve {
            ma_token_address: "matoken1",
            borrow_index: Decimal256::from_ratio(12, 10),
            ..Default::default()
        };
        let mock_reserve_2 = MockReserve {
            ma_token_address: "matoken2",
            borrow_index: Decimal256::one(),
            ..Default::default()
        };

        // should get index 0
        th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoin1",
            mock_reserve_1,
        );
        // shoudl get index 1
        th_init_reserve(
            &deps.api,
            &mut deps.storage,
            b"borrowedcoin2",
            mock_reserve_2,
        );

        // *
        // Borrow coin 1
        // *
        let env = mock_env("borrower", &[]);
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin1"),
            amount: Uint256::from(2400 as u128),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // check correct messages and logging
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: HumanAddr::from("borrower"),
                amount: vec![Coin {
                    denom: String::from("borrowedcoin1"),
                    amount: Uint128(2400),
                },],
            }),]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "borrow"),
                log("reserve", "borrowedcoin1"),
                log("user", "borrower"),
                log("amount", 2400),
            ]
        );

        let borrower_addr_canonical = deps
            .api
            .canonical_address(&HumanAddr::from("borrower"))
            .unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());

        let debt = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();

        assert_eq!(Uint256::from(2000 as u128), debt.amount_scaled);

        // *
        // Borrow coin 1 (again)
        // *
        let env = mock_env("borrower", &[]);
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin1"),
            amount: Uint256::from(1200 as u128),
        };
        let _res = handle(&mut deps, env, msg).unwrap();
        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());
        let debt = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();

        assert_eq!(Uint256::from(3000 as u128), debt.amount_scaled);

        // *
        // Borrow coin 2
        // *
        let env = mock_env("borrower", &[]);
        let msg = HandleMsg::BorrowNative {
            denom: String::from("borrowedcoin2"),
            amount: Uint256::from(4000 as u128),
        };
        let _res = handle(&mut deps, env, msg).unwrap();
        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());
        let debt1 = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(3000 as u128), debt1.amount_scaled);
        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(4000 as u128), debt2.amount_scaled);

        // *
        // Repay zero debt 2 (should fail)
        // *
        let env = mock_env("borrower", &[]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // *
        // Repay some debt 2
        // *
        let env = mock_env("borrower", &[coin(2000, "borrowedcoin2")]);
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
                log("amount", 2000),
            ]
        );
        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(true, get_bit(user.borrowed_assets, 1).unwrap());
        let debt1 = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(3000 as u128), debt1.amount_scaled);
        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(2000 as u128), debt2.amount_scaled);

        // *
        // Repay all debt 2
        // *
        let env = mock_env("borrower", &[coin(2000, "borrowedcoin2")]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin2"),
        };
        let _res = handle(&mut deps, env, msg).unwrap();

        let user = users_state_read(&deps.storage)
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(true, get_bit(user.borrowed_assets, 0).unwrap());
        assert_eq!(false, get_bit(user.borrowed_assets, 1).unwrap());
        let debt1 = debts_asset_state_read(&deps.storage, b"borrowedcoin1")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(3000 as u128), debt1.amount_scaled);
        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(0 as u128), debt2.amount_scaled);

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
        let env = mock_env("borrower", &[coin(4800, "borrowedcoin1")]);
        let msg = HandleMsg::RepayNative {
            denom: String::from("borrowedcoin1"),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Bank(BankMsg::Send {
                from_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                to_address: HumanAddr::from("borrower"),
                amount: vec![Coin {
                    denom: String::from("borrowedcoin1"),
                    amount: Uint128(1200), // scaled * borrow_index = 1000 * 1.2
                },],
            }),],
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "repay"),
                log("reserve", "borrowedcoin1"),
                log("user", "borrower"),
                log("amount", 3600),
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
        assert_eq!(Uint256::from(0 as u128), debt1.amount_scaled);
        let debt2 = debts_asset_state_read(&deps.storage, b"borrowedcoin2")
            .load(&borrower_addr_canonical.as_slice())
            .unwrap();
        assert_eq!(Uint256::from(0 as u128), debt2.amount_scaled);
    }

    // TEST HELPERS
    #[derive(Default)]
    struct MockReserve<'a> {
        ma_token_address: &'a str,
        liquidity_index: Decimal256,
        borrow_index: Decimal256,
    }

    fn th_setup() -> Extern<MockStorage, MockApi, MockQuerier> {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            ma_token_code_id: 1u64,
        };
        let env = mock_env("owner", &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        deps
    }

    fn th_init_reserve<S: Storage, A: Api>(
        api: &A,
        storage: &mut S,
        key: &[u8],
        reserve: MockReserve,
    ) {
        let mut index = 0;

        config_state(storage)
            .update(|mut c: Config| -> StdResult<Config> {
                index = c.reserve_count;
                c.reserve_count += 1;
                Ok(c)
            })
            .unwrap();

        let mut reserve_bucket = reserves_state(storage);
        reserve_bucket
            .save(
                key,
                &Reserve {
                    ma_token_address: api
                        .canonical_address(&HumanAddr::from(reserve.ma_token_address))
                        .unwrap(),
                    index: index,
                    liquidity_index: reserve.liquidity_index,
                    borrow_index: reserve.borrow_index,
                },
            )
            .unwrap();
    }
}
