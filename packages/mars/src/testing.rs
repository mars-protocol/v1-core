use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
/// cosmwasm_std::testing overrides and custom test helpers
use cosmwasm_std::{
    from_binary, from_slice, to_binary, BlockInfo, Coin, ContractInfo, Decimal, Env, Extern,
    HumanAddr, MessageInfo, Querier, QuerierResult, QueryRequest, StdError, StdResult, SystemError,
    Uint128, WasmQuery,
};
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};
use std::collections::HashMap;
use terra_cosmwasm::{
    ExchangeRateItem, ExchangeRatesResponse, TerraQuery, TerraQueryWrapper, TerraRoute,
};

pub struct MockEnvParams<'a> {
    pub sent_funds: &'a [Coin],
    pub block_time: u64,
}

impl<'a> Default for MockEnvParams<'a> {
    fn default() -> Self {
        MockEnvParams {
            sent_funds: &[],
            block_time: 1_571_797_419,
        }
    }
}

/// mock_env replacement for cosmwasm_std::testing::mock_env
pub fn mock_env(sender: &str, mock_env_params: MockEnvParams) -> Env {
    Env {
        block: BlockInfo {
            height: 12_345,
            time: mock_env_params.block_time,
            chain_id: "cosmos-testnet-14002".to_string(),
        },
        message: MessageInfo {
            sender: HumanAddr::from(sender),
            sent_funds: mock_env_params.sent_funds.to_vec(),
        },
        contract: ContractInfo {
            address: HumanAddr::from(MOCK_CONTRACT_ADDR),
        },
    }
}

/// mock_dependencies replacement for cosmwasm_std::testing::mock_dependencies
pub fn mock_dependencies(
    canonical_length: usize,
    contract_balance: &[Coin],
) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
    let contract_addr = HumanAddr::from(MOCK_CONTRACT_ADDR);
    let custom_querier: WasmMockQuerier =
        WasmMockQuerier::new(MockQuerier::new(&[(&contract_addr, contract_balance)]));

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
    pub fn new(base: MockQuerier<TerraQueryWrapper>) -> Self {
        WasmMockQuerier {
            base,
            native_querier: NativeQuerier::default(),
            cw20_querier: Cw20Querier::new(),
        }
    }

    /// Set mock querier exchange rates query results for a given denom
    pub fn set_native_exchange_rates(
        &mut self,
        base_denom: String,
        exchange_rates: &[(String, Decimal)],
    ) {
        self.native_querier
            .exchange_rates
            .insert(base_denom, exchange_rates.iter().cloned().collect());
    }

    /// Set mock querier balances results for a given cw20 token
    pub fn set_cw20_balances(
        &mut self,
        cw20_address: HumanAddr,
        balances: &[(HumanAddr, Uint128)],
    ) {
        self.cw20_querier
            .balances
            .insert(cw20_address, balances.iter().cloned().collect());
    }

    /// Set mock querier so that it returns a specific total supply on the token info query
    /// for a given cw20 token (note this will override existing token info with default
    /// values for the rest of the fields)
    pub fn set_cw20_total_supply(&mut self, cw20_address: HumanAddr, total_supply: Uint128) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.total_supply = total_supply;
    }

    pub fn set_cw20_symbol(&mut self, cw20_address: HumanAddr, symbol: String) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.symbol = symbol;
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
                        _ => panic!(
                            "[mock]: Unsupported query data for QueryRequest::Custom : {:?}",
                            query_data
                        ),
                    }
                } else {
                    panic!(
                        "[mock]: Unsupported route for QueryRequest::Custom : {:?}",
                        route
                    )
                }
            }

            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => match from_binary(&msg)
                .unwrap()
            {
                Cw20QueryMsg::Balance { address } => {
                    let contract_balances = match self.cw20_querier.balances.get(&contract_addr) {
                        Some(balances) => balances,
                        None => {
                            return Err(SystemError::InvalidRequest {
                                error: format!(
                                    "no balance available for account address {}",
                                    contract_addr
                                ),
                                request: msg.as_slice().into(),
                            })
                        }
                    };

                    let user_balance = match contract_balances.get(&address) {
                        Some(balance) => balance,
                        None => {
                            return Err(SystemError::InvalidRequest {
                                error: format!(
                                    "no balance available for account address {}",
                                    contract_addr
                                ),
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
                        match self.cw20_querier.token_info_responses.get(&contract_addr) {
                            Some(tir) => tir,
                            None => {
                                return Err(SystemError::InvalidRequest {
                                    error: format!(
                                        "no token_info mock for account address {}",
                                        contract_addr
                                    ),
                                    request: msg.as_slice().into(),
                                })
                            }
                        };

                    Ok(to_binary(token_info_response))
                }

                other_query => panic!("[mock]: Unsupported wasm query: {:?}", other_query),
            },

            _ => self.base.handle_query(request),
        }
    }
}
