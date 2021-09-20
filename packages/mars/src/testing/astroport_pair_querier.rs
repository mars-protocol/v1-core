use std::collections::HashMap;

use cosmwasm_std::{to_binary, Binary, ContractResult, QuerierResult, SystemError};
use terraswap::{asset::PairInfo, factory::QueryMsg};

#[derive(Clone, Default)]
pub struct AstroportPairQuerier {
    pub pairs: HashMap<String, PairInfo>,
}

impl AstroportPairQuerier {
    pub fn handle_query(&self, request: &terraswap::factory::QueryMsg) -> QuerierResult {
        let ret: ContractResult<Binary> = match &request {
            QueryMsg::Pair { asset_infos } => {
                let key = format!("{}-{}", asset_infos[0], asset_infos[1]);
                match self.pairs.get(&key) {
                    Some(pair_info) => to_binary(&pair_info).into(),
                    None => Err(SystemError::InvalidRequest {
                        error: format!("PairInfo is not found for {}", key),
                        request: Default::default(),
                    })
                    .into(),
                }
            }
            _ => panic!("[mock]: Unsupported Astroport Pair query"),
        };

        Ok(ret).into()
    }
}
