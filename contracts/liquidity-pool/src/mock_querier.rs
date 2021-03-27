use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
use cosmwasm_std::{
    from_slice, to_binary, Api, Coin, Decimal, Extern, HumanAddr, Querier, QuerierResult,
    QueryRequest, StdError, StdResult, SystemError,
};
use std::collections::HashMap;
use terra_cosmwasm::{
    ExchangeRateItem, ExchangeRatesResponse, TerraQuery, TerraQueryWrapper, TerraRoute,
};

/// mock_dependencies is a drop-in replacement for cosmwasm_std::testing::mock_dependencies
/// this uses our CustomQuerier.
pub fn mock_dependencies(
    canonical_length: usize,
    contract_balance: &[Coin],
) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
    let contract_addr = HumanAddr::from(MOCK_CONTRACT_ADDR);
    let custom_querier: WasmMockQuerier = WasmMockQuerier::new(
        MockQuerier::new(&[(&contract_addr, contract_balance)]),
        canonical_length,
        MockApi::new(canonical_length),
    );

    Extern {
        storage: MockStorage::default(),
        api: MockApi::new(canonical_length),
        querier: custom_querier,
    }
}

pub struct WasmMockQuerier {
    base: MockQuerier<TerraQueryWrapper>,
    exchange_rate_querier: ExchangeRateQuerier,
    canonical_length: usize,
}

#[derive(Clone, Default)]
pub struct ExchangeRateQuerier {
    // maps denom to exchange rates
    exchange_rates: HashMap<String, HashMap<String, Decimal>>,
}

impl ExchangeRateQuerier {
    pub fn new(exchange_rates: &[(&String, &[(&String, &Decimal)])]) -> Self {
        ExchangeRateQuerier {
            exchange_rates: exchange_rates_to_map(exchange_rates),
        }
    }
}

pub(crate) fn exchange_rates_to_map(
    exchange_rates: &[(&String, &[(&String, &Decimal)])],
) -> HashMap<String, HashMap<String, Decimal>> {
    let mut exchange_rates_map: HashMap<String, HashMap<String, Decimal>> = HashMap::new();
    for (denom, exchange_rates) in exchange_rates.iter() {
        let mut denom_exchange_rates_map: HashMap<String, Decimal> = HashMap::new();
        for (denom, rate) in exchange_rates.iter() {
            denom_exchange_rates_map.insert((**denom.clone()).parse().unwrap(), **rate);
        }

        exchange_rates_map.insert((**denom.clone()).parse().unwrap(), denom_exchange_rates_map);
    }
    exchange_rates_map
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
                                match self.exchange_rate_querier.exchange_rates.get(base_denom) {
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
                        _ => panic!("DO NOT ENTER HERE"),
                    }
                } else {
                    panic!("DO NOT ENTER HERE")
                }
            }
            _ => self.base.handle_query(request),
        }
    }
}

impl WasmMockQuerier {
    pub fn new<A: Api>(
        base: MockQuerier<TerraQueryWrapper>,
        canonical_length: usize,
        _api: A,
    ) -> Self {
        WasmMockQuerier {
            base,
            exchange_rate_querier: ExchangeRateQuerier::default(),
            canonical_length,
        }
    }

    // configure the exchange rates mock querier
    pub fn with_exchange_rates(&mut self, exchange_rates: &[(&String, &[(&String, &Decimal)])]) {
        self.exchange_rate_querier = ExchangeRateQuerier::new(exchange_rates);
    }
}
