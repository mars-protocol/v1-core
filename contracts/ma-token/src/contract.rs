use cosmwasm_std::{
    attr, entry_point, to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response,
    StdError, StdResult, SubMsg, Uint128,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::Cw20ReceiveMsg;
use cw20_base::allowances::{
    execute_decrease_allowance, execute_increase_allowance, query_allowance,
};
use cw20_base::contract::{create_accounts, query_balance, query_minter, query_token_info};
use cw20_base::enumerable::{query_all_accounts, query_all_allowances};
use cw20_base::state::{BALANCES, TOKEN_INFO};
use cw20_base::ContractError as Cw20BaseError;

use mars::cw20_core::instantiate_token_info;
use mars::ma_token::msg::{
    BalanceAndTotalSupplyResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg,
};

use crate::allowances::{execute_send_from, execute_transfer_from};
use crate::core;
use crate::error::ContractError;
use crate::state::{Config, CONFIG};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:ma-token";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    mut deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let base_msg = cw20_base::msg::InstantiateMsg {
        name: msg.name,
        symbol: msg.symbol,
        decimals: msg.decimals,
        initial_balances: msg.initial_balances,
        mint: msg.mint,
    };
    base_msg.validate()?;

    let total_supply = create_accounts(&mut deps, &base_msg.initial_balances)?;
    instantiate_token_info(&mut deps, base_msg, total_supply)?;

    // store token config
    CONFIG.save(
        deps.storage,
        &Config {
            red_bank_address: deps.api.addr_validate(&msg.red_bank_address)?,
            incentives_address: deps.api.addr_validate(&msg.incentives_address)?,
        },
    )?;

    Ok(Response::default())
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Transfer { recipient, amount } => {
            execute_transfer(deps, env, info, recipient, amount)
        }
        ExecuteMsg::TransferOnLiquidation {
            sender,
            recipient,
            amount,
        } => execute_transfer_on_liquidation(deps, env, info, sender, recipient, amount),
        ExecuteMsg::Burn { user, amount } => execute_burn(deps, env, info, user, amount),
        ExecuteMsg::Send {
            contract,
            amount,
            msg,
        } => execute_send(deps, env, info, contract, amount, msg),
        ExecuteMsg::Mint { recipient, amount } => execute_mint(deps, env, info, recipient, amount),
        ExecuteMsg::IncreaseAllowance {
            spender,
            amount,
            expires,
        } => Ok(execute_increase_allowance(
            deps, env, info, spender, amount, expires,
        )?),
        ExecuteMsg::DecreaseAllowance {
            spender,
            amount,
            expires,
        } => Ok(execute_decrease_allowance(
            deps, env, info, spender, amount, expires,
        )?),
        ExecuteMsg::TransferFrom {
            owner,
            recipient,
            amount,
        } => execute_transfer_from(deps, env, info, owner, recipient, amount),
        ExecuteMsg::SendFrom {
            owner,
            contract,
            amount,
            msg,
        } => execute_send_from(deps, env, info, owner, contract, amount, msg),
    }
}

pub fn execute_transfer(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    recipient_unchecked: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    if amount == Uint128::zero() {
        return Err(Cw20BaseError::InvalidZeroAmount {}.into());
    }

    let config = CONFIG.load(deps.storage)?;

    let recipient = deps.api.addr_validate(&recipient_unchecked)?;
    let messages = core::transfer(
        deps.storage,
        &config,
        info.sender.clone(),
        recipient,
        amount,
        true,
    )?;

    let res = Response {
        messages,
        attributes: vec![
            attr("action", "transfer"),
            attr("from", info.sender.to_string()),
            attr("to", recipient_unchecked),
            attr("amount", amount),
        ],
        events: vec![],
        data: None,
    };
    Ok(res)
}

pub fn execute_transfer_on_liquidation(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    sender_unchecked: String,
    recipient_unchecked: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    // only red bank can call
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.red_bank_address {
        return Err(Cw20BaseError::Unauthorized {}.into());
    }

    let sender = deps.api.addr_validate(&sender_unchecked)?;
    let recipient = deps.api.addr_validate(&recipient_unchecked)?;

    let messages = core::transfer(deps.storage, &config, sender, recipient, amount, false)?;

    let res = Response {
        messages,
        attributes: vec![
            attr("action", "transfer_on_liquidation"),
            attr("from", sender_unchecked),
            attr("to", recipient_unchecked),
            attr("amount", amount),
        ],
        events: vec![],
        data: None,
    };
    Ok(res)
}

