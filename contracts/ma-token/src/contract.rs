use cosmwasm_std::{
    log, to_binary, Api, Binary, Env, Extern, HandleResponse, HumanAddr, InitResponse,
    MigrateResponse, Querier, StdError, StdResult, Storage, Uint128,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{BalanceResponse, Cw20CoinHuman, Cw20ReceiveMsg, MinterResponse, TokenInfoResponse};

use crate::allowances::{
    handle_decrease_allowance, handle_increase_allowance, handle_transfer_from, query_allowance,
};
use crate::core;
use crate::enumerable::{query_all_accounts, query_all_allowances};
use crate::state;
use crate::state::{
    balances, balances_read, token_info, token_info_read, Config, MinterData, TokenInfo,
};
use mars::ma_token::msg::{
    BalanceAndTotalSupplyResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg,
};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:ma-token";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    set_contract_version(&mut deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    // check valid token info
    msg.validate()?;
    // create initial accounts
    let total_supply = create_accounts(deps, &msg.initial_balances)?;

    if let Some(limit) = msg.get_cap() {
        if total_supply > limit {
            return Err(StdError::generic_err("Initial supply greater than cap"));
        }
    }

    let mint = match msg.mint {
        Some(m) => Some(MinterData {
            minter: deps.api.canonical_address(&m.minter)?,
            cap: m.cap,
        }),
        None => None,
    };

    // store token info
    let data = TokenInfo {
        name: msg.name,
        symbol: msg.symbol,
        decimals: msg.decimals,
        total_supply,
        mint,
    };
    token_info(&mut deps.storage).save(&data)?;

    state::save_config(
        &mut deps.storage,
        &Config {
            red_bank_address: deps.api.canonical_address(&msg.red_bank_address)?,
            incentives_address: deps.api.canonical_address(&msg.incentives_address)?,
        },
    )?;

    // store token config
    Ok(InitResponse::default())
}

pub fn create_accounts<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    accounts: &[Cw20CoinHuman],
) -> StdResult<Uint128> {
    let mut total_supply = Uint128::zero();
    let mut store = balances(&mut deps.storage);
    for row in accounts {
        let raw_address = deps.api.canonical_address(&row.address)?;
        store.save(raw_address.as_slice(), &row.amount)?;
        total_supply += row.amount;
    }
    Ok(total_supply)
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Transfer { recipient, amount } => handle_transfer(deps, env, recipient, amount),
        HandleMsg::TransferOnLiquidation {
            sender,
            recipient,
            amount,
        } => handle_transfer_on_liquidation(deps, env, sender, recipient, amount),
        HandleMsg::Burn { user, amount } => handle_burn(deps, env, user, amount),
        HandleMsg::Send {
            contract,
            amount,
            msg,
        } => handle_send(deps, env, contract, amount, msg),
        HandleMsg::Mint { recipient, amount } => handle_mint(deps, env, recipient, amount),
        HandleMsg::IncreaseAllowance {
            spender,
            amount,
            expires,
        } => handle_increase_allowance(deps, env, spender, amount, expires),
        HandleMsg::DecreaseAllowance {
            spender,
            amount,
            expires,
        } => handle_decrease_allowance(deps, env, spender, amount, expires),
        HandleMsg::TransferFrom {
            owner,
            recipient,
            amount,
        } => handle_transfer_from(deps, env, owner, recipient, amount),
        // TODO: Bring back SendFrom (Won't cause troubles anymore witht he new withdraw API)
    }
}

pub fn handle_transfer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    let config = state::load_config(&deps.storage)?;
    let messages = core::transfer(deps, &config, &env.message.sender, &recipient, amount, true)?;

    let res = HandleResponse {
        messages,
        log: vec![
            log("action", "transfer"),
            log("from", env.message.sender),
            log("to", recipient),
            log("amount", amount),
        ],
        data: None,
    };
    Ok(res)
}

