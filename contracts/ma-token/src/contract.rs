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
use mars::ma_token::msg::{HandleMsg, InitMsg, MigrateMsg, QueryMsg};

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
            money_market_address: deps.api.canonical_address(&msg.money_market_address)?,
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
        HandleMsg::TransferOnLiquidation { from, to, amount } => {
            handle_transfer_on_liquidation(deps, env, from, to, amount)
        }
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
    let sender_raw = deps.api.canonical_address(&env.message.sender)?;
    let rcpt_raw = deps.api.canonical_address(&recipient)?;

    let (from_previous_balance, to_previous_balance) =
        core::transfer(deps, &sender_raw, &rcpt_raw, amount)?;

    let res = HandleResponse {
        messages: vec![core::finalize_transfer_msg(
            &deps.api,
            &state::load_config(&deps.storage)?.money_market_address,
            env.message.sender,
            recipient.clone(),
            from_previous_balance,
            to_previous_balance,
            amount,
        )?],
        log: vec![
            log("action", "transfer"),
            log("from", deps.api.human_address(&sender_raw)?),
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
    from: HumanAddr,
    to: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    // only money market can call
    let config = state::load_config(&deps.storage)?;
    if deps.api.canonical_address(&env.message.sender)? != config.money_market_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }
    let from_raw = deps.api.canonical_address(&from)?;
    let to_raw = deps.api.canonical_address(&to)?;

    core::transfer(deps, &from_raw, &to_raw, amount)?;

    let res = HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "transfer_on_liquidation"),
            log("from", from),
            log("to", to),
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
    if deps.api.canonical_address(&env.message.sender)? != config.money_market_address {
        return Err(StdError::unauthorized());
    }

    if amount == Uint128::zero() {
        return Err(StdError::generic_err("Invalid zero amount"));
    }

    let user_raw = deps.api.canonical_address(&user)?;

    // lower balance
    let mut accounts = balances(&mut deps.storage);
    let user_balance_old = accounts.load(user_raw.as_slice()).unwrap_or_default();
    let user_balance_new = (user_balance_old - amount)?;
    accounts.save(user_raw.as_slice(), &user_balance_new)?;

    // reduce total_supply
    token_info(&mut deps.storage).update(|mut info| {
        info.total_supply = (info.total_supply - amount)?;
        Ok(info)
    })?;

    let res = HandleResponse {
        messages: vec![],
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

    let mut config = token_info_read(&deps.storage).load()?;
    if config.mint.is_none()
        || config.mint.as_ref().unwrap().minter
            != deps.api.canonical_address(&env.message.sender)?
    {
        return Err(StdError::unauthorized());
    }

    // update supply and enforce cap
    config.total_supply += amount;
    if let Some(limit) = config.get_cap() {
        if config.total_supply > limit {
            return Err(StdError::generic_err("Minting cannot exceed the cap"));
        }
    }
    token_info(&mut deps.storage).save(&config)?;

    // add amount to recipient balance
    let mut accounts = balances(&mut deps.storage);
    let rcpt_raw = deps.api.canonical_address(&recipient)?;
    let rcpt_balance_old = accounts.load(rcpt_raw.as_slice()).unwrap_or_default();
    let rcpt_balance_new = rcpt_balance_old + amount;
    accounts.save(rcpt_raw.as_slice(), &rcpt_balance_new)?;

    let res = HandleResponse {
        messages: vec![],
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

    let rcpt_raw = deps.api.canonical_address(&contract)?;
    let sender_raw = deps.api.canonical_address(&env.message.sender)?;

    // move the tokens to the contract
    let (from_previous_balance, to_previous_balance) =
        core::transfer(deps, &sender_raw, &rcpt_raw, amount)?;

    let sender = deps.api.human_address(&sender_raw)?;
    let logs = vec![
        log("action", "send"),
        log("from", &sender),
        log("to", &contract),
        log("amount", amount),
    ];

    // If the send call is to the money market (to redeem), then
    // don't ask money market to finalize the transfer. The corresponding logic
    // should be run synchronously with the redeem call.
    let money_market_address = state::load_config(&deps.storage)?.money_market_address;

    let mut messages = if money_market_address == deps.api.canonical_address(&contract)? {
        vec![]
    } else {
        vec![core::finalize_transfer_msg(
            &deps.api,
            &money_market_address,
            env.message.sender,
            contract.clone(),
            from_previous_balance,
            to_previous_balance,
            amount,
        )?]
    };

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
            money_market_address: HumanAddr::from("money_market"),
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
        let money_market_address = HumanAddr::from("money_market");

        let init_msg = InitMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20CoinHuman {
                address: HumanAddr("addr0000".to_string()),
                amount,
            }],
            mint: None,
            money_market_address: money_market_address.clone(),
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
            config.money_market_address,
            deps.api.canonical_address(&money_market_address).unwrap()
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
            money_market_address: HumanAddr::from("money_market"),
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
            money_market_address: HumanAddr::from("money_market"),
        };
        let env = mock_env(&HumanAddr("creator".to_string()), &[]);
        let res = init(&mut deps, env, init_msg);
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!(&msg, "Initial supply greater than cap"),
            e => panic!("Unexpected error: {}", e),
        }
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
        assert_eq!(0, res.messages.len());
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
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!("Invalid zero amount", msg),
            e => panic!("Unexpected error: {}", e),
        }

        // but if it exceeds cap (even over multiple rounds), it fails
        // cap is enforced
        let msg = HandleMsg::Mint {
            recipient: winner,
            amount: Uint128(333_222_222),
        };
        let env = mock_env(&minter, &[]);
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => {
                assert_eq!(msg, "Minting cannot exceed the cap".to_string())
            }
            e => panic!("Unexpected error: {}", e),
        }
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
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Unauthorized { .. } => {}
            e => panic!("expected unauthorized error, got {}", e),
        }
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
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Unauthorized { .. } => {}
            e => panic!("expected unauthorized error, got {}", e),
        }
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
            money_market_address: HumanAddr::from("money_market"),
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
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!("Invalid zero amount", msg),
            e => panic!("Unexpected error: {}", e),
        }

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
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("money_market"),
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
            })],
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
        let money_market = HumanAddr::from("money_market");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_init(&mut deps, &addr1, amount1);

        // cannot transfer nothing
        {
            let env = mock_env(money_market.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                from: addr1.clone(),
                to: addr2.clone(),
                amount: Uint128::zero(),
            };
            let res = handle(&mut deps, env, msg);
            match res.unwrap_err() {
                StdError::GenericErr { msg, .. } => assert_eq!("Invalid zero amount", msg),
                e => panic!("Unexpected error: {}", e),
            }
        }

        // cannot send more than we have
        {
            let env = mock_env(money_market.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                from: addr1.clone(),
                to: addr2.clone(),
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
            let env = mock_env(money_market.clone(), &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                from: addr2.clone(),
                to: addr1.clone(),
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
                from: addr1.clone(),
                to: addr2.clone(),
                amount: transfer,
            };
            let res = handle(&mut deps, env, msg);
            match res.unwrap_err() {
                StdError::Unauthorized { .. } => {}
                e => panic!("Unexpected error: {}", e),
            }
        }

        // valid transfer on liquidation
        {
            let env = mock_env(money_market, &[]);
            let msg = HandleMsg::TransferOnLiquidation {
                from: addr1.clone(),
                to: addr2.clone(),
                amount: transfer,
            };
            let res = handle(&mut deps, env, msg).unwrap();
            assert_eq!(res.messages.len(), 0);

            let remainder = (amount1 - transfer).unwrap();
            assert_eq!(get_balance(&deps, &addr1), remainder);
            assert_eq!(get_balance(&deps, &addr2), transfer);
            assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);
        }
    }

    #[test]
    fn burn() {
        let mut deps = mock_dependencies(20, &coins(2, "token"));
        let money_market = HumanAddr::from("money_market");
        let addr1 = HumanAddr::from("addr0001");
        let amount1 = Uint128::from(12340000u128);
        let burn = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_init(&mut deps, &addr1, amount1);

        // cannot burn nothing
        let env = mock_env(money_market.clone(), &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: Uint128::zero(),
        };
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!("Invalid zero amount", msg),
            e => panic!("Unexpected error: {}", e),
        }
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // cannot burn more than we have
        let env = mock_env(money_market.clone(), &[]);
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
        let res = handle(&mut deps, env, msg);
        match res.unwrap_err() {
            StdError::Unauthorized { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // valid burn reduces total supply
        let env = mock_env(money_market, &[]);
        let msg = HandleMsg::Burn {
            user: addr1.clone(),
            amount: burn,
        };
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(res.messages.len(), 0);

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
        match res.unwrap_err() {
            StdError::GenericErr { msg, .. } => assert_eq!("Invalid zero amount", msg),
            e => panic!("Unexpected error: {}", e),
        }

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
        assert_eq!(res.messages.len(), 2);

        // Ensure finalize liquidity token transfer msg is sent
        assert_eq!(
            res.messages[0],
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("money_market"),
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
            })
        );

        // ensure proper send message sent
        // this is the message we want delivered to the other side
        let binary_msg = Cw20ReceiveMsg {
            sender: addr1.clone(),
            amount: transfer,
            msg: Some(send_msg.clone()),
        }
        .into_binary()
        .unwrap();
        // and this is how it must be wrapped for the vm to process it
        assert_eq!(
            res.messages[1],
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: contract.clone(),
                msg: binary_msg,
                send: vec![],
            })
        );

        // ensure balance is properly transfered
        let remainder = (amount1 - transfer).unwrap();
        assert_eq!(get_balance(&deps, &addr1), remainder);
        assert_eq!(get_balance(&deps, &contract), transfer);
        assert_eq!(query_token_info(&deps).unwrap().total_supply, amount1);

        // valid transfer to money market
        let env = mock_env(addr1.clone(), &[]);
        let msg = HandleMsg::Send {
            contract: HumanAddr::from("money_market"),
            amount: transfer,
            msg: Some(send_msg.clone()),
        };
        let res = handle(&mut deps, env, msg).unwrap();

        // should not have finalize token transfer call
        assert_eq!(res.messages.len(), 1);
        // ensure proper send message sent
        // this is the message we want delivered to the other side
        let binary_msg = Cw20ReceiveMsg {
            sender: addr1,
            amount: transfer,
            msg: Some(send_msg),
        }
        .into_binary()
        .unwrap();
        // and this is how it must be wrapped for the vm to process it
        assert_eq!(
            res.messages[0],
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("money_market"),
                msg: binary_msg,
                send: vec![],
            })
        );
    }
}