pub fn execute_burn(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    user_unchecked: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    // only money market can burn
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.red_bank_address {
        return Err(Cw20BaseError::Unauthorized {}.into());
    }

    if amount == Uint128::zero() {
        return Err(Cw20BaseError::InvalidZeroAmount {}.into());
    }

    // lower balance
    let user_address = deps.api.addr_validate(&user_unchecked)?;
    let user_balance_before = core::decrease_balance(deps.storage, &user_address, amount)?;

    // reduce total_supply
    let mut total_supply_before = Uint128::zero();
    TOKEN_INFO.update(deps.storage, |mut info| -> StdResult<_> {
        total_supply_before = info.total_supply;
        info.total_supply = info.total_supply.checked_sub(amount)?;
        Ok(info)
    })?;

    let res = Response {
        messages: vec![core::balance_change_msg(
            config.incentives_address,
            user_address,
            user_balance_before,
            total_supply_before,
        )?],
        attributes: vec![
            attr("action", "burn"),
            attr("user", user_unchecked),
            attr("amount", amount),
        ],
        events: vec![],
        data: None,
    };
    Ok(res)
}

pub fn execute_mint(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    recipient_unchecked: String,
    amount: Uint128,
) -> Result<Response, ContractError> {
    if amount == Uint128::zero() {
        return Err(Cw20BaseError::InvalidZeroAmount {}.into());
    }

    let mut token_info = TOKEN_INFO.load(deps.storage)?;
    if token_info.mint.is_none() || token_info.mint.as_ref().unwrap().minter != info.sender {
        return Err(Cw20BaseError::Unauthorized {}.into());
    }

    let total_supply_before = token_info.total_supply;

    // update supply and enforce cap
    token_info.total_supply += amount;
    if let Some(limit) = token_info.get_cap() {
        if token_info.total_supply > limit {
            return Err(Cw20BaseError::CannotExceedCap {}.into());
        }
    }
    TOKEN_INFO.save(deps.storage, &token_info)?;

    // add amount to recipient balance
    let rcpt_address = deps.api.addr_validate(&recipient_unchecked)?;
    let rcpt_balance_before = core::increase_balance(deps.storage, &rcpt_address, amount)?;

    let config = CONFIG.load(deps.storage)?;

    let res = Response {
        messages: vec![core::balance_change_msg(
            config.incentives_address,
            rcpt_address,
            rcpt_balance_before,
            total_supply_before,
        )?],
        attributes: vec![
            attr("action", "mint"),
            attr("to", recipient_unchecked),
            attr("amount", amount),
        ],
        events: vec![],
        data: None,
    };
    Ok(res)
}

pub fn execute_send(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    contract_unchecked: String,
    amount: Uint128,
    msg: Binary,
) -> Result<Response, ContractError> {
    if amount == Uint128::zero() {
        return Err(Cw20BaseError::InvalidZeroAmount {}.into());
    }

    // move the tokens to the contract
    let config = CONFIG.load(deps.storage)?;
    let contract_address = deps.api.addr_validate(&contract_unchecked)?;
    let mut messages = core::transfer(
        deps.storage,
        &config,
        info.sender.clone(),
        contract_address,
        amount,
        true,
    )?;

    let attributes = vec![
        attr("action", "send"),
        attr("from", info.sender.to_string()),
        attr("to", &contract_unchecked),
        attr("amount", amount),
    ];

    // create a send message
    messages.push(SubMsg::new(
        Cw20ReceiveMsg {
            sender: info.sender.to_string(),
            amount,
            msg,
        }
        .into_cosmos_msg(contract_unchecked)?,
    ));

    let res = Response {
        messages,
        attributes,
        events: vec![],
        data: None,
    };

    Ok(res)
}

// QUERY

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
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

