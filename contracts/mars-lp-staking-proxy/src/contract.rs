use cosmwasm_std::{
    entry_point, from_binary, to_binary, Addr, Binary, CosmosMsg, Decimal, Deps, DepsMut, Empty,
    Env, MessageInfo, Response, StdError, StdResult, SubMsg, Uint128, WasmMsg,
};
use cw20::{BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Cw20ReceiveMsg};

use crate::error::ContractError;
use crate::state::{Config, CONFIG};
use astroport::asset::addr_validate_to_lower;
use astroport::generator::{
    Cw20HookMsg as AstroGeneratorCw20HookMsg, ExecuteMsg as AstroGeneratorExecuteMsg,
    QueryMsg as AstroGeneratorQueryMsg,
};
use cw2::set_contract_version;
use mars_core::lp_staking_proxy::{
    CallbackMsg, ConfigResponse, Cw20HookMsg, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg,
};

// version info for migration info
const CONTRACT_NAME: &str = "mars-lp-staking-proxy";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let mut config = Config {
        redbank_addr: msg.redbank_addr,
        astro_generator_addr: msg.astro_generator_addr,
        token_addr: msg.token_addr,
        ma_token_addr: None,
        pool_addr: msg.pool_addr,
        astro_token_addr: msg.astro_token_addr,
        astro_treasury_fee: Decimal::zero(),
        proxy_token_reward_addr: msg.proxy_token_reward_addr,
        proxy_token_treasury_fee: Decimal::zero(),
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::UpdateRewards {} => update_rewards(deps),
        ExecuteMsg::UpdateFeeConfig {
            astro_treasury_fee,
            proxy_token_treasury_fee,
        } => update_fees(deps, info, astro_treasury_fee, proxy_token_treasury_fee),
        ExecuteMsg::SendAstroRewards { account, amount } => {
            send_astro_rewards(deps, info, account, amount)
        }
        ExecuteMsg::SendProxyRewards { account, amount } => {
            send_proxy_rewards(deps, info, account, amount)
        }
        ExecuteMsg::Withdraw { account, amount } => withdraw(deps, env, info, account, amount),
        ExecuteMsg::EmergencyWithdraw { account, amount } => {
            withdraw(deps, env, info, account, amount)
        }
        ExecuteMsg::Callback(msg) => handle_callback(deps, env, info, msg),
    }
}

pub fn handle_callback(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: CallbackMsg,
) -> Result<Response, ContractError> {
    // Callback functions can only be called this contract itself
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized {});
    }
    match msg {
        CallbackMsg::TransferTokensAfterWithdraw {
            account,
            prev_balance,
        } => transfer_tokens_after_withdraw(deps, env, account, prev_balance),
    }
}

/// @dev Receives tokens sent by Red Bank MM contract
/// Stakes them with the AstroGenerator contract
fn receive_cw20(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    let mut response = Response::new();
    let cfg = CONFIG.load(deps.storage)?;

    if let Ok(Cw20HookMsg::DepositWithProxy {}) = from_binary(&cw20_msg.msg) {
        if cw20_msg.sender != cfg.redbank_addr || info.sender != cfg.token_addr {
            return Err(ContractError::Unauthorized {});
        }
        response
            .messages
            .push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cfg.token_addr.to_string(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: cfg.astro_generator_addr.to_string(),
                    amount: cw20_msg.amount,
                    msg: to_binary(&AstroGeneratorCw20HookMsg::Deposit {})?,
                })?,
            })));
    } else {
        return Err(ContractError::IncorrectCw20HookMessageVariant {});
    }
    Ok(response)
}

/// @dev Admin function to update fee charged by Red Bank on the rewards
fn update_fees(
    deps: DepsMut,
    info: MessageInfo,
    astro_treasury_fee: Decimal,
    proxy_token_treasury_fee: Decimal,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.redbank_addr {
        return Err(ContractError::Unauthorized {});
    };

    cfg.astro_treasury_fee = astro_treasury_fee;
    cfg.proxy_token_treasury_fee = proxy_token_treasury_fee;

    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::default())
}