pub fn handle_transfer_on_liquidation<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    sender: HumanAddr,
    recipient: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    // only money market can call
    let config = state::load_config(&deps.storage)?;
    if deps.api.canonical_address(&env.message.sender)? != config.red_bank_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    let messages = core::transfer(deps, &config, &sender, &recipient, amount, false)?;

    let res = HandleResponse {
        messages,
        log: vec![
            log("action", "transfer_on_liquidation"),
            log("from", env.message.sender),
            log("to", recipient),
            log("amount", amount),
        ],
        data: None,
    };
    Ok(res)
}

pub fn handle_burn<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    user: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    // only money market can burn
    let config = state::load_config(&deps.storage)?;
    if deps.api.canonical_address(&env.message.sender)? != config.red_bank_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    let user_raw = deps.api.canonical_address(&user)?;

    // lower balance
    let mut accounts = balances(&mut deps.storage);
    let user_balance_before = accounts.load(user_raw.as_slice()).unwrap_or_default();
    let user_balance_new = (user_balance_before - amount)?;
    accounts.save(user_raw.as_slice(), &user_balance_new)?;

    // reduce total_supply
    let mut total_supply_before = Uint128::zero();
    token_info(&mut deps.storage).update(|mut info| {
        total_supply_before = info.total_supply;
        info.total_supply = (info.total_supply - amount)?;
        Ok(info)
    })?;

    let res = HandleResponse {
        messages: vec![core::balance_change_msg(
            &deps.api,
            &config.incentives_address,
            user.clone(),
            user_balance_before,
            total_supply_before,
        )?],
        log: vec![
            log("action", "burn"),
            log("user", user),
            log("amount", amount),
        ],
        data: None,
    };
    Ok(res)
}

pub fn handle_mint<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    let mut info = token_info_read(&deps.storage).load()?;
    if info.mint.is_none()
        || info.mint.as_ref().unwrap().minter != deps.api.canonical_address(&env.message.sender)?
    {
        return Err(StdError::unauthorized());
    }
    let total_supply_before = info.total_supply;

    // update supply and enforce cap
    info.total_supply += amount;
    if let Some(limit) = info.get_cap() {
        if info.total_supply > limit {
            return Err(StdError::generic_err("Minting cannot exceed the cap"));
        }
    }
    token_info(&mut deps.storage).save(&info)?;

    // add amount to recipient balance
    let mut accounts = balances(&mut deps.storage);
    let rcpt_raw = deps.api.canonical_address(&recipient)?;
    let rcpt_balance_before = accounts.load(rcpt_raw.as_slice()).unwrap_or_default();
    let rcpt_balance_new = rcpt_balance_before + amount;
    accounts.save(rcpt_raw.as_slice(), &rcpt_balance_new)?;

    let config = state::load_config(&deps.storage)?;

    let res = HandleResponse {
        messages: vec![core::balance_change_msg(
            &deps.api,
            &config.incentives_address,
            recipient.clone(),
            rcpt_balance_before,
            total_supply_before,
        )?],
        log: vec![
            log("action", "mint"),
            log("to", recipient),
            log("amount", amount),
        ],
        data: None,
    };
    Ok(res)
}

pub fn handle_send<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    contract: HumanAddr,
    amount: Uint128,
    msg: Option<Binary>,
) -> StdResult<HandleResponse> {
    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    // move the tokens to the contract
    let sender = env.message.sender;
    let config = state::load_config(&deps.storage)?;
    let mut messages = core::transfer(deps, &config, &sender, &contract, amount, true)?;

    let logs = vec![
        log("action", "send"),
        log("from", &sender),
        log("to", &contract),
        log("amount", amount),
    ];

    // create a send message
    messages.push(
        Cw20ReceiveMsg {
            sender,
            amount,
            msg,
        }
        .into_cosmos_msg(contract)?,
    );

    let res = HandleResponse {
        messages,
        log: logs,
        data: None,
    };

    Ok(res)
}