fn query_balance_and_total_supply(
    deps: Deps,
    address_unchecked: String,
) -> StdResult<BalanceAndTotalSupplyResponse> {
    let address = deps.api.addr_validate(&address_unchecked)?;
    let balance = BALANCES
        .may_load(deps.storage, &address)?
        .unwrap_or_default();
    let info = TOKEN_INFO.load(deps.storage)?;
    Ok(BalanceAndTotalSupplyResponse {
        balance,
        total_supply: info.total_supply,
    })
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    let old_version = get_contract_version(deps.storage)?;
    if old_version.contract != CONTRACT_NAME {
        return Err(StdError::generic_err(format!(
            "This is {}, cannot migrate from {}",
            CONTRACT_NAME, old_version.contract
        )));
    }
    // NOTE: v0.1.0 were not auto-generated and started with v0.
    // more recent versions do not have the v prefix

    // TODO: This is copied from the old cw20_base, see if it makes sense for us
    if old_version.version.starts_with("v0.1.") {
        set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        return Ok(Response::default());
    }

    Err(StdError::generic_err(format!(
        "Unknown version {}",
        old_version.version
    )))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coins, Addr, CosmosMsg, StdError, WasmMsg};
    use cw20::{Cw20Coin, MinterResponse, TokenInfoResponse};

    use super::*;

    fn get_balance<T: Into<String>>(deps: Deps, address: T) -> Uint128 {
        query_balance(deps, address.into()).unwrap().balance
    }

    // this will set up the instantiation for other tests
    fn do_instantiate_with_minter(
        deps: DepsMut,
        addr: &str,
        amount: Uint128,
        minter: &str,
        cap: Option<Uint128>,
    ) -> TokenInfoResponse {
        _do_instantiate(
            deps,
            addr,
            amount,
            Some(MinterResponse {
                minter: minter.to_string(),
                cap,
            }),
        )
    }

    // this will set up the instantiation for other tests
    fn do_instantiate(deps: DepsMut, addr: &str, amount: Uint128) -> TokenInfoResponse {
        _do_instantiate(deps, addr, amount, None)
    }

    // this will set up the instantiation for other tests
    fn _do_instantiate(
        mut deps: DepsMut,
        addr: &str,
        amount: Uint128,
        mint: Option<MinterResponse>,
    ) -> TokenInfoResponse {
        let instantiate_msg = InstantiateMsg {
            name: "Auto Gen".to_string(),
            symbol: "AUTO".to_string(),
            decimals: 3,
            initial_balances: vec![Cw20Coin {
                address: addr.to_string(),
                amount,
            }],
            mint: mint.clone(),
            red_bank_address: String::from("red_bank"),
            incentives_address: String::from("incentives"),
        };
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let res = instantiate(deps.branch(), env, info, instantiate_msg).unwrap();
        assert_eq!(0, res.messages.len());

        let meta = query_token_info(deps.as_ref()).unwrap();
        assert_eq!(
            meta,
            TokenInfoResponse {
                name: "Auto Gen".to_string(),
                symbol: "AUTO".to_string(),
                decimals: 3,
                total_supply: amount,
            }
        );
        assert_eq!(get_balance(deps.as_ref(), addr), amount);
        assert_eq!(query_minter(deps.as_ref()).unwrap(), mint,);
        meta
    }

    #[test]
    fn proper_instantiation() {
        let mut deps = mock_dependencies(&[]);
        let amount = Uint128::from(11223344u128);
        let instantiate_msg = InstantiateMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20Coin {
                address: String::from("addr0000"),
                amount,
            }],
            mint: None,
            red_bank_address: String::from("red_bank"),
            incentives_address: String::from("incentives"),
        };
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let res = instantiate(deps.as_mut(), env, info, instantiate_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(deps.as_ref()).unwrap(),
            TokenInfoResponse {
                name: "Cash Token".to_string(),
                symbol: "CASH".to_string(),
                decimals: 9,
                total_supply: amount,
            }
        );
        assert_eq!(
            get_balance(deps.as_ref(), "addr0000"),
            Uint128::new(11223344)
        );

        let config = CONFIG.load(&deps.storage).unwrap();
        assert_eq!(config.red_bank_address, Addr::unchecked("red_bank"));
        assert_eq!(config.incentives_address, Addr::unchecked("incentives"));
    }

    #[test]
    fn instantiate_mintable() {
        let mut deps = mock_dependencies(&[]);
        let amount = Uint128::new(11223344);
        let minter = String::from("asmodat");
        let limit = Uint128::new(511223344);
        let instantiate_msg = InstantiateMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20Coin {
                address: "addr0000".into(),
                amount,
            }],
            mint: Some(MinterResponse {
                minter: minter.clone(),
                cap: Some(limit),
            }),
            red_bank_address: String::from("red_bank"),
            incentives_address: String::from("incentives"),
        };
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let res = instantiate(deps.as_mut(), env, info, instantiate_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(deps.as_ref()).unwrap(),
            TokenInfoResponse {
                name: "Cash Token".to_string(),
                symbol: "CASH".to_string(),
                decimals: 9,
                total_supply: amount,
            }
        );
        assert_eq!(
            get_balance(deps.as_ref(), "addr0000"),
            Uint128::new(11223344)
        );
        assert_eq!(
            query_minter(deps.as_ref()).unwrap(),
            Some(MinterResponse {
                minter,
                cap: Some(limit),
            }),
        );
    }

    #[test]
    fn instantiate_mintable_over_cap() {
        let mut deps = mock_dependencies(&[]);
        let amount = Uint128::new(11223344);
        let minter = String::from("asmodat");
        let limit = Uint128::new(11223300);
        let instantiate_msg = InstantiateMsg {
            name: "Cash Token".to_string(),
            symbol: "CASH".to_string(),
            decimals: 9,
            initial_balances: vec![Cw20Coin {
                address: String::from("addr0000"),
                amount,
            }],
            mint: Some(MinterResponse {
                minter,
                cap: Some(limit),
            }),
            red_bank_address: String::from("red_bank"),
            incentives_address: String::from("incentives"),
        };
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let err = instantiate(deps.as_mut(), env, info, instantiate_msg).unwrap_err();
        assert_eq!(
            err,
            StdError::generic_err("Initial supply greater than cap")
        );
    }

    #[test]
    fn can_mint_by_minter() {
        let mut deps = mock_dependencies(&[]);

        let genesis = String::from("genesis");
        let amount = Uint128::new(11223344);
        let minter = String::from("asmodat");
        let limit = Uint128::new(511223344);
        do_instantiate_with_minter(deps.as_mut(), &genesis, amount, &minter, Some(limit));

        // minter can mint coins to some winner
        let winner = String::from("lucky");
        let prize = Uint128::new(222_222_222);
        let msg = ExecuteMsg::Mint {
            recipient: winner.clone(),
            amount: prize,
        };

        let info = mock_info(minter.as_ref(), &[]);
        let env = mock_env();
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("incentives"),
                msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                    user_address: winner.to_string(),
                    user_balance_before: Uint128::zero(),
                    total_supply_before: amount,
                },)
                .unwrap(),
                funds: vec![],
            })),]
        );
        assert_eq!(get_balance(deps.as_ref(), genesis), amount);
        assert_eq!(get_balance(deps.as_ref(), winner.clone()), prize);
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount + prize
        );

        // but cannot mint nothing
        let msg = ExecuteMsg::Mint {
            recipient: winner.clone(),
            amount: Uint128::zero(),
        };
        let info = mock_info(minter.as_ref(), &[]);
        let env = mock_env();
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::InvalidZeroAmount {}.into());

        // but if it exceeds cap (even over multiple rounds), it fails
        // cap is enforced
        let msg = ExecuteMsg::Mint {
            recipient: winner,
            amount: Uint128::new(333_222_222),
        };
        let info = mock_info(minter.as_ref(), &[]);
        let env = mock_env();
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::CannotExceedCap {}.into());
    }

    #[test]
    fn others_cannot_mint() {
        let mut deps = mock_dependencies(&[]);
        do_instantiate_with_minter(
            deps.as_mut(),
            &String::from("genesis"),
            Uint128::new(1234),
            &String::from("minter"),
            None,
        );

        let msg = ExecuteMsg::Mint {
            recipient: String::from("lucky"),
            amount: Uint128::new(222),
        };
        let info = mock_info("anyone else", &[]);
        let env = mock_env();
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::Unauthorized {}.into());
    }

    #[test]
    fn no_one_mints_if_minter_unset() {
        let mut deps = mock_dependencies(&[]);
        do_instantiate(deps.as_mut(), &String::from("genesis"), Uint128::new(1234));

        let msg = ExecuteMsg::Mint {
            recipient: String::from("lucky"),
            amount: Uint128::new(222),
        };
        let info = mock_info("genesis", &[]);
        let env = mock_env();
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::Unauthorized {}.into());
    }

    #[test]
    fn instantiate_multiple_accounts() {
        let mut deps = mock_dependencies(&[]);
        let amount1 = Uint128::from(11223344u128);
        let addr1 = String::from("addr0001");
        let amount2 = Uint128::from(7890987u128);
        let addr2 = String::from("addr0002");
        let instantiate_msg = InstantiateMsg {
            name: "Bash Shell".to_string(),
            symbol: "BASH".to_string(),
            decimals: 6,
            initial_balances: vec![
                Cw20Coin {
                    address: addr1.clone(),
                    amount: amount1,
                },
                Cw20Coin {
                    address: addr2.clone(),
                    amount: amount2,
                },
            ],
            mint: None,
            red_bank_address: String::from("red_bank"),
            incentives_address: String::from("incentives"),
        };
        let info = mock_info("creator", &[]);
        let env = mock_env();
        let res = instantiate(deps.as_mut(), env, info, instantiate_msg).unwrap();
        assert_eq!(0, res.messages.len());

        assert_eq!(
            query_token_info(deps.as_ref()).unwrap(),
            TokenInfoResponse {
                name: "Bash Shell".to_string(),
                symbol: "BASH".to_string(),
                decimals: 6,
                total_supply: amount1 + amount2,
            }
        );
        assert_eq!(get_balance(deps.as_ref(), addr1), amount1);
        assert_eq!(get_balance(deps.as_ref(), addr2), amount2);
    }

    #[test]
    fn transfer() {
        let mut deps = mock_dependencies(&coins(2, "token"));
        let addr1 = String::from("addr0001");
        let addr2 = String::from("addr0002");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_instantiate(deps.as_mut(), &addr1, amount1);

        // cannot transfer nothing
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Transfer {
            recipient: addr2.clone(),
            amount: Uint128::zero(),
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::InvalidZeroAmount {}.into());

        // cannot send more than we have
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Transfer {
            recipient: addr2.clone(),
            amount: too_much,
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));

        // cannot send from empty account
        let info = mock_info(addr2.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Transfer {
            recipient: addr1.clone(),
            amount: transfer,
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));

        // valid transfer
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Transfer {
            recipient: addr2.clone(),
            amount: transfer,
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("red_bank"),
                    msg: to_binary(
                        &mars::red_bank::msg::ExecuteMsg::FinalizeLiquidityTokenTransfer {
                            sender_address: addr1.clone(),
                            recipient_address: addr2.clone(),
                            sender_previous_balance: amount1,
                            recipient_previous_balance: Uint128::zero(),
                            amount: transfer,
                        }
                    )
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                        user_address: addr1.clone(),
                        user_balance_before: amount1,
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                        user_address: addr2.clone(),
                        user_balance_before: Uint128::zero(),
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    funds: vec![],
                })),
            ],
        );

        let remainder = amount1.checked_sub(transfer).unwrap();
        assert_eq!(get_balance(deps.as_ref(), addr1), remainder);
        assert_eq!(get_balance(deps.as_ref(), addr2), transfer);
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount1
        );
    }

    #[test]
    fn transfer_on_liquidation() {
        let mut deps = mock_dependencies(&coins(2, "token"));
        let addr1 = String::from("addr0001");
        let addr2 = String::from("addr0002");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_instantiate(deps.as_mut(), &addr1, amount1);

        // cannot transfer nothing
        {
            let info = mock_info("red_bank", &[]);
            let env = mock_env();
            let msg = ExecuteMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: Uint128::zero(),
            };
            let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
            assert_eq!(err, Cw20BaseError::InvalidZeroAmount {}.into());
        }

        // cannot send more than we have
        {
            let info = mock_info("red_bank", &[]);
            let env = mock_env();
            let msg = ExecuteMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: too_much,
            };
            let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
            assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));
        }

        // cannot send from empty account
        {
            let info = mock_info("red_bank", &[]);
            let env = mock_env();
            let msg = ExecuteMsg::TransferOnLiquidation {
                sender: addr2.clone(),
                recipient: addr1.clone(),
                amount: transfer,
            };
            let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
            assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));
        }

        // only money market can call transfer on liquidation
        {
            let info = mock_info(addr1.as_ref(), &[]);
            let env = mock_env();
            let msg = ExecuteMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: transfer,
            };
            let res_error = execute(deps.as_mut(), env, info, msg).unwrap_err();
            assert_eq!(res_error, Cw20BaseError::Unauthorized {}.into());
        }

        // valid transfer on liquidation
        {
            let info = mock_info("red_bank", &[]);
            let env = mock_env();
            let msg = ExecuteMsg::TransferOnLiquidation {
                sender: addr1.clone(),
                recipient: addr2.clone(),
                amount: transfer,
            };
            let res = execute(deps.as_mut(), env, info, msg).unwrap();
            assert_eq!(
                res.messages,
                vec![
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: String::from("incentives"),
                        msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                            user_address: addr1.clone(),
                            user_balance_before: amount1,
                            total_supply_before: amount1,
                        },)
                        .unwrap(),
                        funds: vec![],
                    })),
                    SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                        contract_addr: String::from("incentives"),
                        msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                            user_address: addr2.clone(),
                            user_balance_before: Uint128::zero(),
                            total_supply_before: amount1,
                        },)
                        .unwrap(),
                        funds: vec![],
                    })),
                ]
            );

            let remainder = amount1.checked_sub(transfer).unwrap();
            assert_eq!(get_balance(deps.as_ref(), addr1), remainder);
            assert_eq!(get_balance(deps.as_ref(), addr2), transfer);
            assert_eq!(
                query_token_info(deps.as_ref()).unwrap().total_supply,
                amount1
            );
        }
    }

    #[test]
    fn burn() {
        let mut deps = mock_dependencies(&coins(2, "token"));
        let addr1 = String::from("addr0001");
        let amount1 = Uint128::from(12340000u128);
        let burn = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);

        do_instantiate(deps.as_mut(), &addr1, amount1);

        // cannot burn nothing
        let info = mock_info("red_bank", &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Burn {
            user: addr1.clone(),
            amount: Uint128::zero(),
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::InvalidZeroAmount {}.into());
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount1
        );

        // cannot burn more than we have
        let info = mock_info("red_bank", &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Burn {
            user: addr1.clone(),
            amount: too_much,
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount1
        );

        // only red bank can burn
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Burn {
            user: addr1.clone(),
            amount: burn,
        };
        let res_error = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(res_error, Cw20BaseError::Unauthorized {}.into());
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount1
        );

        // valid burn reduces total supply
        let info = mock_info("red_bank", &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Burn {
            user: addr1.clone(),
            amount: burn,
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("incentives"),
                msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                    user_address: addr1.clone(),
                    user_balance_before: amount1,
                    total_supply_before: amount1,
                },)
                .unwrap(),
                funds: vec![],
            })),]
        );

        let remainder = amount1.checked_sub(burn).unwrap();
        assert_eq!(get_balance(deps.as_ref(), addr1), remainder);
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            remainder
        );
    }

    #[test]
    fn send() {
        let mut deps = mock_dependencies(&coins(2, "token"));
        let addr1 = String::from("addr0001");
        let contract = String::from("addr0002");
        let amount1 = Uint128::from(12340000u128);
        let transfer = Uint128::from(76543u128);
        let too_much = Uint128::from(12340321u128);
        let send_msg = Binary::from(r#"{"some":123}"#.as_bytes());

        do_instantiate(deps.as_mut(), &addr1, amount1);

        // cannot send nothing
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Send {
            contract: contract.clone(),
            amount: Uint128::zero(),
            msg: send_msg.clone(),
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert_eq!(err, Cw20BaseError::InvalidZeroAmount {}.into());

        // cannot send more than we have
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Send {
            contract: contract.clone(),
            amount: too_much,
            msg: send_msg.clone(),
        };
        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        assert!(matches!(err, ContractError::Std(StdError::Overflow { .. })));

        // valid transfer
        let info = mock_info(addr1.as_ref(), &[]);
        let env = mock_env();
        let msg = ExecuteMsg::Send {
            contract: contract.clone(),
            amount: transfer,
            msg: send_msg.clone(),
        };
        let res = execute(deps.as_mut(), env, info, msg).unwrap();

        // ensure proper send message sent
        // this is the message we want delivered to the other side
        let binary_msg = Cw20ReceiveMsg {
            sender: addr1.clone(),
            amount: transfer,
            msg: send_msg,
        }
        .into_binary()
        .unwrap();

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("red_bank"),
                    msg: to_binary(
                        &mars::red_bank::msg::ExecuteMsg::FinalizeLiquidityTokenTransfer {
                            sender_address: addr1.clone(),
                            recipient_address: contract.clone(),
                            sender_previous_balance: amount1,
                            recipient_previous_balance: Uint128::zero(),
                            amount: transfer,
                        }
                    )
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                        user_address: addr1.clone(),
                        user_balance_before: amount1,
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("incentives"),
                    msg: to_binary(&mars::incentives::msg::ExecuteMsg::BalanceChange {
                        user_address: contract.clone(),
                        user_balance_before: Uint128::zero(),
                        total_supply_before: amount1,
                    },)
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract.clone(),
                    msg: binary_msg,
                    funds: vec![],
                })),
            ]
        );

        // ensure balance is properly transferred
        let remainder = amount1.checked_sub(transfer).unwrap();
        assert_eq!(get_balance(deps.as_ref(), addr1), remainder);
        assert_eq!(get_balance(deps.as_ref(), contract), transfer);
        assert_eq!(
            query_token_info(deps.as_ref()).unwrap().total_supply,
            amount1
        );
    }
}
