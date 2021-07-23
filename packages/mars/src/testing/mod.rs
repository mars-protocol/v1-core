mod helpers;
mod mock_address_provider;
pub use helpers::*;

use cosmwasm_std::testing::{MockApi, MockQuerier, MockStorage, MOCK_CONTRACT_ADDR};
/// cosmwasm_std::testing overrides and custom test helpers
use cosmwasm_std::{
    from_binary, from_slice, to_binary, Addr, Binary, BlockInfo, Coin, ContractInfo,
    ContractResult, Decimal, Env, MessageInfo, OwnedDeps, Querier, QuerierResult, QueryRequest,
    StdResult, SystemError, Timestamp, Uint128, WasmQuery,
};
use cw20::{BalanceResponse, Cw20QueryMsg, TokenInfoResponse};
use std::collections::HashMap;
use terra_cosmwasm::{
    ExchangeRateItem, ExchangeRatesResponse, TaxCapResponse, TaxRateResponse, TerraQuery,
    TerraQueryWrapper, TerraRoute,
};

use crate::address_provider;
use crate::ma_token;
use crate::xmars_token;
use terraswap::asset::PairInfo;
use terraswap::factory::QueryMsg;

pub struct MockEnvParams<'a> {
    pub sent_funds: &'a [Coin],
    pub block_time: Timestamp,
    pub block_height: u64,
}

impl<'a> Default for MockEnvParams<'a> {
    fn default() -> Self {
        MockEnvParams {
            sent_funds: &[],
            block_time: Timestamp::from_nanos(1_571_797_419_879_305_533),
            block_height: 1,
        }
    }
}

/// mock_env replacement for cosmwasm_std::testing::mock_env
pub fn mock_env(mock_env_params: MockEnvParams) -> Env {
    Env {
        block: BlockInfo {
            height: mock_env_params.block_height,
            time: mock_env_params.block_time,
            chain_id: "cosmos-testnet-14002".to_string(),
        },
        contract: ContractInfo {
            address: Addr::unchecked(MOCK_CONTRACT_ADDR),
        },
    }
}

pub fn mock_env_at_block_time(seconds: u64) -> Env {
    mock_env(MockEnvParams {
        block_time: Timestamp::from_seconds(seconds),
        ..Default::default()
    })
}

pub fn mock_env_at_block_height(block_height: u64) -> Env {
    mock_env(MockEnvParams {
        block_height,
        ..Default::default()
    })
}

/// quick mock info with just the sender
pub fn mock_info(sender: &str) -> MessageInfo {
    MessageInfo {
        sender: Addr::unchecked(sender),
        funds: vec![],
    }
}

/// mock_dependencies replacement for cosmwasm_std::testing::mock_dependencies
pub fn mock_dependencies(
    contract_balance: &[Coin],
) -> OwnedDeps<MockStorage, MockApi, MarsMockQuerier> {
    let contract_addr = Addr::unchecked(MOCK_CONTRACT_ADDR);
    let custom_querier: MarsMockQuerier = MarsMockQuerier::new(MockQuerier::new(&[(
        &contract_addr.to_string(),
        contract_balance,
    )]));

    OwnedDeps {
        storage: MockStorage::default(),
        api: MockApi::default(),
        querier: custom_querier,
    }
}

#[derive(Clone, Default, Debug)]
pub struct NativeQuerier {
    /// maps denom to exchange rates
    pub exchange_rates: HashMap<String, HashMap<String, Decimal>>,

    /// maps denom to tax caps
    pub tax_caps: HashMap<String, Uint128>,

    pub tax_rate: Decimal,
}

#[derive(Clone, Debug)]
pub struct Cw20Querier {
    /// maps cw20 contract address to user balances
    pub balances: HashMap<Addr, HashMap<Addr, Uint128>>,
    /// maps cw20 contract address to token info response
    pub token_info_responses: HashMap<Addr, TokenInfoResponse>,
}

