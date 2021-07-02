mod helpers;
mod mock_address_provider;
pub use helpers::*;

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

use crate::address_provider;
use crate::ma_token;
use crate::xmars_token;
use terraswap::asset::PairInfo;
use terraswap::factory::QueryMsg;

pub struct MockEnvParams<'a> {
    pub sent_funds: &'a [Coin],
    pub block_time: u64,
    pub block_height: u64,
}

impl<'a> Default for MockEnvParams<'a> {
    fn default() -> Self {
        MockEnvParams {
            sent_funds: &[],
            block_time: 1_571_797_419,
            block_height: 1,
        }
    }
}

/// mock_env replacement for cosmwasm_std::testing::mock_env
pub fn mock_env(sender: &str, mock_env_params: MockEnvParams) -> Env {
    Env {
        block: BlockInfo {
            height: mock_env_params.block_height,
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
) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
    let contract_addr = HumanAddr::from(MOCK_CONTRACT_ADDR);
    let custom_querier: MarsMockQuerier =
        MarsMockQuerier::new(MockQuerier::new(&[(&contract_addr, contract_balance)]));

    Extern {
        storage: MockStorage::default(),
        api: MockApi::new(canonical_length),
        querier: custom_querier,
    }
}

#[derive(Clone, Default, Debug)]
pub struct NativeQuerier {
    /// maps denom to exchange rates
    pub exchange_rates: HashMap<String, HashMap<String, Decimal>>,
}

#[derive(Clone, Debug)]
pub struct Cw20Querier {
    /// maps cw20 contract address to user balances
    pub balances: HashMap<HumanAddr, HashMap<HumanAddr, Uint128>>,
    /// maps cw20 contract address to token info response
    pub token_info_responses: HashMap<HumanAddr, TokenInfoResponse>,
}

impl Cw20Querier {
    fn handle_cw20_query(&self, contract_addr: &HumanAddr, query: Cw20QueryMsg) -> QuerierResult {
        match query {
            Cw20QueryMsg::Balance { address } => {
                let contract_balances = match self.balances.get(&contract_addr) {
                    Some(balances) => balances,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no balance available for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
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
                            request: Default::default(),
                        })
                    }
                };

                Ok(to_binary(&BalanceResponse {
                    balance: *user_balance,
                }))
            }

            Cw20QueryMsg::TokenInfo {} => {
                let token_info_response = match self.token_info_responses.get(&contract_addr) {
                    Some(tir) => tir,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no token_info mock for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
                        })
                    }
                };

                Ok(to_binary(token_info_response))
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            }),
        }
    }

    fn handle_ma_token_query(
        &self,
        contract_addr: &HumanAddr,
        query: ma_token::msg::QueryMsg,
    ) -> QuerierResult {
        match query {
            ma_token::msg::QueryMsg::BalanceAndTotalSupply { address } => {
                let contract_balances = match self.balances.get(&contract_addr) {
                    Some(balances) => balances,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no balance available for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
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
                            request: Default::default(),
                        })
                    }
                };
                let token_info_response = match self.token_info_responses.get(&contract_addr) {
                    Some(tir) => tir,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no token_info mock for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
                        })
                    }
                };

                Ok(to_binary(&ma_token::msg::BalanceAndTotalSupplyResponse {
                    balance: *user_balance,
                    total_supply: token_info_response.total_supply,
                }))
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            }),
        }
    }
}

