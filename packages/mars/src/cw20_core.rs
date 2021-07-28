/// cw20_core: Shared functionality for cw20 tokens
use cosmwasm_std::{DepsMut, StdError, StdResult, Uint128};
use cw2::set_contract_version;
use cw20::Cw20Coin;
use cw20_base::msg::InstantiateMsg;
use cw20_base::state::{MinterData, TokenInfo, BALANCES, TOKEN_INFO};

/// Base instantiate call used in cw20 tokens, sets minter, initial balances, base info
/// and contract version
pub fn instantiate(
    deps: &mut DepsMut,
    msg: InstantiateMsg,
    contract_name: &str,
    contract_version: &str,
) -> StdResult<()> {
    set_contract_version(deps.storage, contract_name, contract_version)?;
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
            minter: deps.api.addr_validate(&m.minter)?,
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
    TOKEN_INFO.save(deps.storage, &data)?;
    Ok(())
}

fn create_accounts(deps: &mut DepsMut, accounts: &[Cw20Coin]) -> StdResult<Uint128> {
    let mut total_supply = Uint128::zero();
    for row in accounts {
        let address = deps.api.addr_validate(&row.address)?;
        BALANCES.save(deps.storage, &address, &row.amount)?;
        total_supply += row.amount;
    }
    Ok(total_supply)
}
