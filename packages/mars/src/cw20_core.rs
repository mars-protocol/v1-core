/// cw20_core: Shared functionality for cw20 tokens
use cosmwasm_std::{DepsMut, StdError, StdResult, Uint128};
use cw20_base::msg::InstantiateMsg;
use cw20_base::state::{MinterData, TokenInfo, TOKEN_INFO};

pub fn instantiate_token_info(
    deps: &mut DepsMut,
    msg: InstantiateMsg,
    total_supply: Uint128,
) -> StdResult<()> {
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
