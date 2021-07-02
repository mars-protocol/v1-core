use cosmwasm_std::{
    to_binary, Api, Binary, CanonicalAddr, Env, Extern, HandleResponse, HumanAddr, InitResponse,
    MigrateResponse, MigrateResult, Querier, StdError, StdResult, Storage,
};

use crate::state;
use crate::state::Config;

use mars::address_provider::msg::{
    ConfigResponse, HandleMsg, InitMsg, MarsContract, MigrateMsg, QueryMsg,
};

use mars::helpers::human_addr_into_canonical;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // Initialize config
    let config = Config {
        owner: deps.api.canonical_address(&msg.owner)?,
        council_address: CanonicalAddr::default(),
        incentives_address: CanonicalAddr::default(),
        insurance_fund_address: CanonicalAddr::default(),
        mars_token_address: CanonicalAddr::default(),
        red_bank_address: CanonicalAddr::default(),
        staking_address: CanonicalAddr::default(),
        treasury_address: CanonicalAddr::default(),
        xmars_token_address: CanonicalAddr::default(),
    };

    state::config(&mut deps.storage).save(&config)?;

    Ok(InitResponse::default())
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::UpdateConfig {
            owner,
            council_address,
            incentives_address,
            insurance_fund_address,
            mars_token_address,
            red_bank_address,
            staking_address,
            treasury_address,
            xmars_token_address,
        } => handle_update_config(
            deps,
            env,
            owner,
            council_address,
            incentives_address,
            insurance_fund_address,
            mars_token_address,
            red_bank_address,
            staking_address,
            treasury_address,
            xmars_token_address,
        ),
    }
}

/// Update config
pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    owner: Option<HumanAddr>,
    council_address: Option<HumanAddr>,
    incentives_address: Option<HumanAddr>,
    insurance_fund_address: Option<HumanAddr>,
    mars_token_address: Option<HumanAddr>,
    red_bank_address: Option<HumanAddr>,
    staking_address: Option<HumanAddr>,
    treasury_address: Option<HumanAddr>,
    xmars_token_address: Option<HumanAddr>,
) -> StdResult<HandleResponse> {
    let mut config = state::config_read(&deps.storage).load()?;

    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    // Update config
    config.owner = human_addr_into_canonical(deps.api, owner, config.owner)?;
    config.council_address =
        human_addr_into_canonical(deps.api, council_address, config.council_address)?;
    config.incentives_address =
        human_addr_into_canonical(deps.api, incentives_address, config.incentives_address)?;
    config.insurance_fund_address = human_addr_into_canonical(
        deps.api,
        insurance_fund_address,
        config.insurance_fund_address,
    )?;
    config.mars_token_address =
        human_addr_into_canonical(deps.api, mars_token_address, config.mars_token_address)?;
    config.red_bank_address =
        human_addr_into_canonical(deps.api, red_bank_address, config.red_bank_address)?;
    config.staking_address =
        human_addr_into_canonical(deps.api, staking_address, config.staking_address)?;
    config.treasury_address =
        human_addr_into_canonical(deps.api, treasury_address, config.treasury_address)?;
    config.xmars_token_address =
        human_addr_into_canonical(deps.api, xmars_token_address, config.xmars_token_address)?;

    state::config(&mut deps.storage).save(&config)?;

    Ok(HandleResponse::default())
}

// QUERIES

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::Address { contract } => to_binary(&query_address(deps, contract)?),
        QueryMsg::Addresses { contracts } => to_binary(&query_addresses(deps, contracts)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = state::config_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        owner: deps.api.human_address(&config.owner)?,
        council_address: deps.api.human_address(&config.council_address)?,
        incentives_address: deps.api.human_address(&config.incentives_address)?,
        insurance_fund_address: deps.api.human_address(&config.insurance_fund_address)?,
        mars_token_address: deps.api.human_address(&config.mars_token_address)?,
        red_bank_address: deps.api.human_address(&config.red_bank_address)?,
        staking_address: deps.api.human_address(&config.staking_address)?,
        treasury_address: deps.api.human_address(&config.treasury_address)?,
        xmars_token_address: deps.api.human_address(&config.xmars_token_address)?,
    })
}

fn query_address<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    contract: MarsContract,
) -> StdResult<HumanAddr> {
    let config = state::config_read(&deps.storage).load()?;
    Ok(get_address(&deps.api, &config, contract)?)
}

fn query_addresses<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    contracts: Vec<MarsContract>,
) -> StdResult<Vec<HumanAddr>> {
    let config = state::config_read(&deps.storage).load()?;
    let mut ret: Vec<HumanAddr> = Vec::with_capacity(contracts.len());
    for contract in contracts {
        ret.push(get_address(&deps.api, &config, contract)?);
    }

    Ok(ret)
}