/// @dev Claims pending rewards from the ASTRO Generator contract
fn update_rewards(deps: DepsMut) -> Result<Response, ContractError> {
    let mut response = Response::new();
    let cfg = CONFIG.load(deps.storage)?;

    let mut lp_tokens_addr: Vec<Addr> = vec![];
    lp_tokens_addr.push(addr_validate_to_lower(
        deps.api,
        &cfg.token_addr.to_string(),
    )?);

    response
        .messages
        .push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: cfg.astro_generator_addr.to_string(),
            funds: vec![],
            msg: to_binary(&AstroGeneratorExecuteMsg::ClaimRewards {
                lp_tokens: lp_tokens_addr,
            })?,
        })));

    Ok(response)
}

/// @dev Transfers accrued rewards
/// @param account : User to which accrued ASTRO tokens are to be transferred
/// @param amount : Number of ASTRO to be transferred
fn send_astro_rewards(
    deps: DepsMut,
    info: MessageInfo,
    account: Addr,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let mut response = Response::new();
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.redbank_addr {
        return Err(ContractError::Unauthorized {});
    };

    response
        .messages
        .push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: cfg.astro_token_addr.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: account.to_string(),
                amount,
            })?,
            funds: vec![],
        })));
    Ok(response)
}

/// @dev Transfers accrued Proxy token rewards
/// @param account : User to which accrued proxy reward tokens are to be transferred
/// @param amount : Number of proxy reward tokens are to be transferred
fn send_proxy_rewards(
    deps: DepsMut,
    info: MessageInfo,
    account: Addr,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.redbank_addr {
        return Err(ContractError::Unauthorized {});
    };

    if !cfg.proxy_token_reward_addr.is_some() {
        return Err(ContractError::ProxyRewardNotSet {});
    }

    let mut response = Response::new();
    response
        .messages
        .push(SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: cfg.proxy_token_reward_addr.unwrap().to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: account.to_string(),
                amount,
            })?,
            funds: vec![],
        })));
    Ok(response)
}

/// @dev Withdraws Tokens from the AstroGenerator contract
/// @param account : User to which tokens are to be transferred
/// @param amount : Number of tokens to be unstaked and transferred
fn withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    account: Addr,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let mut response = Response::new();
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.redbank_addr {
        return Err(ContractError::Unauthorized {});
    };

    // current LP Tokens balance
    let prev_balance = {
        let res: BalanceResponse = deps.querier.query_wasm_smart(
            &cfg.token_addr,
            &Cw20QueryMsg::Balance {
                address: env.contract.address.to_string(),
            },
        )?;
        res.balance
    };

    // withdraw from the AstroGenerator contract
    response.messages.push(SubMsg::new(WasmMsg::Execute {
        contract_addr: cfg.astro_generator_addr.to_string(),
        funds: vec![],
        msg: to_binary(&AstroGeneratorExecuteMsg::Withdraw {
            lp_token: cfg.token_addr,
            amount: amount,
        })?,
    }));

    // Callback function
    response.messages.push(SubMsg::new(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        funds: vec![],
        msg: to_binary(&ExecuteMsg::Callback(
            CallbackMsg::TransferTokensAfterWithdraw {
                account,
                prev_balance,
            },
        ))?,
    }));

    Ok(response)
}

pub fn transfer_tokens_after_withdraw(
    deps: DepsMut,
    env: Env,
    account: Addr,
    prev_balance: Uint128,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Calculate number of LP Tokens withdrawn from the staking contract
    let amount = {
        let res: BalanceResponse = deps.querier.query_wasm_smart(
            &cfg.token_addr,
            &Cw20QueryMsg::Balance {
                address: env.contract.address.to_string(),
            },
        )?;
        res.balance - prev_balance
    };

    Ok(Response::new().add_message(WasmMsg::Execute {
        contract_addr: cfg.token_addr.to_string(),
        funds: vec![],
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: account.to_string(),
            amount,
        })?,
    }))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    let cfg = CONFIG.load(deps.storage)?;
    match msg {
        QueryMsg::Config {} => to_binary(&Config {
            redbank_addr: cfg.redbank_addr,
            astro_generator_addr: cfg.astro_generator_addr,
            token_addr: cfg.token_addr,
            ma_token_addr: cfg.ma_token_addr,
            pool_addr: cfg.pool_addr,
            astro_token_addr: cfg.astro_token_addr,
            astro_treasury_fee: cfg.astro_treasury_fee,
            proxy_token_reward_addr: cfg.proxy_token_reward_addr,
            proxy_token_treasury_fee: cfg.proxy_token_treasury_fee,
        }),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: MigrateMsg) -> StdResult<Response> {
    Ok(Response::default())
}
