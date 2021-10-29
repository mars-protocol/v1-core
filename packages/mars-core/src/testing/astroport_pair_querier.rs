use std::collections::HashMap;

use cosmwasm_std::{to_binary, Addr, Binary, ContractResult, QuerierResult, SystemError};

use crate::astroport::pair::{PoolResponse, QueryMsg};

#[derive(Clone, Default)]
pub struct AstroportPairQuerier {
    pub pairs: HashMap<String, PoolResponse>,
}

impl AstroportPairQuerier {
    pub fn handle_query(&self, contract_addr: &Addr, request: &QueryMsg) -> QuerierResult {
        let key = contract_addr.to_string();
        let ret: ContractResult<Binary> = match &request {
            QueryMsg::Pool {} => match self.pairs.get(&key) {
                Some(pool_response) => to_binary(&pool_response).into(),
                None => Err(SystemError::InvalidRequest {
                    error: format!("PoolResponse is not found for {}", key),
                    request: Default::default(),
                })
                .into(),
            },
            _ => panic!("[mock]: Unsupported Astroport pair query"),
        };

        Ok(ret).into()
    }
}
