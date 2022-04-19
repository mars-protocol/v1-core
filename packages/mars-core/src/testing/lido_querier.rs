use cosmwasm_std::{to_binary, Binary, ContractResult, QuerierResult};

use crate::lido::{QueryMsg, StateResponse};

#[derive(Clone, Default)]
pub struct LidoQuerier {
    pub state_response: Option<StateResponse>,
}

impl LidoQuerier {
    pub fn handle_query(&self, request: &QueryMsg) -> QuerierResult {
        let ret: ContractResult<Binary> = match &request {
            QueryMsg::State {} => match self.state_response.as_ref() {
                Some(resp) => to_binary(resp).into(),
                None => panic!("[mock]: StateResponse is not provided for query"),
            },
        };

        Ok(ret).into()
    }
}
