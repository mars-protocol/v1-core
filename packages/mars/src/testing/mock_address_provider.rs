use cosmwasm_std::{to_binary, HumanAddr, QuerierResult, SystemError};

use crate::address_provider::msg::{MarsContract, QueryMsg};

// NOTE: Addresses here are all hardcoded as we always use those to target a specific contract
// in tests. This module implicitly supposes those are used.
// Having to explicitly inject each address on each needed test vs doing this
// seems overkill for now.

pub fn handle_query(contract_addr: &HumanAddr, query: QueryMsg) -> QuerierResult {
    let address_provider = HumanAddr::from("address_provider");
    if *contract_addr != address_provider {
        return Err(SystemError::InvalidRequest {
            error: format!(
                "[mock]: Address provider request made to {} shoud be {}",
                contract_addr, address_provider
            ),
            request: Default::default(),
        });
    }

    match query {
        QueryMsg::Address { contract } => Ok(to_binary(&get_contract_address(contract))),

        QueryMsg::Addresses { contracts } => {
            let mut ret: Vec<HumanAddr> = Vec::with_capacity(contracts.len());
            for contract in contracts {
                ret.push(get_contract_address(contract));
            }
            Ok(to_binary(&ret))
        }

        _ => panic!("[mock]: Unsupported address provider query"),
    }
}

fn get_contract_address(contract: MarsContract) -> HumanAddr {
    match contract {
        MarsContract::Council => HumanAddr::from("council"),
        MarsContract::Incentives => HumanAddr::from("incentives_address"),
        MarsContract::InsuranceFund => HumanAddr::from("insurance_fund"),
        MarsContract::MarsToken => HumanAddr::from("mars_token"),
        MarsContract::RedBank => HumanAddr::from("red_bank"),
        MarsContract::Staking => HumanAddr::from("staking"),
        MarsContract::Treasury => HumanAddr::from("treasury"),
        MarsContract::XMarsToken => HumanAddr::from("xmars_token"),
    }
}