fn get_address<A: Api>(api: &A, config: &Config, address: MarsContract) -> StdResult<HumanAddr> {
    let canonical_addr = match address {
        MarsContract::Council => &config.council_address,
        MarsContract::Incentives => &config.incentives_address,
        MarsContract::InsuranceFund => &config.insurance_fund_address,
        MarsContract::MarsToken => &config.mars_token_address,
        MarsContract::RedBank => &config.red_bank_address,
        MarsContract::Staking => &config.staking_address,
        MarsContract::Treasury => &config.treasury_address,
        MarsContract::XMarsToken => &config.xmars_token_address,
    };

    api.human_address(&canonical_addr)
}

// MIGRATION

pub fn migrate<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _msg: MigrateMsg,
) -> MigrateResult {
    Ok(MigrateResponse::default())
}

// HELPERS

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{from_binary, Coin};
    use mars::testing::{
        get_test_addresses, mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams,
    };

    use cosmwasm_std::testing::{MockApi, MockStorage};

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);
        let (owner_address, owner_canonical_address) = get_test_addresses(&deps.api, "owner");

        // *
        // init config with empty params
        // *
        let msg = InitMsg {
            owner: owner_address,
        };
        let env = mock_env("owner", MockEnvParams::default());
        init(&mut deps, env, msg).unwrap();

        let config = state::config_read(&deps.storage).load().unwrap();
        assert_eq!(owner_canonical_address, config.owner);
    }

    #[test]
    fn test_update_config() {
        let mut deps = th_setup(&[]);
        // *
        // non owner is not authorized
        // *
        {
            let msg = HandleMsg::UpdateConfig {
                owner: None,
                council_address: None,
                incentives_address: Some(HumanAddr::from("incentives")),
                insurance_fund_address: None,
                mars_token_address: Some(HumanAddr::from("mars-token")),
                red_bank_address: None,
                staking_address: None,
                treasury_address: Some(HumanAddr::from("treasury")),
                xmars_token_address: None,
            };
            let env = cosmwasm_std::testing::mock_env("somebody", &[]);
            let error_res = handle(&mut deps, env, msg).unwrap_err();
            assert_eq!(error_res, StdError::unauthorized());
        }

        // *
        // update config
        // *
        {
            let msg = HandleMsg::UpdateConfig {
                owner: None,
                council_address: None,
                incentives_address: Some(HumanAddr::from("incentives")),
                insurance_fund_address: None,
                mars_token_address: Some(HumanAddr::from("mars-token")),
                red_bank_address: None,
                staking_address: None,
                treasury_address: Some(HumanAddr::from("treasury")),
                xmars_token_address: None,
            };
            let env = cosmwasm_std::testing::mock_env("owner", &[]);
            // we can just call .unwrap() to assert this was a success
            let res = handle(&mut deps, env, msg).unwrap();
            assert_eq!(0, res.messages.len());

            // Read config from state
            let new_config = state::config_read(&deps.storage).load().unwrap();

            assert_eq!(
                new_config.owner,
                deps.api
                    .canonical_address(&HumanAddr::from("owner"))
                    .unwrap()
            );
            assert_eq!(new_config.xmars_token_address, CanonicalAddr::default(),);
            assert_eq!(
                new_config.incentives_address,
                deps.api
                    .canonical_address(&HumanAddr::from("incentives"))
                    .unwrap()
            );
            assert_eq!(
                new_config.mars_token_address,
                deps.api
                    .canonical_address(&HumanAddr::from("mars-token"))
                    .unwrap()
            );
            assert_eq!(
                new_config.treasury_address,
                deps.api
                    .canonical_address(&HumanAddr::from("treasury"))
                    .unwrap()
            );
        }
    }

    #[test]
    fn test_address_queries() {
        let mut deps = th_setup(&[]);

        let (council_address, council_canonical_address) = get_test_addresses(&deps.api, "council");
        let (incentives_address, incentives_canonical_address) =
            get_test_addresses(&deps.api, "incentives");
        let (xmars_token_address, xmars_token_canonical_address) =
            get_test_addresses(&deps.api, "xmars-token");

        state::config(&mut deps.storage)
            .update(|mut config: Config| {
                config.council_address = council_canonical_address;
                config.incentives_address = incentives_canonical_address;
                config.xmars_token_address = xmars_token_canonical_address;
                Ok(config)
            })
            .unwrap();

        {
            let address_query = query(
                &deps,
                QueryMsg::Address {
                    contract: MarsContract::Incentives,
                },
            )
            .unwrap();
            let result: HumanAddr = from_binary(&address_query).unwrap();
            assert_eq!(result, incentives_address);
        }

        {
            let addresses_query = query(
                &deps,
                QueryMsg::Addresses {
                    contracts: vec![MarsContract::XMarsToken, MarsContract::Council],
                },
            )
            .unwrap();
            let result: Vec<HumanAddr> = from_binary(&addresses_query).unwrap();
            assert_eq!(result[0], xmars_token_address);
            assert_eq!(result[1], council_address);
        }
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);
        let msg = InitMsg {
            owner: HumanAddr::from("owner"),
        };
        let env = mock_env("owner", MockEnvParams::default());
        init(&mut deps, env, msg).unwrap();
        deps
    }
}