impl Cw20Querier {
    fn handle_cw20_query(&self, contract_addr: &Addr, query: Cw20QueryMsg) -> QuerierResult {
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
                        .into()
                    }
                };

                let user_balance = match contract_balances.get(&Addr::unchecked(address)) {
                    Some(balance) => balance,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no balance available for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
                        })
                        .into()
                    }
                };

                Ok(to_binary(&BalanceResponse {
                    balance: *user_balance,
                })
                .into())
                .into()
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
                        .into()
                    }
                };

                Ok(to_binary(token_info_response).into()).into()
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            })
            .into(),
        }
    }

    fn handle_ma_token_query(
        &self,
        contract_addr: &Addr,
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
                        .into()
                    }
                };

                let user_balance = match contract_balances.get(&Addr::unchecked(address)) {
                    Some(balance) => balance,
                    None => {
                        return Err(SystemError::InvalidRequest {
                            error: format!(
                                "no balance available for account address {}",
                                contract_addr
                            ),
                            request: Default::default(),
                        })
                        .into()
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
                        .into()
                    }
                };

                Ok(to_binary(&ma_token::msg::BalanceAndTotalSupplyResponse {
                    balance: *user_balance,
                    total_supply: token_info_response.total_supply,
                })
                .into())
                .into()
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            })
            .into(),
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
    pub xmars_address: Addr,
    /// maps human address and a block to a specific xmars balance
    pub balances_at: HashMap<(Addr, u64), Uint128>,
    /// maps block to a specific xmars balance
    pub total_supplies_at: HashMap<u64, Uint128>,
}

impl XMarsQuerier {
    fn handle_query(
        &self,
        contract_addr: &Addr,
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
                match self
                    .balances_at
                    .get(&(Addr::unchecked(address.clone()), block))
                {
                    Some(balance) => {
                        Ok(to_binary(&BalanceResponse { balance: *balance }).into()).into()
                    }
                    None => Err(SystemError::InvalidRequest {
                        error: format!(
                            "[mock]: no balance at block {} for account address {}",
                            block, &address
                        ),
                        request: Default::default(),
                    })
                    .into(),
                }
            }

            xmars_token::msg::QueryMsg::TotalSupplyAt { block } => {
                match self.total_supplies_at.get(&block) {
                    Some(balance) => Ok(to_binary(&xmars_token::msg::TotalSupplyResponse {
                        total_supply: *balance,
                    })
                    .into())
                    .into(),
                    None => Err(SystemError::InvalidRequest {
                        error: format!("[mock]: no total supply at block {}", block),
                        request: Default::default(),
                    })
                    .into(),
                }
            }

            other_query => Err(SystemError::InvalidRequest {
                error: format!("[mock]: query not supported {:?}", other_query),
                request: Default::default(),
            })
            .into(),
        }
    }
}

