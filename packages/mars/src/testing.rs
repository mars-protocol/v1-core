/// cosmwasm_std::testing overrides and custom test helpers

use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_binary, from_slice, to_binary, Api, Coin, Decimal, Extern, HumanAddr, Querier,
    QuerierResult, QueryRequest, StdError, StdResult, SystemError, Uint128, WasmQuery,
};
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};
use std::collections::HashMap;
use terra_cosmwasm::{
    ExchangeRateItem, ExchangeRatesResponse, TerraQuery, TerraQueryWrapper, TerraRoute,
};

/// mock_dependencies is a drop-in replacement for cosmwasm_std::testing::mock_dependencies
/// in order to add a custom querier 
pub fn mock_dependencies(
    canonical_length: usize,
    contract_balance: &[Coin],
) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
    let contract_addr = HumanAddr::from(MOCK_CONTRACT_ADDR);
    let custom_querier: WasmMockQuerier = WasmMockQuerier::new(
        MockQuerier::new(&[(&contract_addr, contract_balance)]),
        MockApi::new(canonical_length),
    );

    Extern {
        storage: MockStorage::default(),
        api: MockApi::new(canonical_length),
        querier: custom_querier,
    }
}

#[derive(Clone, Default, Debug)]
pub struct NativeQuerier {
    /// maps denom to exchange rates
    exchange_rates: HashMap<String, HashMap<String, Decimal>>,
}

#[derive(Clone, Debug)]
pub struct Cw20Querier {
    /// maps cw20 contract address to user balances
    balances: HashMap<HumanAddr, HashMap<HumanAddr, Uint128>>,
    token_info_responses: HashMap<HumanAddr, TokenInfoResponse>,
}

impl Cw20Querier {
    fn new() -> Self {
        Cw20Querier {
            balances: HashMap::new(),
            token_info_responses: HashMap::new(),
        }
    }
}

pub fn mock_token_info_response() -> TokenInfoResponse {
    TokenInfoResponse {
        name: "".to_string(),
        symbol: "".to_string(),
        decimals: 0,
        total_supply: Uint128(0),
    }
}

pub struct WasmMockQuerier {
    base: MockQuerier<TerraQueryWrapper>,
    native_querier: NativeQuerier,
    cw20_querier: Cw20Querier,
}

impl Querier for WasmMockQuerier {
    fn raw_query(&self, bin_request: &[u8]) -> QuerierResult {
        // MockQuerier doesn't support Custom, so we ignore it completely here
        let request: QueryRequest<TerraQueryWrapper> = match from_slice(bin_request) {
            Ok(v) => v,
            Err(e) => {
                return Err(SystemError::InvalidRequest {
                    error: format!("Parsing query request: {}", e),
                    request: bin_request.into(),
                })
            }
        };
        self.handle_query(&request)
    }
}

impl WasmMockQuerier {
    // TODO: Why is the api needed here? Is it for the type to be set somehow
    pub fn new<A: Api>(base: MockQuerier<TerraQueryWrapper>, _api: A) -> Self {
        WasmMockQuerier {
            base,
            native_querier: NativeQuerier::default(),
            cw20_querier: Cw20Querier::new(),
        }
    }

    /// Set querirer balances for native exchange rates taken as a list of tuples
    pub fn set_native_exchange_rates(
        &mut self, 
        base_denom: String,
        exchange_rates: &[(String, Decimal)])
    {
        self.native_querier.exchange_rates.insert(base_denom, exchange_rates.iter().cloned().collect());
    }

    /// Set mock querirer balances for a cw20 token as a list of tuples in the form
    pub fn set_cw20_balances(
        &mut self,
        cw20_address: HumanAddr,
        balances: &[(HumanAddr, Uint128)]) 
    {
        self.cw20_querier.balances.insert(cw20_address, balances.iter().cloned().collect());
    }

    pub fn set_cw20_total_supply(&mut self, cw20_address: HumanAddr, total_supply: Uint128) {
        let mut token_info = mock_token_info_response();
        token_info.total_supply = total_supply;
        self.cw20_querier.token_info_responses.insert(cw20_address, token_info);
    }

    pub fn handle_query(&self, request: &QueryRequest<TerraQueryWrapper>) -> QuerierResult {
        match &request {
            QueryRequest::Custom(TerraQueryWrapper { route, query_data }) => {
                if &TerraRoute::Oracle == route {
                    match query_data {
                        TerraQuery::ExchangeRates {
                            base_denom,
                            quote_denoms,
                        } => {
                            let base_exchange_rates =
                                match self.native_querier.exchange_rates.get(base_denom) {
                                    Some(res) => res,
                                    None => return Err(SystemError::InvalidRequest {
                                        error:
                                            "no exchange rates available for provided base denom"
                                                .to_string(),
                                        request: Default::default(),
                                    }),
                                };

                            let exchange_rate_items: StdResult<Vec<ExchangeRateItem>> =
                                quote_denoms
                                    .iter()
                                    .map(|denom| {
                                        let exchange_rate = match base_exchange_rates.get(denom) {
                                            Some(rate) => rate,
                                            None => {
                                                return Err(StdError::generic_err(format!(
                                                    "no exchange rate available for {}",
                                                    denom
                                                )))
                                            }
                                        };

                                        Ok(ExchangeRateItem {
                                            quote_denom: denom.into(),
                                            exchange_rate: *exchange_rate,
                                        })
                                    })
                                    .collect();

                            let res = ExchangeRatesResponse {
                                base_denom: base_denom.into(),
                                exchange_rates: exchange_rate_items.unwrap(),
                            };
                            Ok(to_binary(&res))
                        }
                        _ => panic!("[mock]: Unsupported query data for QueryRequest::Custom : {:?}", query_data),
                    }
                } else {
                    panic!("[mock]: Unsupported route for QueryRequest::Custom : {:?}", route)
                }
            }

            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => match from_binary(&msg)
                .unwrap()
            {
                Cw20QueryMsg::Balance { address } => {
                    let contract_balances = match self.cw20_querier.balances.get(&contract_addr)
                    {
                        Some(balances) => balances,
                        None => {
                            return Err(SystemError::InvalidRequest {
                                error: "no balances available for provided contract address"
                                    .to_string(),
                                request: msg.as_slice().into(),
                            })
                        }
                    };

                    let user_balance = match contract_balances.get(&address) {
                        Some(balance) => balance,
                        None => {
                            return Err(SystemError::InvalidRequest {
                                error: "no balance available for provided account address"
                                    .to_string(),
                                request: msg.as_slice().into(),
                            })
                        }
                    };

                    Ok(to_binary(&BalanceResponse {
                        balance: *user_balance,
                    }))
                }

                Cw20QueryMsg::TokenInfo {} => {
                    let token_info_response = 
                        match self.cw20_querier.balances.get(&contract_addr) {
                            Some(tir) => tir,
                            None => {
                                return Err(SystemError::InvalidRequest {
                                    error: format!("no token_info mock for account address {}", contract_addr),
                                    request: msg.as_slice().into(),
                                })
                            }
                        };

                    Ok(to_binary(token_info_response))
                }

                other_query => panic!("[mock]: Unsupported wasm query: {:?}", other_query)
            }

            _ => self.base.handle_query(request),
        }
    }
}