impl Default for Cw20Querier {
    fn default() -> Self {
        Cw20Querier {
            balances: HashMap::new(),
            token_info_responses: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct XMarsQuerier {
    /// xmars token address to be used in queries
    pub xmars_address: HumanAddr,
    /// maps human address and a block to a specific xmars balance
    pub balances_at: HashMap<(HumanAddr, u64), Uint128>,
    /// maps block to a specific xmars balance
    pub total_supplies_at: HashMap<u64, Uint128>,
}

impl XMarsQuerier {
    fn handle_query(
        &self,
        contract_addr: &HumanAddr,
        query: xmars_token::msg::QueryMsg,
    ) -> QuerierResult {
        if contract_addr != &self.xmars_address {
            panic!(
                "[mock]: made an xmars query but xmars address is incorrect, was: {}, should be {}",
                contract_addr, self.xmars_address
            );
        }

        match query {
            xmars_token::msg::QueryMsg::BalanceAt { address, block } => {
                match self.balances_at.get(&(address.clone(), block)) {
                    Some(balance) => Ok(to_binary(&BalanceResponse { balance: *balance })),
                    None => Err(SystemError::InvalidRequest {
                        error: format!(
                            "[mock]: no balance at block {} for account address {}",
                            block, &address
                        ),
                        request: Default::default(),
                    }),
                }
            }

            xmars_token::msg::QueryMsg::TotalSupplyAt { block } => {
                match self.total_supplies_at.get(&block) {
                    Some(balance) => Ok(to_binary(&xmars_token::msg::TotalSupplyResponse {
                        total_supply: *balance,
                    })),
                    None => Err(SystemError::InvalidRequest {
                        error: format!("[mock]: no total supply at block {}", block),
                        request: Default::default(),
                    }),
                }
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            }),
        }
    }
}

impl Default for XMarsQuerier {
    fn default() -> Self {
        XMarsQuerier {
            xmars_address: HumanAddr::default(),
            balances_at: HashMap::new(),
            total_supplies_at: HashMap::new(),
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

pub struct MarsMockQuerier {
    base: MockQuerier<TerraQueryWrapper>,
    native_querier: NativeQuerier,
    cw20_querier: Cw20Querier,
    xmars_querier: XMarsQuerier,
    terraswap_pair_querier: TerraswapPairQuerier,
}

impl Querier for MarsMockQuerier {
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

impl MarsMockQuerier {
    pub fn new(base: MockQuerier<TerraQueryWrapper>) -> Self {
        MarsMockQuerier {
            base,
            native_querier: NativeQuerier::default(),
            cw20_querier: Cw20Querier::default(),
            xmars_querier: XMarsQuerier::default(),
            terraswap_pair_querier: TerraswapPairQuerier::default(),
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
    #[allow(clippy::or_fun_call)]
    pub fn set_cw20_total_supply(&mut self, cw20_address: HumanAddr, total_supply: Uint128) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.total_supply = total_supply;
    }

    #[allow(clippy::or_fun_call)]
    pub fn set_cw20_symbol(&mut self, cw20_address: HumanAddr, symbol: String) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.symbol = symbol;
    }

    pub fn set_xmars_address(&mut self, address: HumanAddr) {
        self.xmars_querier.xmars_address = address;
    }

    pub fn set_xmars_balance_at(&mut self, address: HumanAddr, block: u64, balance: Uint128) {
        self.xmars_querier
            .balances_at
            .insert((address, block), balance);
    }

    pub fn set_xmars_total_supply_at(&mut self, block: u64, balance: Uint128) {
        self.xmars_querier.total_supplies_at.insert(block, balance);
    }

    pub fn set_terraswap_pair(&mut self, pair_info: PairInfo) {
        let asset_infos = &pair_info.asset_infos;
        let key = format!("{}-{}", asset_infos[0], asset_infos[1]);
        self.terraswap_pair_querier.pairs.insert(key, pair_info);
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

            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => {
                // Cw20 Queries
                let parse_cw20_query: StdResult<Cw20QueryMsg> = from_binary(&msg);
                if let Ok(cw20_query) = parse_cw20_query {
                    return self
                        .cw20_querier
                        .handle_cw20_query(contract_addr, cw20_query);
                }

                // MaToken Queries
                let parse_ma_token_query: StdResult<ma_token::msg::QueryMsg> = from_binary(&msg);
                if let Ok(ma_token_query) = parse_ma_token_query {
                    return self
                        .cw20_querier
                        .handle_ma_token_query(contract_addr, ma_token_query);
                }

                // XMars Queries
                let parse_xmars_query: StdResult<xmars_token::msg::QueryMsg> = from_binary(&msg);
                if let Ok(xmars_query) = parse_xmars_query {
                    return self.xmars_querier.handle_query(contract_addr, xmars_query);
                }

                // Address Provider Queries
                let parse_address_provider_query: StdResult<address_provider::msg::QueryMsg> =
                    from_binary(&msg);
                if let Ok(address_provider_query) = parse_address_provider_query {
                    return mock_address_provider::handle_query(
                        contract_addr,
                        address_provider_query,
                    );
                }

                // Terraswap Queries
                let terraswap_pair_query: StdResult<terraswap::factory::QueryMsg> =
                    from_binary(&msg);
                if let Ok(pair_query) = terraswap_pair_query {
                    return self.terraswap_pair_querier.handle_query(&pair_query);
                }

                panic!("[mock]: Unsupported wasm query: {:?}", msg);
            }

            _ => self.base.handle_query(request),
        }
    }
}

#[derive(Clone, Default)]
pub struct TerraswapPairQuerier {
    pairs: HashMap<String, PairInfo>,
}

impl TerraswapPairQuerier {
    pub fn handle_query(&self, request: &terraswap::factory::QueryMsg) -> QuerierResult {
        match &request {
            QueryMsg::Pair { asset_infos } => {
                let key = format!("{}-{}", asset_infos[0], asset_infos[1]);
                match self.pairs.get(&key) {
                    Some(pair_info) => Ok(to_binary(&pair_info)),
                    None => Err(SystemError::InvalidRequest {
                        error: format!("PairInfo is not found for {}", key),
                        request: Default::default(),
                    }),
                }
            }
            _ => panic!("[mock]: Unsupported Terraswap Pair query"),
        }
    }
}