impl Default for XMarsQuerier {
    fn default() -> Self {
        XMarsQuerier {
            xmars_address: Addr::unchecked(""),
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
        total_supply: Uint128::zero(),
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
                .into()
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

    /// Set mock querier for tax data
    pub fn set_native_tax(&mut self, tax_rate: Decimal, tax_caps: &[(String, Uint128)]) {
        self.native_querier.tax_rate = tax_rate;
        self.native_querier.tax_caps = tax_caps.iter().cloned().collect();
    }

    /// Set mock querier balances results for a given cw20 token
    pub fn set_cw20_balances(&mut self, cw20_address: Addr, balances: &[(Addr, Uint128)]) {
        self.cw20_querier
            .balances
            .insert(cw20_address, balances.iter().cloned().collect());
    }

    /// Set mock querier so that it returns a specific total supply on the token info query
    /// for a given cw20 token (note this will override existing token info with default
    /// values for the rest of the fields)
    #[allow(clippy::or_fun_call)]
    pub fn set_cw20_total_supply(&mut self, cw20_address: Addr, total_supply: Uint128) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.total_supply = total_supply;
    }

    #[allow(clippy::or_fun_call)]
    pub fn set_cw20_symbol(&mut self, cw20_address: Addr, symbol: String) {
        let token_info = self
            .cw20_querier
            .token_info_responses
            .entry(cw20_address)
            .or_insert(mock_token_info_response());

        token_info.symbol = symbol;
    }

    pub fn set_xmars_address(&mut self, address: Addr) {
        self.xmars_querier.xmars_address = address;
    }

    pub fn set_xmars_balance_at(&mut self, address: Addr, block: u64, balance: Uint128) {
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
                    let ret: ContractResult<Binary> = match query_data {
                        TerraQuery::ExchangeRates {
                            base_denom,
                            quote_denoms,
                        } => {
                            let base_exchange_rates =
                                match self.native_querier.exchange_rates.get(base_denom) {
                                    Some(res) => res,
                                    None => {
                                        let err: ContractResult<Binary> = Err(format!(
                                        "no exchange rates available for provided base denom: {}",
                                        base_denom
                                    ))
                                        .into();
                                        return Ok(err).into();
                                    }
                                };

                            let exchange_rate_items: Result<Vec<ExchangeRateItem>, String> =
                                quote_denoms
                                    .iter()
                                    .map(|denom| {
                                        let exchange_rate = match base_exchange_rates.get(denom) {
                                            Some(rate) => rate,
                                            None => {
                                                return Err(format!(
                                                    "no exchange rate available for {}",
                                                    denom
                                                ))
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
                            to_binary(&res).into()
                        }
                        _ => panic!(
                            "[mock]: Unsupported query data for QueryRequest::Custom : {:?}",
                            query_data
                        ),
                    };
                    Ok(ret).into()
                } else if &TerraRoute::Treasury == route {
                    let ret: ContractResult<Binary> = match query_data {
                        TerraQuery::TaxRate {} => {
                            let res = TaxRateResponse {
                                rate: self.native_querier.tax_rate,
                            };
                            to_binary(&res).into()
                        }
                        TerraQuery::TaxCap { denom } => {
                            match self.native_querier.tax_caps.get(denom) {
                                Some(cap) => {
                                    let res = TaxCapResponse { cap: *cap };
                                    to_binary(&res).into()
                                }
                                None => Err(format!(
                                    "no tax cap available for provided denom: {}",
                                    denom
                                ))
                                .into(),
                            }
                        }
                        _ => panic!(
                            "[mock]: Unsupported query data for QueryRequest::Custom : {:?}",
                            query_data
                        ),
                    };

                    Ok(ret).into()
                } else {
                    panic!(
                        "[mock]: Unsupported route for QueryRequest::Custom : {:?}",
                        route
                    )
                }
            }

            QueryRequest::Wasm(WasmQuery::Smart { contract_addr, msg }) => {
                let contract_addr = Addr::unchecked(contract_addr);
                // Cw20 Queries
                let parse_cw20_query: StdResult<Cw20QueryMsg> = from_binary(&msg);
                if let Ok(cw20_query) = parse_cw20_query {
                    return self
                        .cw20_querier
                        .handle_cw20_query(&contract_addr, cw20_query);
                }

                // MaToken Queries
                let parse_ma_token_query: StdResult<ma_token::msg::QueryMsg> = from_binary(&msg);
                if let Ok(ma_token_query) = parse_ma_token_query {
                    return self
                        .cw20_querier
                        .handle_ma_token_query(&contract_addr, ma_token_query);
                }

                // XMars Queries
                let parse_xmars_query: StdResult<xmars_token::msg::QueryMsg> = from_binary(&msg);
                if let Ok(xmars_query) = parse_xmars_query {
                    return self.xmars_querier.handle_query(&contract_addr, xmars_query);
                }

                // Address Provider Queries
                let parse_address_provider_query: StdResult<address_provider::msg::QueryMsg> =
                    from_binary(&msg);
                if let Ok(address_provider_query) = parse_address_provider_query {
                    return mock_address_provider::handle_query(
                        &contract_addr,
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
            _ => panic!("[mock]: Unsupported Terraswap Pair query"),
        };

        Ok(ret).into()
    }
}