// QUERY

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Balance { address } => to_binary(&query_balance(deps, address)?),
        QueryMsg::BalanceAndTotalSupply { address } => {
            to_binary(&query_balance_and_total_supply(deps, address)?)
        }
        QueryMsg::TokenInfo {} => to_binary(&query_token_info(deps)?),
        QueryMsg::Minter {} => to_binary(&query_minter(deps)?),
        QueryMsg::Allowance { owner, spender } => {
            to_binary(&query_allowance(deps, owner, spender)?)
        }
        QueryMsg::AllAllowances {
            owner,
            start_after,
            limit,
        } => to_binary(&query_all_allowances(deps, owner, start_after, limit)?),
        QueryMsg::AllAccounts { start_after, limit } => {
            to_binary(&query_all_accounts(deps, start_after, limit)?)
        }
    }
}

pub fn query_balance<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<BalanceResponse> {
    let addr_raw = deps.api.canonical_address(&address)?;
    let balance = balances_read(&deps.storage)
        .may_load(addr_raw.as_slice())?
        .unwrap_or_default();
    Ok(BalanceResponse { balance })
}

pub fn query_balance_and_total_supply<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<BalanceAndTotalSupplyResponse> {
    let addr_raw = deps.api.canonical_address(&address)?;
    let balance = balances_read(&deps.storage)
        .may_load(addr_raw.as_slice())?
        .unwrap_or_default();
    let info = token_info_read(&deps.storage).load()?;
    Ok(BalanceAndTotalSupplyResponse {
        balance,
        total_supply: info.total_supply,
    })
}

pub fn query_token_info<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<TokenInfoResponse> {
    let info = token_info_read(&deps.storage).load()?;
    let res = TokenInfoResponse {
        name: info.name,
        symbol: info.symbol,
        decimals: info.decimals,
        total_supply: info.total_supply,
    };
    Ok(res)
}

