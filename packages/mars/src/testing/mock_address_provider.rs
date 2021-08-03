use cosmwasm_std::{to_binary, Addr, Binary, ContractResult, QuerierResult};

use crate::address_provider::msg::{MarsContract, QueryMsg};

// NOTE: Addresses here are all hardcoded as we always use those to target a specific contract
// in tests. This module implicitly supposes those are used.
// Having to explicitly inject each address on each needed test vs doing this
// seems overkill for now.

pub fn handle_query(contract_addr: &Addr, query: QueryMsg) -> QuerierResult {
    let address_provider = Addr::unchecked("address_provider");
    if *contract_addr != address_provider {
        panic!(
            "[mock]: Address provider request made to {} shoud be {}",
            contract_addr, address_provider
        );
    }

    let ret: ContractResult<Binary> = match query {
        QueryMsg::Address { contract } => to_binary(&get_contract_address(contract)).into(),

        QueryMsg::Addresses { contracts } => {
            let mut ret: Vec<Addr> = Vec::with_capacity(contracts.len());
            for contract in contracts {
                ret.push(get_contract_address(contract));
            }
            to_binary(&ret).into()
        }

        _ => panic!("[mock]: Unsupported address provider query"),
    };

    Ok(ret).into()
}

fn get_contract_address(contract: MarsContract) -> Addr {
    match contract {
        MarsContract::Council => Addr::unchecked("council"),
        MarsContract::Incentives => Addr::unchecked("incentives"),
        MarsContract::InsuranceFund => Addr::unchecked("insurance_fund"),
        MarsContract::MarsToken => Addr::unchecked("mars_token"),
        MarsContract::RedBank => Addr::unchecked("red_bank"),
        MarsContract::Staking => Addr::unchecked("staking"),
        MarsContract::Treasury => Addr::unchecked("treasury"),
        MarsContract::XMarsToken => Addr::unchecked("xmars_token"),
    }
}