pub fn query_minter<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<Option<MinterResponse>> {
    let meta = token_info_read(&deps.storage).load()?;
    let minter = match meta.mint {
        Some(m) => Some(MinterResponse {
            minter: deps.api.human_address(&m.minter)?,
            cap: m.cap,
        }),
        None => None,
    };
    Ok(minter)
}

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> StdResult<MigrateResponse> {
    let old_version = get_contract_version(&deps.storage)?;
    if old_version.contract != CONTRACT_NAME {
        return Err(StdError::generic_err(format!(
            "This is {}, cannot migrate from {}",
            CONTRACT_NAME, old_version.contract
        )));
    }
    Err(StdError::generic_err(format!(
        "Unknown version {}",
        old_version.version
    )))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_env};
    use cosmwasm_std::{coins, from_binary, CosmosMsg, StdError, WasmMsg};

    use super::*;
    use mars::testing::assert_generic_error_message;

    const CANONICAL_LENGTH: usize = 20;

    fn get_balance<S: Storage, A: Api, Q: Querier, T: Into<HumanAddr>>(
        deps: &Extern<S, A, Q>,
        address: T,
    ) -> Uint128 {
        query_balance(&deps, address.into()).unwrap().balance
    }

    // this will set up the init for other tests
    fn do_init_with_minter<S: Storage, A: Api, Q: Querier>(
        deps: &mut Extern<S, A, Q>,
        addr: &HumanAddr,
        amount: Uint128,
        minter: &HumanAddr,
        cap: Option<Uint128>,
    ) -> TokenInfoResponse {
        _do_init(
            deps,
            addr,
            amount,
            Some(MinterResponse {
                minter: minter.into(),
                cap,
            }),
        )
    }

    // this will set up the init for other tests
    fn do_init<S: Storage, A: Api, Q: Querier>(
        deps: &mut Extern<S, A, Q>,
        addr: &HumanAddr,
        amount: Uint128,
    ) -> TokenInfoResponse {
        _do_init(deps, addr, amount, None)
    }

    // this will set up the init for other tests
    fn _do_init<S: Storage, A: Api, Q: Querier>(
        deps: &mut Extern<S, A, Q>,
        addr: &HumanAddr,
        amount: Uint128,
        mint: Option<MinterResponse>,
    ) -> TokenInfoResponse {
        let init_msg = InitMsg {
            name: "Auto Gen".to_string(),
            symbol: "AUTO".to_string(),
            decimals: 3,
            initial_balances: vec![Cw20CoinHuman {
                address: addr.into(),
                amount,
            }],
            mint: mint.clone(),
            red_bank_address: HumanAddr::from("red_bank"),
            incentives_address: HumanAddr::from("incentives"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(deps, env, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        let meta = query_token_info(&deps).unwrap();
        assert_eq!(
            meta,
            TokenInfoResponse {
                name: "Auto Gen".to_string(),
                symbol: "AUTO".to_string(),
                decimals: 3,
                total_supply: amount,
            }
        );
        assert_eq!(get_balance(&deps, addr), amount);
        assert_eq!(query_minter(&deps).unwrap(), mint,);
        meta
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        let amount = Uint128::from(11223344u128);
        let red_bank_address = HumanAddr::from("red_bank");

        let init_msg = InitMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20CoinHuman {
                address: HumanAddr("addr0000".to_string()),
                amount,
            }],
            mint: None,
            red_bank_address: red_bank_address.clone(),
            incentives_address: HumanAddr::from("incentives"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(&mut deps, env, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(&deps).unwrap(),
            TokenInfoResponse {
                name: "Cash Token".to_string(),
                symbol: "CASH".to_string(),
                decimals: 9,
                total_supply: amount,
            }
        );
        assert_eq!(get_balance(&deps, "addr0000"), Uint128(11223344));
        let config = state::load_config(&deps.storage).unwrap();
        assert_eq!(
            config.red_bank_address,
            deps.api.canonical_address(&red_bank_address).unwrap()
        );
    }

    #[test]
    fn init_mintable() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        let amount = Uint128(11223344);
        let minter = HumanAddr::from("asmodat");
        let limit = Uint128(511223344);
        let init_msg = InitMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20CoinHuman {
                address: HumanAddr("addr0000".to_string()),
                amount,
            }],
            mint: Some(MinterResponse {
                minter: minter.clone(),
                cap: Some(limit),
            }),
            red_bank_address: HumanAddr::from("red_bank"),
            incentives_address: HumanAddr::from("incentives"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(&mut deps, env, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(&deps).unwrap(),
            TokenInfoResponse {
                name: "Cash Token".to_string(),
                symbol: "CASH".to_string(),
                decimals: 9,
                total_supply: amount,
            }
        );
        assert_eq!(get_balance(&deps, "addr0000"), Uint128(11223344));
        assert_eq!(
            query_minter(&deps).unwrap(),
            Some(MinterResponse {
                minter,
                cap: Some(limit)
            }),
        );
    }

    #[test]
    fn init_mintable_over_cap() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        let amount = Uint128(11223344);
        let minter = HumanAddr::from("asmodat");
        let limit = Uint128(11223300);
        let init_msg = InitMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20CoinHuman {
                address: HumanAddr("addr0000".to_string()),
                amount,
            }],
            mint: Some(MinterResponse {
                minter,
                cap: Some(limit),
            }),
            red_bank_address: HumanAddr::from("red_bank"),
            incentives_address: HumanAddr::from("incentives"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(&mut deps, env, init_msg);
        assert_generic_error_message(res, "Initial supply greater than cap");
    }

    #[test]
    fn can_mint_by_minter() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);

        let genesis = HumanAddr::from("genesis");
        let amount = Uint128(11223344);
        let minter = HumanAddr::from("asmodat");
        let limit = Uint128(511223344);
        do_init_with_minter(&mut deps, &genesis, amount, &minter, Some(limit));

        // minter can mint coins to some winner
        let winner = HumanAddr::from("lucky");
        let prize = Uint128(222_222_222);
        let msg = HandleMsg::Mint {
            recipient: winner.clone(),
            amount: prize,
        };

        let env = mock_env(&minter, &[]);
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("incentives"),
                msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                    user_address: winner.clone(),
                    user_balance_before: Uint128::zero(),
                    total_supply_before: amount,
                },)
                .unwrap(),
                send: vec![],
            }),]
        );
        assert_eq!(get_balance(&deps, &genesis), amount);
        assert_eq!(get_balance(&deps, &winner), prize);
        assert_eq!(
            query_token_info(&deps).unwrap().total_supply,
            amount + prize
        );

        // but cannot mint nothing
        let msg = HandleMsg::Mint {
            recipient: winner.clone(),
            amount: Uint128::zero(),
        };
        let env = mock_env(&minter, &[]);
        let res = handle(&mut deps, env, msg);
        assert_generic_error_message(res, "Invalid zero amount");

        // but if it exceeds cap (even over multiple rounds), it fails
        // cap is enforced
        let msg = HandleMsg::Mint {
            recipient: winner,
            amount: Uint128(333_222_222),
        };
        let env = mock_env(&minter, &[]);
        let res = handle(&mut deps, env, msg);
        assert_generic_error_message(res, "Minting cannot exceed the cap");
    }

    #[test]
    fn others_cannot_mint() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        do_init_with_minter(
            &mut deps,
            &HumanAddr::from("genesis"),
            Uint128(1234),
            &HumanAddr::from("minter"),
            None,
        );

        let msg = HandleMsg::Mint {
            recipient: HumanAddr::from("lucky"),
            amount: Uint128(222),
        };
        let env = mock_env(&HumanAddr::from("anyone else"), &[]);
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::unauthorized());
    }

    #[test]
    fn no_one_mints_if_minter_unset() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        do_init(&mut deps, &HumanAddr::from("genesis"), Uint128(1234));

        let msg = HandleMsg::Mint {
            recipient: HumanAddr::from("lucky"),
            amount: Uint128(222),
        };
        let env = mock_env(&HumanAddr::from("genesis"), &[]);
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::unauthorized());
    }

    #[test]
    fn init_multiple_accounts() {
        let mut deps = mock_dependencies(CANONICAL_LENGTH, &[]);
        let amount1 = Uint128::from(11223344u128);
        let addr1 = HumanAddr::from("addr0001");
        let amount2 = Uint128::from(7890987u128);
        let addr2 = HumanAddr::from("addr0002");
        let init_msg = InitMsg {
            name: "Bash Shell".to_string(),
            symbol: "BASH".to_string(),
            decimals: 6,
            initial_balances: vec![
                Cw20CoinHuman {
                    address: addr1.clone(),
                    amount: amount1,
                },
                Cw20CoinHuman {
                    address: addr2.clone(),
                    amount: amount2,
                },
            ],
            mint: None,
            red_bank_address: HumanAddr::from("red_bank"),
            incentives_address: HumanAddr::from("incentives"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(&mut deps, env, init_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(&deps).unwrap(),
            TokenInfoResponse {
                name: "Bash Shell".to_string(),
                symbol: "BASH".to_string(),
                decimals: 6,
                total_supply: amount1 + amount2,
            }
        );
        assert_eq!(get_balance(&deps, &addr1), amount1);
        assert_eq!(get_balance(&deps, &addr2), amount2);
    }

    #[test]
    fn queries_work() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let addr1 = HumanAddr::from("addr0001");
        let amount1 = Uint128::from(12340000u128);

        let expected = do_init(&mut deps, &addr1, amount1);

        // check meta query
        let loaded = query_token_info(&deps).unwrap();
        assert_eq!(expected, loaded);

        // check balance query (full)
        let data = query(&deps, QueryMsg::Balance { address: addr1 }).unwrap();
        let loaded: BalanceResponse = from_binary(&data).unwrap();
        assert_eq!(loaded.balance, amount1);

        // check balance query (empty)
        let data = query(
            &deps,
            QueryMsg::Balance {
                address: HumanAddr::from("addr0002"),
            },
        )
        .unwrap();
        let loaded: BalanceResponse = from_binary(&data).unwrap();
        assert_eq!(loaded.balance, Uint128::zero());
    }

    #[test]
    fn transfer() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let addr1 = HumanAddr::from("addr0001");
        let addr2 = HumanAddr::from("addr0002");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_init(&mut deps, &addr1, amount1);

        // cannot transfer nothing
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Transfer {
            recipient: addr2.clone(),
            amount: Uint128::zero(),
        };
        let res = handle(&mut deps, env, msg);
        assert_generic_error_message(res, "Invalid zero amount");

        // cannot send more than we have
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Transfer {
            recipient: addr2.clone(),
            amount: too_much,
        };
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Underflow { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }

        // cannot send from empty account
        let env = mock_env(addr2.clone(), &[]);
        let msg = HandleMsg::Transfer {
            recipient: addr1.clone(),
            amount: transfer,
        };
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Underflow { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }

        // valid transfer
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Transfer {
            recipient: addr2.clone(),
            amount: transfer,
        };
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("red_bank"),
                    msg: to_binary(
                        &mars::liquidity_pool::msg::HandleMsg::FinalizeLiquidityTokenTransfer {
                            sender_address: addr1.clone(),
                            recipient_address: addr2.clone(),
                            sender_previous_balance: amount1,
                            recipient_previous_balance: Uint128::zero(),
                            amount: transfer,
                        }
                    )
                    .unwrap(),
                    send: vec![],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                        user_address: addr1.clone(),
                        user_balance_before: amount1,
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    send: vec![],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                        user_address: addr2.clone(),
                        user_balance_before: Uint128::zero(),
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    send: vec![],
                }),
            ],
        );

        let remainder = (amount1 - transfer).unwrap();
        assert_eq!(get_balance(&deps, &addr1), remainder);
        assert_eq!(get_balance(&deps, &addr2), transfer);
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);
    }

    #[test]
    fn transfer_on_liquidation() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let addr1 = HumanAddr::from("addr0001");
        let addr2 = HumanAddr::from("addr0002");
        let red_bank = HumanAddr::from("red_bank");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_init(&mut deps, &addr1, amount1);

        // cannot transfer nothing
        {
            let env = mock_env(red_bank.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: Uint128::zero(),
            };
            let res = handle(&mut deps, env, msg);
            assert_generic_error_message(res, "Invalid zero amount");
        }

        // cannot send more than we have
        {
            let env = mock_env(red_bank.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: too_much,
            };
            let res = handle(&mut deps, env, msg);
            match res.unwrap_err() {
                StdError::Underflow { .. } => {}
                e => panic!("Unexpected error: {}", e),
            }
        }

        // cannot send from empty account
        {
            let env = mock_env(red_bank.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                sender: addr2.clone(),
                recipient: addr1.clone(),
                amount: transfer,
            };
            let res = handle(&mut deps, env, msg);
            match res.unwrap_err() {
                StdError::Underflow { .. } => {}
                e => panic!("Unexpected error: {}", e),
            }
        }

        // only money market can call transfer on liquidation
        {
            let env = mock_env(addr1.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: transfer,
            };
            let res_error = handle(&mut deps, env, msg).unwrap_err();
            assert_eq!(res_error, StdError::unauthorized());
        }

        // valid transfer on liquidation
        {
            let env = mock_env(red_bank, &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: transfer,
            };
            let res = handle(&mut deps, env, msg).unwrap();
            assert_eq!(
                res.messages,
                vec![
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("incentives"),
                        msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                            user_address: addr1.clone(),
                            user_balance_before: amount1,
                            total_supply_before: amount1,
                        },)
                        .unwrap(),
                        send: vec![],
                    }),
                    CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: HumanAddr::from("incentives"),
                        msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                            user_address: addr2.clone(),
                            user_balance_before: Uint128::zero(),
                            total_supply_before: amount1,
                        },)
                        .unwrap(),
                        send: vec![],
                    }),
                ]
            );

            let remainder = (amount1 - transfer).unwrap();
            assert_eq!(get_balance(&deps, &addr1), remainder);
            assert_eq!(get_balance(&deps, &addr2), transfer);
            assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);
        }
    }

    #[test]
    fn burn() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let red_bank = HumanAddr::from("red_bank");
        let addr1 = HumanAddr::from("addr0001");
        let amount1 = Uint128::from(12340000u128);
        let burn = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_init(&mut deps, &addr1, amount1);

        // cannot burn nothing
        let env = mock_env(red_bank.clone(), &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: Uint128::zero(),
        };
        let res = handle(&mut deps, env, msg);
        assert_generic_error_message(res, "Invalid zero amount");
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // cannot burn more than we have
        let env = mock_env(red_bank.clone(), &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: too_much,
        };
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Underflow { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // only money market can burn
        let env = mock_env(HumanAddr::from("someone_else"), &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: burn,
        };
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::unauthorized());
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // valid burn reduces total supply
        let env = mock_env(red_bank, &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: burn,
        };
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("incentives"),
                msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                    user_address: addr1.clone(),
                    user_balance_before: amount1,
                    total_supply_before: amount1,
                },)
                .unwrap(),
                send: vec![],
            }),]
        );

        let remainder = (amount1 - burn).unwrap();
        assert_eq!(get_balance(&deps, &addr1), remainder);
        assert_eq!(query_token_info(&deps).unwrap().total_supply, remainder);
    }

    #[test]
    fn send() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let addr1 = HumanAddr::from("addr0001");
        let contract = HumanAddr::from("addr0002");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);
        let send_msg = Binary::from(br#"{"some":123}"#);

        do_init(&mut deps, &addr1, amount1);

        // cannot send nothing
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Send {
            contract: contract.clone(),
            amount: Uint128::zero(),
            msg: Some(send_msg.clone()),
        };
        let res = handle(&mut deps, env, msg);
        assert_generic_error_message(res, "Invalid zero amount");

        // cannot send more than we have
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Send {
            contract: contract.clone(),
            amount: too_much,
            msg: Some(send_msg.clone()),
        };
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Underflow { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }

        // valid transfer
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Send {
            contract: contract.clone(),
            amount: transfer,
            msg: Some(send_msg.clone()),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // ensure proper send message sent
        // this is the message we want delivered to the other side
        let binary_msg = Cw20ReceiveMsg {
            sender: addr1.clone(),
            amount: transfer,
            msg: Some(send_msg),
        }
        .into_binary()
        .unwrap();

        // Ensure finalize liquidity token transfer msg is sent
        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("red_bank"),
                    msg: to_binary(
                        &mars::liquidity_pool::msg::HandleMsg::FinalizeLiquidityTokenTransfer {
                            sender_address: addr1.clone(),
                            recipient_address: contract.clone(),
                            sender_previous_balance: amount1,
                            recipient_previous_balance: Uint128::zero(),
                            amount: transfer,
                        }
                    )
                    .unwrap(),
                    send: vec![],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                        user_address: addr1.clone(),
                        user_balance_before: amount1,
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    send: vec![],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::HandleMsg::BalanceChange {
                        user_address: contract.clone(),
                        user_balance_before: Uint128::zero(),
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    send: vec![],
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract.clone(),
                    msg: binary_msg,
                    send: vec![],
                }),
            ]
        );

        // ensure balance is properly transfered
        let remainder = (amount1 - transfer).unwrap();
        assert_eq!(get_balance(&deps, &addr1), remainder);
        assert_eq!(get_balance(&deps, &contract), transfer);
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);
    }
}
