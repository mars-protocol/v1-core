use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Decimal, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Order, Querier,
    QueryRequest, StdError, StdResult, Storage, Uint128, WasmMsg, WasmQuery,
};

use cw0::calc_range_start_human;
use cw20::{Cw20HandleMsg, Cw20ReceiveMsg};
use mars::address_provider;
use mars::address_provider::msg::MarsContract;
use mars::helpers::{human_addr_into_canonical, read_be_u64};
use mars::xmars_token;

use crate::msg::{
    ConfigResponse, CreateOrUpdateConfig, HandleMsg, InitMsg, MigrateMsg, MsgExecuteCall,
    ProposalInfo, ProposalVoteResponse, ProposalVotesResponse, ProposalsListResponse, QueryMsg,
    ReceiveMsg,
};
use crate::state;
use crate::state::{
    Config, Council, Proposal, ProposalExecuteCall, ProposalStatus, ProposalVote,
    ProposalVoteOption,
};

// CONSTANTS
const MIN_TITLE_LENGTH: usize = 4;
const MAX_TITLE_LENGTH: usize = 64;
const MIN_DESC_LENGTH: usize = 4;
const MAX_DESC_LENGTH: usize = 1024;
const MIN_LINK_LENGTH: usize = 12;
const MAX_LINK_LENGTH: usize = 128;
const DEFAULT_START: u64 = 1;
const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 30;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    _env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        address_provider_address,
        proposal_voting_period,
        proposal_effective_delay,
        proposal_expiration_period,
        proposal_required_deposit,
        proposal_required_quorum,
        proposal_required_threshold,
    } = msg.config;

    // Check required fields are available
    let available = address_provider_address.is_some()
        && proposal_voting_period.is_some()
        && proposal_effective_delay.is_some()
        && proposal_expiration_period.is_some()
        && proposal_required_deposit.is_some()
        && proposal_required_quorum.is_some()
        && proposal_required_threshold.is_some();

    if !available {
        return Err(StdError::generic_err(
            "All params should be available during initialization",
        ));
    };

    // initialize Config
    let config = Config {
        address_provider_address: deps
            .api
            .canonical_address(&address_provider_address.unwrap())?,

        proposal_voting_period: proposal_voting_period.unwrap(),
        proposal_effective_delay: proposal_effective_delay.unwrap(),
        proposal_expiration_period: proposal_expiration_period.unwrap(),
        proposal_required_deposit: proposal_required_deposit.unwrap(),
        proposal_required_quorum: proposal_required_quorum.unwrap(),
        proposal_required_threshold: proposal_required_threshold.unwrap(),
    };

    // Validate config
    config.validate()?;

    state::config(&mut deps.storage).save(&config)?;

    // initialize State
    state::council(&mut deps.storage).save(&Council { proposal_count: 0 })?;

    // Prepare response, should instantiate Mars and use the Register hook
    Ok(InitResponse::default())
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Receive(cw20_msg) => handle_receive_cw20(deps, env, cw20_msg),

        HandleMsg::CastVote { proposal_id, vote } => handle_cast_vote(deps, env, proposal_id, vote),
        HandleMsg::EndProposal { proposal_id } => handle_end_proposal(deps, env, proposal_id),

        HandleMsg::ExecuteProposal { proposal_id } => {
            handle_execute_proposal(deps, env, proposal_id)
        }

        HandleMsg::UpdateConfig { config } => handle_update_config(deps, env, config),
    }
}

/// cw20 receive implementation
pub fn handle_receive_cw20<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<HandleResponse> {
    if let Some(msg) = cw20_msg.msg {
        match from_binary(&msg)? {
            ReceiveMsg::SubmitProposal {
                title,
                description,
                link,
                execute_calls,
            } => handle_submit_proposal(
                deps,
                env,
                cw20_msg.sender,
                cw20_msg.amount,
                title,
                description,
                link,
                execute_calls,
            ),
        }
    } else {
        Err(StdError::generic_err("Invalid Cw20ReceiveMsg"))
    }
}

pub fn handle_submit_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    submitter_address: HumanAddr,
    deposit_amount: Uint128,
    title: String,
    description: String,
    option_link: Option<String>,
    option_msg_execute_calls: Option<Vec<MsgExecuteCall>>,
) -> StdResult<HandleResponse> {
    // Validate title
    if title.len() < MIN_TITLE_LENGTH {
        return Err(StdError::generic_err("Title too short"));
    }
    if title.len() > MAX_TITLE_LENGTH {
        return Err(StdError::generic_err("Title too long"));
    }

    // Validate description
    if description.len() < MIN_DESC_LENGTH {
        return Err(StdError::generic_err("Description too short"));
    }
    if description.len() > MAX_DESC_LENGTH {
        return Err(StdError::generic_err("Description too long"));
    }

    // Validate Link
    if let Some(link) = &option_link {
        if link.len() < MIN_LINK_LENGTH {
            return Err(StdError::generic_err("Link too short"));
        }
        if link.len() > MAX_LINK_LENGTH {
            return Err(StdError::generic_err("Link too long"));
        }
    }

    let config = state::config_read(&deps.storage).load()?;
    let mars_token_address = address_provider::helpers::query_address(
        &deps,
        &config.address_provider_address,
        MarsContract::MarsToken,
    )?;

    if env.message.sender != mars_token_address {
        return Err(StdError::unauthorized());
    }

    // Validate deposit amount
    if deposit_amount < config.proposal_required_deposit {
        return Err(StdError::generic_err(format!(
            "Must deposit at least {} tokens",
            config.proposal_required_deposit
        )));
    }

    // Update proposal totals
    let mut council_singleton = state::council(&mut deps.storage);
    let mut council = council_singleton.load()?;
    council.proposal_count += 1;
    council_singleton.save(&council)?;

    // Transform MsgExecuteCalls into ProposalExecuteCalls by canonicalizing the contract address
    let option_proposal_execute_calls = if let Some(calls) = option_msg_execute_calls {
        let mut proposal_execute_calls: Vec<ProposalExecuteCall> = vec![];
        for call in calls {
            proposal_execute_calls.push(ProposalExecuteCall {
                execution_order: call.execution_order,
                target_contract_canonical_address: deps
                    .api
                    .canonical_address(&call.target_contract_address)?,
                msg: call.msg,
            });
        }
        Some(proposal_execute_calls)
    } else {
        None
    };

    let new_proposal = Proposal {
        submitter_canonical_address: deps.api.canonical_address(&submitter_address)?,
        status: ProposalStatus::Active,
        for_votes: Uint128::zero(),
        against_votes: Uint128::zero(),
        start_height: env.block.height,
        end_height: env.block.height + config.proposal_voting_period,
        title,
        description,
        link: option_link,
        execute_calls: option_proposal_execute_calls,
        deposit_amount,
    };
    state::proposals(&mut deps.storage)
        .save(&council.proposal_count.to_be_bytes(), &new_proposal)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "submit_proposal"),
            log("proposal_submitter", &submitter_address),
            log("proposal_id", &council.proposal_count),
            log("proposal_end_height", &new_proposal.end_height),
        ],
        data: None,
    })
}

pub fn handle_cast_vote<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    proposal_id: u64,
    vote_option: ProposalVoteOption,
) -> StdResult<HandleResponse> {
    let mut proposal = state::proposals_read(&deps.storage).load(&proposal_id.to_be_bytes())?;
    if proposal.status != ProposalStatus::Active {
        return Err(StdError::generic_err("Proposal is not active"));
    }

    if env.block.height > proposal.end_height {
        return Err(StdError::generic_err("Proposal has expired"));
    }

    let voter_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    if state::proposal_votes_read(&deps.storage, proposal_id)
        .may_load(voter_canonical_address.as_slice())?
        .is_some()
    {
        return Err(StdError::generic_err(
            "User has already voted in this proposal",
        ));
    }

    let config = state::config_read(&deps.storage).load()?;
    let xmars_token_address = address_provider::helpers::query_address(
        &deps,
        &config.address_provider_address,
        MarsContract::XMarsToken,
    )?;

    let balance_at_block = proposal.start_height - 1;
    let voting_power = xmars_get_balance_at(
        &deps.querier,
        xmars_token_address,
        env.message.sender.clone(),
        balance_at_block,
    )?;

    if voting_power == Uint128::zero() {
        return Err(StdError::generic_err(format!(
            "User has no balance at block: {}",
            balance_at_block
        )));
    }

    match vote_option {
        ProposalVoteOption::For => proposal.for_votes += voting_power,
        ProposalVoteOption::Against => proposal.against_votes += voting_power,
    };

    state::proposal_votes(&mut deps.storage, proposal_id).save(
        voter_canonical_address.as_slice(),
        &ProposalVote {
            option: vote_option.clone(),
            power: voting_power,
        },
    )?;

    state::proposals(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "cast_vote"),
            log("proposal_id", proposal_id),
            log("voter", &env.message.sender),
            log("vote", vote_option),
            log("voting_power", voting_power),
        ],
        data: None,
    })
}

pub fn handle_end_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    proposal_id: u64,
) -> StdResult<HandleResponse> {
    let proposals_bucket = state::proposals(&mut deps.storage);
    let mut proposal = proposals_bucket.load(&proposal_id.to_be_bytes())?;

    if proposal.status != ProposalStatus::Active {
        return Err(StdError::generic_err("Proposal is not active"));
    }

    if env.block.height <= proposal.end_height {
        return Err(StdError::generic_err("Voting period has not ended"));
    }

    let config = state::config_read(&deps.storage).load()?;
    let mars_contracts = vec![
        MarsContract::MarsToken,
        MarsContract::Staking,
        MarsContract::XMarsToken,
    ];
    let expected_len = mars_contracts.len();
    let mut addresses_query = address_provider::helpers::query_addresses(
        &deps,
        &config.address_provider_address,
        mars_contracts,
    )?;

    if addresses_query.len() != expected_len {
        return Err(StdError::generic_err(format!(
            "Incorrect number of addresses, expected {} got {}",
            expected_len,
            addresses_query.len()
        )));
    }
    let xmars_token_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting address from provider"))?;
    let staking_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting address from provider"))?;
    let mars_token_address = addresses_query
        .pop()
        .ok_or_else(|| StdError::generic_err("error while getting address from provider"))?;

    // Compute proposal quorum and threshold
    let for_votes = proposal.for_votes;
    let against_votes = proposal.against_votes;
    let total_votes = for_votes + against_votes;
    let total_voting_power = xmars_get_total_supply_at(
        &deps.querier,
        xmars_token_address,
        proposal.start_height - 1,
    )?;

    let mut proposal_quorum: Decimal = Decimal::zero();
    let mut proposal_threshold: Decimal = Decimal::zero();
    if total_voting_power > Uint128::zero() {
        proposal_quorum = Decimal::from_ratio(total_votes, total_voting_power);
    }
    if total_votes > Uint128::zero() {
        proposal_threshold = Decimal::from_ratio(for_votes, total_votes);
    }

    // Determine proposal result
    let (new_proposal_status, log_proposal_result, messages) = if proposal_quorum
        >= config.proposal_required_quorum
        && proposal_threshold >= config.proposal_required_threshold
    {
        // if quorum and threshold are met then proposal passes
        // refund deposit amount to submitter
        let msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: mars_token_address,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: deps
                    .api
                    .human_address(&proposal.submitter_canonical_address)?,
                amount: proposal.deposit_amount,
            })?,
        });

        (ProposalStatus::Passed, "passed", vec![msg])
    } else {
        // Else proposal is rejected
        let msg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: mars_token_address,
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: staking_address,
                amount: proposal.deposit_amount,
            })?,
            send: vec![],
        });

        (ProposalStatus::Rejected, "rejected", vec![msg])
    };

    // Update proposal status
    proposal.status = new_proposal_status;
    state::proposals(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

    Ok(HandleResponse {
        messages,
        log: vec![
            log("action", "end_proposal"),
            log("proposal_id", proposal_id),
            log("proposal_result", log_proposal_result),
        ],
        data: None,
    })
}

pub fn handle_execute_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    proposal_id: u64,
) -> StdResult<HandleResponse> {
    let mut proposal = state::proposals_read(&deps.storage).load(&proposal_id.to_be_bytes())?;

    if proposal.status != ProposalStatus::Passed {
        return Err(StdError::generic_err(
            "Proposal has not passed or has already been executed",
        ));
    }

    let config = state::config_read(&deps.storage).load()?;
    if env.block.height < (proposal.end_height + config.proposal_effective_delay) {
        return Err(StdError::generic_err(
            "Proposal has not ended its delay period",
        ));
    }
    if env.block.height
        > (proposal.end_height
            + config.proposal_effective_delay
            + config.proposal_expiration_period)
    {
        return Err(StdError::generic_err("Proposal has expired"));
    }

    proposal.status = ProposalStatus::Executed;
    state::proposals(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

    let messages: Vec<CosmosMsg> = if let Some(mut proposal_execute_calls) = proposal.execute_calls
    {
        let mut ret = Vec::<CosmosMsg>::with_capacity(proposal_execute_calls.len());

        proposal_execute_calls.sort_by(|a, b| a.execution_order.cmp(&b.execution_order));

        for execute_call in proposal_execute_calls {
            ret.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps
                    .api
                    .human_address(&execute_call.target_contract_canonical_address)?,
                msg: execute_call.msg,
                send: vec![],
            }));
        }

        ret
    } else {
        vec![]
    };

    Ok(HandleResponse {
        messages,
        log: vec![
            log("action", "execute_proposal"),
            log("proposal_id", proposal_id),
        ],
        data: None,
    })
}

/// Update config
pub fn handle_update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    new_config: CreateOrUpdateConfig,
) -> StdResult<HandleResponse> {
    let mut config = state::config_read(&deps.storage).load()?;

    // In council, config can be updated only by itself (through an approved proposal)
    // instead of by it's owner
    if env.message.sender != env.contract.address {
        return Err(StdError::unauthorized());
    }

    // Destructuring a struct’s fields into separate variables in order to force
    // compile error if we add more params
    let CreateOrUpdateConfig {
        address_provider_address,

        proposal_voting_period,
        proposal_effective_delay,
        proposal_expiration_period,
        proposal_required_deposit,
        proposal_required_quorum,
        proposal_required_threshold,
    } = new_config;

    // Update config
    config.address_provider_address = human_addr_into_canonical(
        deps.api,
        address_provider_address,
        config.address_provider_address,
    )?;

    config.proposal_voting_period = proposal_voting_period.unwrap_or(config.proposal_voting_period);
    config.proposal_effective_delay =
        proposal_effective_delay.unwrap_or(config.proposal_effective_delay);
    config.proposal_expiration_period =
        proposal_expiration_period.unwrap_or(config.proposal_expiration_period);
    config.proposal_required_deposit =
        proposal_required_deposit.unwrap_or(config.proposal_required_deposit);
    config.proposal_required_quorum =
        proposal_required_quorum.unwrap_or(config.proposal_required_quorum);
    config.proposal_required_threshold =
        proposal_required_threshold.unwrap_or(config.proposal_required_threshold);

    // Validate config
    config.validate()?;

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
        QueryMsg::Proposals { start, limit } => to_binary(&query_proposals(deps, start, limit)?),
        QueryMsg::Proposal { proposal_id } => to_binary(&query_proposal(deps, proposal_id)?),
        QueryMsg::LatestExecutedProposal {} => to_binary(&query_latest_executed_proposal(deps)?),
        QueryMsg::ProposalVotes {
            proposal_id,
            start_after,
            limit,
        } => to_binary(&query_proposal_votes(
            deps,
            proposal_id,
            start_after,
            limit,
        )?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = state::config_read(&deps.storage).load()?;

    let address_provider_address = deps
        .api
        .human_address(&config.address_provider_address)
        .unwrap_or_default();

    Ok(ConfigResponse {
        address_provider_address,

        proposal_voting_period: config.proposal_voting_period,
        proposal_effective_delay: config.proposal_effective_delay,
        proposal_expiration_period: config.proposal_expiration_period,
        proposal_required_deposit: config.proposal_required_deposit,
        proposal_required_quorum: config.proposal_required_quorum,
        proposal_required_threshold: config.proposal_required_threshold,
    })
}

fn query_proposals<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    start: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalsListResponse> {
    let council = state::council_read(&deps.storage).load().unwrap();
    let start = start.unwrap_or(DEFAULT_START).to_be_bytes();
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let proposals = state::proposals_read(&deps.storage);
    let proposals_list: StdResult<Vec<_>> = proposals
        .range(Option::from(&start[..]), None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (k, v) = item?;
            let proposal_id = read_be_u64(k.as_slice())?;

            Ok(ProposalInfo {
                proposal_id,
                submitter_address: deps.api.human_address(&v.submitter_canonical_address)?,
                status: v.status,
                for_votes: v.for_votes,
                against_votes: v.against_votes,
                start_height: v.start_height,
                end_height: v.end_height,
                title: v.title,
                description: v.description,
                link: v.link,
                execute_calls: v.execute_calls,
                deposit_amount: v.deposit_amount,
            })
        })
        .collect();

    Ok(ProposalsListResponse {
        proposal_count: council.proposal_count,
        proposal_list: proposals_list?,
    })
}

fn query_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    proposal_id: u64,
) -> StdResult<ProposalInfo> {
    let proposal = state::proposals_read(&deps.storage).load(&proposal_id.to_be_bytes())?;

    Ok(ProposalInfo {
        proposal_id,
        submitter_address: deps
            .api
            .human_address(&proposal.submitter_canonical_address)?,
        status: proposal.status,
        for_votes: proposal.for_votes,
        against_votes: proposal.against_votes,
        start_height: proposal.start_height,
        end_height: proposal.end_height,
        title: proposal.title,
        description: proposal.description,
        link: proposal.link,
        execute_calls: proposal.execute_calls,
        deposit_amount: proposal.deposit_amount,
    })
}

fn query_latest_executed_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ProposalInfo> {
    let latest_execute_proposal = state::proposals_read(&deps.storage)
        .range(None, None, Order::Descending)
        .take(MAX_LIMIT as usize)
        .find(|proposal| {
            let (_, v) = proposal.as_ref().unwrap();
            v.status == ProposalStatus::Executed
        });

    match latest_execute_proposal {
        Some(proposal) => {
            let (k, v) = proposal?;
            let proposal_id = read_be_u64(k.as_slice())?;

            Ok(ProposalInfo {
                proposal_id,
                submitter_address: deps.api.human_address(&v.submitter_canonical_address)?,
                status: v.status,
                for_votes: v.for_votes,
                against_votes: v.against_votes,
                start_height: v.start_height,
                end_height: v.end_height,
                title: v.title,
                description: v.description,
                link: v.link,
                execute_calls: v.execute_calls,
                deposit_amount: v.deposit_amount,
            })
        }
        None => Result::Err(StdError::not_found("No executed proposals found")),
    }
}

fn query_proposal_votes<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    proposal_id: u64,
    start_after: Option<HumanAddr>,
    limit: Option<u32>,
) -> StdResult<ProposalVotesResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let start = calc_range_start_human(deps.api, start_after)?;

    let votes: StdResult<Vec<ProposalVoteResponse>> =
        state::proposal_votes_read(&deps.storage, proposal_id)
            .range(start.as_deref(), None, Order::Ascending)
            .take(limit)
            .map(|vote| {
                let (k, v) = vote?;
                let voter_address = deps.api.human_address(&CanonicalAddr::from(k))?;

                Ok(ProposalVoteResponse {
                    voter_address,
                    option: v.option,
                    power: v.power,
                })
            })
            .collect();

    Ok(ProposalVotesResponse {
        proposal_id,
        votes: votes?,
    })
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
//
fn xmars_get_total_supply_at<Q: Querier>(
    querier: &Q,
    xmars_address: HumanAddr,
    block: u64,
) -> StdResult<Uint128> {
    let query: xmars_token::msg::TotalSupplyResponse =
        querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
            contract_addr: xmars_address,
            msg: to_binary(&xmars_token::msg::QueryMsg::TotalSupplyAt { block })?,
        }))?;

    Ok(query.total_supply)
}

fn xmars_get_balance_at<Q: Querier>(
    querier: &Q,
    xmars_address: HumanAddr,
    user_address: HumanAddr,
    block: u64,
) -> StdResult<Uint128> {
    let query: cw20::BalanceResponse = querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
        contract_addr: xmars_address,
        msg: to_binary(&xmars_token::msg::QueryMsg::BalanceAt {
            address: user_address,
            block,
        })?,
    }))?;

    Ok(query.balance)
}

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::Coin;
    use mars::testing::{
        assert_generic_error_message, get_test_addresses, mock_dependencies, mock_env,
        MarsMockQuerier, MockEnvParams,
    };

    use crate::msg::HandleMsg::UpdateConfig;

    const TEST_PROPOSAL_VOTING_PERIOD: u64 = 2000;
    const TEST_PROPOSAL_EFFECTIVE_DELAY: u64 = 200;
    const TEST_PROPOSAL_EXPIRATION_PERIOD: u64 = 300;
    const TEST_PROPOSAL_REQUIRED_DEPOSIT: Uint128 = Uint128(10000);

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with empty params
        // *
        let empty_config = CreateOrUpdateConfig {
            address_provider_address: None,

            proposal_voting_period: None,
            proposal_effective_delay: None,
            proposal_expiration_period: None,
            proposal_required_deposit: None,
            proposal_required_threshold: None,
            proposal_required_quorum: None,
        };
        let msg = InitMsg {
            config: empty_config,
        };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = init(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "All params should be available during initialization",
        );

        // *
        // init with proposal_required_quorum, proposal_required_threshold greater than 1
        // *
        let config = CreateOrUpdateConfig {
            address_provider_address: Some(HumanAddr::from("address_provider")),

            proposal_voting_period: Some(1),
            proposal_effective_delay: Some(1),
            proposal_expiration_period: Some(1),
            proposal_required_deposit: Some(Uint128(1)),
            proposal_required_quorum: Some(Decimal::from_ratio(11u128, 10u128)),
            proposal_required_threshold: Some(Decimal::from_ratio(11u128, 10u128)),
        };
        let msg = InitMsg { config };
        let env = cosmwasm_std::testing::mock_env("owner", &[]);
        let response = init(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "[proposal_required_quorum, proposal_required_threshold] should be less or equal 1. \
                Invalid params: [proposal_required_quorum, proposal_required_threshold]",
        );

        let config = CreateOrUpdateConfig {
            address_provider_address: Some(HumanAddr::from("address_provider")),

            proposal_voting_period: Some(1),
            proposal_effective_delay: Some(1),
            proposal_expiration_period: Some(1),
            proposal_required_deposit: Some(Uint128(1)),
            proposal_required_threshold: Some(Decimal::one()),
            proposal_required_quorum: Some(Decimal::one()),
        };
        let msg = InitMsg { config };
        let env = mock_env("owner", MockEnvParams::default());

        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        let config = state::config_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("address_provider"))
                .unwrap(),
            config.address_provider_address
        );

        let council = state::council_read(&deps.storage).load().unwrap();
        assert_eq!(council.proposal_count, 0);
    }

    #[test]
    fn test_update_config() {
        let mut deps = mock_dependencies(20, &[]);

        // *
        // init config with valid params
        // *
        let init_config = CreateOrUpdateConfig {
            address_provider_address: Some(HumanAddr::from("address_provider")),

            proposal_voting_period: Some(10),
            proposal_effective_delay: Some(11),
            proposal_expiration_period: Some(12),
            proposal_required_deposit: Some(Uint128(111)),
            proposal_required_threshold: Some(Decimal::one()),
            proposal_required_quorum: Some(Decimal::one()),
        };
        let msg = InitMsg {
            config: init_config.clone(),
        };
        let env = cosmwasm_std::testing::mock_env(MOCK_CONTRACT_ADDR, &[]);
        let _res = init(&mut deps, env, msg).unwrap();

        // *
        // update config with proposal_required_quorum, proposal_required_threshold greater than 1
        // *
        let config = CreateOrUpdateConfig {
            proposal_required_quorum: Some(Decimal::from_ratio(11u128, 10u128)),
            proposal_required_threshold: Some(Decimal::from_ratio(11u128, 10u128)),
            ..init_config.clone()
        };
        let msg = UpdateConfig { config };
        let env = cosmwasm_std::testing::mock_env(MOCK_CONTRACT_ADDR, &[]);
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "[proposal_required_quorum, proposal_required_threshold] should be less or equal 1. \
                Invalid params: [proposal_required_quorum, proposal_required_threshold]",
        );

        // *
        // only council itself is authorized
        // *
        let msg = UpdateConfig {
            config: init_config,
        };
        let env = cosmwasm_std::testing::mock_env("somebody", &[]);
        let error_res = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(error_res, StdError::unauthorized());

        // *
        // update config with all new params
        // *
        let config = CreateOrUpdateConfig {
            address_provider_address: Some(HumanAddr::from("new_address_provider")),

            proposal_voting_period: Some(101),
            proposal_effective_delay: Some(111),
            proposal_expiration_period: Some(121),
            proposal_required_deposit: Some(Uint128(1111)),
            proposal_required_threshold: Some(Decimal::from_ratio(4u128, 5u128)),
            proposal_required_quorum: Some(Decimal::from_ratio(1u128, 5u128)),
        };
        let msg = UpdateConfig {
            config: config.clone(),
        };
        // sender as contract address
        let env = cosmwasm_std::testing::mock_env(MOCK_CONTRACT_ADDR, &[]);
        // we can just call .unwrap() to assert this was a success
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Read config from state
        let new_config = state::config_read(&deps.storage).load().unwrap();

        assert_eq!(
            new_config.address_provider_address,
            deps.api
                .canonical_address(&HumanAddr::from("new_address_provider"))
                .unwrap()
        );
        assert_eq!(
            new_config.proposal_voting_period,
            config.proposal_voting_period.unwrap()
        );
        assert_eq!(
            new_config.proposal_effective_delay,
            config.proposal_effective_delay.unwrap()
        );
        assert_eq!(
            new_config.proposal_expiration_period,
            config.proposal_expiration_period.unwrap()
        );
        assert_eq!(
            new_config.proposal_required_deposit,
            config.proposal_required_deposit.unwrap()
        );
        assert_eq!(
            new_config.proposal_required_threshold,
            config.proposal_required_threshold.unwrap()
        );
        assert_eq!(
            new_config.proposal_required_quorum,
            config.proposal_required_quorum.unwrap()
        );
    }

    #[test]
    fn test_submit_proposal_invalid_params() {
        let mut deps = th_setup(&[]);

        // Invalid title
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "a".to_string(),
                    description: "A valid description".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Title too short");

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: (0..100).map(|_| "a").collect::<String>(),
                    description: "A valid description".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Title too long");

        // Invalid description
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: "a".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Description too short");

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: (0..1030).map(|_| "a").collect::<String>(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Description too long");

        // Invalid link
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: "A valid description".to_string(),
                    link: Some("a".to_string()),
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Link too short");

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: "A valid description".to_string(),
                    link: Some((0..150).map(|_| "a").collect::<String>()),
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: Uint128(2_000_000),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Link too long");

        // Invalid deposit amount
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: "A valid description".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: (TEST_PROPOSAL_REQUIRED_DEPOSIT - Uint128(100)).unwrap(),
        });
        let env = mock_env("mars_token", MockEnvParams::default());
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Must deposit at least 10000 tokens");

        // Invalid deposit currency
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid Title".to_string(),
                    description: "A valid description".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: HumanAddr::from("submitter"),
            amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
        });
        let env = mock_env("someothertoken", MockEnvParams::default());
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(res_error, StdError::unauthorized());
    }

    #[test]
    fn test_submit_proposal() {
        let mut deps = th_setup(&[]);
        let submitter_address = HumanAddr::from("submitter");
        let submitter_canonical_address = deps.api.canonical_address(&submitter_address).unwrap();

        // Submit Proposal without link or call data
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid title".to_string(),
                    description: "A valid description".to_string(),
                    link: None,
                    execute_calls: None,
                })
                .unwrap(),
            ),
            sender: submitter_address.clone(),
            amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
        });
        let env = mock_env(
            "mars_token",
            MockEnvParams {
                block_height: 100_000,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();
        let expected_end_height = 100_000 + TEST_PROPOSAL_VOTING_PERIOD;
        assert_eq!(
            res.log,
            vec![
                log("action", "submit_proposal"),
                log("proposal_submitter", "submitter"),
                log("proposal_id", 1),
                log("proposal_end_height", expected_end_height),
            ]
        );

        let council = state::council_read(&deps.storage).load().unwrap();
        assert_eq!(council.proposal_count, 1);

        let proposal = state::proposals_read(&deps.storage)
            .load(&1_u64.to_be_bytes())
            .unwrap();
        assert_eq!(
            proposal.submitter_canonical_address,
            submitter_canonical_address
        );
        assert_eq!(proposal.status, ProposalStatus::Active);
        assert_eq!(proposal.for_votes, Uint128(0));
        assert_eq!(proposal.against_votes, Uint128(0));
        assert_eq!(proposal.start_height, 100_000);
        assert_eq!(proposal.end_height, expected_end_height);
        assert_eq!(proposal.title, "A valid title");
        assert_eq!(proposal.description, "A valid description");
        assert_eq!(proposal.link, None);
        assert_eq!(proposal.execute_calls, None);
        assert_eq!(proposal.deposit_amount, TEST_PROPOSAL_REQUIRED_DEPOSIT);

        // Submit Proposal with link and call data
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(
                to_binary(&ReceiveMsg::SubmitProposal {
                    title: "A valid title".to_string(),
                    description: "A valid description".to_string(),
                    link: Some("https://www.avalidlink.com".to_string()),
                    execute_calls: Some(vec![MsgExecuteCall {
                        execution_order: 0,
                        target_contract_address: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        msg: to_binary(&HandleMsg::UpdateConfig {
                            config: CreateOrUpdateConfig::default(),
                        })
                        .unwrap(),
                    }]),
                })
                .unwrap(),
            ),
            sender: submitter_address,
            amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
        });
        let env = mock_env(
            "mars_token",
            MockEnvParams {
                block_height: 100_000,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();
        let expected_end_height = 100_000 + TEST_PROPOSAL_VOTING_PERIOD;
        assert_eq!(
            res.log,
            vec![
                log("action", "submit_proposal"),
                log("proposal_submitter", "submitter"),
                log("proposal_id", 2),
                log("proposal_end_height", expected_end_height),
            ]
        );

        let council = state::council_read(&deps.storage).load().unwrap();
        assert_eq!(council.proposal_count, 2);

        let proposal = state::proposals_read(&deps.storage)
            .load(&2_u64.to_be_bytes())
            .unwrap();
        assert_eq!(
            proposal.link,
            Some("https://www.avalidlink.com".to_string())
        );
        assert_eq!(
            proposal.execute_calls,
            Some(vec![ProposalExecuteCall {
                execution_order: 0,
                target_contract_canonical_address: deps
                    .api
                    .canonical_address(&HumanAddr::from(MOCK_CONTRACT_ADDR))
                    .unwrap(),
                msg: to_binary(&HandleMsg::UpdateConfig {
                    config: CreateOrUpdateConfig::default()
                })
                .unwrap(),
            }])
        );
    }

    #[test]
    fn test_invalid_cast_votes() {
        let mut deps = th_setup(&[]);
        let (voter_address, _voter_canonical_address) =
            get_test_addresses(&deps.api, "valid_voter");
        let (invalid_voter_address, _invalid_voter_canonical_address) =
            get_test_addresses(&deps.api, "invalid_voter");

        deps.querier
            .set_xmars_address(HumanAddr::from("xmars_token"));
        deps.querier
            .set_xmars_balance_at(voter_address, 99_999, Uint128(100));
        deps.querier
            .set_xmars_balance_at(invalid_voter_address, 99_999, Uint128::zero());

        let active_proposal_id = 1_u64;
        let active_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_id.to_be_bytes(), &active_proposal)
            .unwrap();

        let executed_proposal_id = 2_u64;
        let executed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: executed_proposal_id,
                status: ProposalStatus::Executed,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&executed_proposal_id.to_be_bytes(), &executed_proposal)
            .unwrap();

        // voting on a non-existent proposal should fail
        let msg = HandleMsg::CastVote {
            proposal_id: 3,
            vote: ProposalVoteOption::For,
        };
        let env = mock_env(
            "valid_voter",
            MockEnvParams {
                block_height: 100_001,
                ..Default::default()
            },
        );
        let res_error = handle(&mut deps, env, msg).unwrap_err();
        assert_eq!(
            res_error,
            StdError::NotFound {
                kind: "council::state::Proposal".to_string(),
                backtrace: None
            }
        );

        // voting on an inactive proposal should fail
        let msg = HandleMsg::CastVote {
            proposal_id: executed_proposal_id,
            vote: ProposalVoteOption::For,
        };
        let env = mock_env(
            "valid_voter",
            MockEnvParams {
                block_height: 100_001,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Proposal is not active");

        // voting after proposal end should fail
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };
        let env = mock_env(
            "valid_voter",
            MockEnvParams {
                block_height: 100_200,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Proposal has expired");

        // voting without any voting power should fail
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };
        let env = mock_env(
            "invalid_voter",
            MockEnvParams {
                block_height: 100_001,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "User has no balance at block: 99999");
    }

    #[test]
    fn test_cast_vote() {
        // setup
        let mut deps = th_setup(&[]);
        let (voter_address, voter_canonical_address) = get_test_addresses(&deps.api, "voter");

        let active_proposal_id = 1_u64;

        deps.querier
            .set_xmars_address(HumanAddr::from("xmars_token"));
        deps.querier
            .set_xmars_balance_at(voter_address, 99_999, Uint128(100));

        let active_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_id.to_be_bytes(), &active_proposal)
            .unwrap();

        // Add another vote on an extra proposal to voter to validate voting on multiple proposals
        // is valid
        state::proposal_votes(&mut deps.storage, 4_u64)
            .save(
                voter_canonical_address.as_slice(),
                &ProposalVote {
                    option: ProposalVoteOption::Against,
                    power: Uint128(100),
                },
            )
            .unwrap();

        // Valid vote for
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };

        let env = mock_env(
            "voter",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![
                log("action", "cast_vote"),
                log("proposal_id", active_proposal_id),
                log("voter", "voter"),
                log("vote", "for"),
                log("voting_power", 100),
            ],
            res.log
        );

        let proposal = state::proposals_read(&deps.storage)
            .load(&active_proposal_id.to_be_bytes())
            .unwrap();
        assert_eq!(proposal.for_votes, Uint128(100));
        assert_eq!(proposal.against_votes, Uint128(0));

        let proposal_vote = state::proposal_votes_read(&deps.storage, active_proposal_id)
            .load(voter_canonical_address.as_slice())
            .unwrap();

        assert_eq!(proposal_vote.option, ProposalVoteOption::For);
        assert_eq!(proposal_vote.power, Uint128(100));

        // Voting again with same address should fail
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };

        let env = mock_env(
            "voter",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "User has already voted in this proposal");

        // Valid against vote
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::Against,
        };

        deps.querier.set_xmars_balance_at(
            HumanAddr::from("voter2"),
            active_proposal.start_height - 1,
            Uint128(200),
        );

        let env = mock_env(
            "voter2",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                log("action", "cast_vote"),
                log("proposal_id", active_proposal_id),
                log("voter", "voter2"),
                log("vote", "against"),
                log("voting_power", 200),
            ],
            res.log
        );

        // Extra for and against votes to check aggregates are computed correctly
        deps.querier.set_xmars_balance_at(
            HumanAddr::from("voter3"),
            active_proposal.start_height - 1,
            Uint128(300),
        );

        deps.querier.set_xmars_balance_at(
            HumanAddr::from("voter4"),
            active_proposal.start_height - 1,
            Uint128(400),
        );

        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };
        let env = mock_env(
            "voter3",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg).unwrap();

        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::Against,
        };
        let env = mock_env(
            "voter4",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg).unwrap();

        let proposal = state::proposals_read(&deps.storage)
            .load(&active_proposal_id.to_be_bytes())
            .unwrap();
        assert_eq!(proposal.for_votes, Uint128(100 + 300));
        assert_eq!(proposal.against_votes, Uint128(200 + 400));
    }

    #[test]
    fn test_query_proposals() {
        // Arrange
        let mut deps = th_setup(&[]);

        let active_proposal_1_id = 1_u64;
        let active_proposal_1 = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_1_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_1_id.to_be_bytes(), &active_proposal_1)
            .unwrap();

        let active_proposal_2_id = 2_u64;
        let active_proposal_2 = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_2_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_2_id.to_be_bytes(), &active_proposal_2)
            .unwrap();

        let council = Council {
            proposal_count: 2_u64,
        };
        state::council(&mut deps.storage).save(&council).unwrap();

        // Assert corectly sorts asc
        let res = query_proposals(&deps, Option::None, Option::None).unwrap();
        assert_eq!(res.proposal_count, 2);
        assert_eq!(res.proposal_list.len(), 2);
        assert_eq!(res.proposal_list[0].proposal_id, active_proposal_1_id);
        assert_eq!(res.proposal_list[1].proposal_id, active_proposal_2_id);

        // Assert start != 0
        let res = query_proposals(&deps, Option::from(2), Option::None).unwrap();
        assert_eq!(res.proposal_count, 2);
        assert_eq!(res.proposal_list.len(), 1);
        assert_eq!(res.proposal_list[0].proposal_id, active_proposal_2_id);

        // Assert start > length of collection
        let res = query_proposals(&deps, Option::from(99), Option::None).unwrap();
        assert_eq!(res.proposal_count, 2);
        assert_eq!(res.proposal_list.len(), 0);

        // Assert limit
        let res = query_proposals(&deps, Option::None, Option::from(1)).unwrap();
        assert_eq!(res.proposal_count, 2);
        assert_eq!(res.proposal_list.len(), 1);
        assert_eq!(res.proposal_list[0].proposal_id, active_proposal_1_id);

        // Assert limit greater than length of collection
        let res = query_proposals(&deps, Option::None, Option::from(99)).unwrap();
        assert_eq!(res.proposal_count, 2);
        assert_eq!(res.proposal_list.len(), 2);
    }

    #[test]
    fn test_query_latest_executed_proposal() {
        // Arrange
        let mut deps = th_setup(&[]);

        let active_proposal_1_id = 1_u64;
        let active_proposal_1 = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_1_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_1_id.to_be_bytes(), &active_proposal_1)
            .unwrap();

        let council = Council {
            proposal_count: 2_u64,
        };
        state::council(&mut deps.storage).save(&council).unwrap();

        // Assert not found error when no executed proposals
        let res = query_latest_executed_proposal(&deps).unwrap_err();
        assert_eq!(res, StdError::not_found("No executed proposals found"));

        // Arrange
        let active_proposal_2_id = 2_u64;
        let active_proposal_2 = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_2_id,
                status: ProposalStatus::Executed,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_2_id.to_be_bytes(), &active_proposal_2)
            .unwrap();

        let active_proposal_3_id = 3_u64;
        let active_proposal_3 = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_3_id,
                status: ProposalStatus::Executed,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_3_id.to_be_bytes(), &active_proposal_3)
            .unwrap();

        // Assert that the latest proposal is returned when multiple proposals executed
        let res = query_latest_executed_proposal(&deps).unwrap();
        assert_eq!(res.proposal_id, active_proposal_3_id);
        assert_eq!(res.status, ProposalStatus::Executed);
    }

    #[test]
    fn test_query_proposal_votes() {
        // Arrange
        let mut deps = th_setup(&[]);

        deps.querier
            .set_xmars_address(HumanAddr::from("xmars_token"));
        let active_proposal_id = 1_u64;

        let (voter_address1, _voter_canonical_address1) = get_test_addresses(&deps.api, "voter1");
        let (voter_address2, _voter_canonical_address2) = get_test_addresses(&deps.api, "voter2");
        let (voter_address3, _voter_canonical_address3) = get_test_addresses(&deps.api, "voter3");
        let (voter_address4, _voter_canonical_address4) = get_test_addresses(&deps.api, "voter4");
        let (voter_address5, _voter_canonical_address5) = get_test_addresses(&deps.api, "voter5");
        deps.querier
            .set_xmars_balance_at(voter_address1, 99_999, Uint128(100));
        deps.querier
            .set_xmars_balance_at(voter_address2, 99_999, Uint128(200));
        deps.querier
            .set_xmars_balance_at(voter_address3, 99_999, Uint128(300));
        deps.querier
            .set_xmars_balance_at(voter_address4, 99_999, Uint128(400));
        deps.querier
            .set_xmars_balance_at(voter_address5, 99_999, Uint128(500));

        let active_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        state::proposals(&mut deps.storage)
            .save(&active_proposal_id.to_be_bytes(), &active_proposal)
            .unwrap();

        let msg_vote_for = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
        };
        let msg_vote_against = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::Against,
        };

        // Act
        let env = mock_env(
            "voter1",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg_vote_for.clone()).unwrap();

        let env = mock_env(
            "voter2",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg_vote_for.clone()).unwrap();

        let env = mock_env(
            "voter3",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg_vote_for).unwrap();

        let env = mock_env(
            "voter4",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg_vote_against.clone()).unwrap();

        let env = mock_env(
            "voter5",
            MockEnvParams {
                block_height: active_proposal.start_height + 1,
                ..Default::default()
            },
        );
        handle(&mut deps, env, msg_vote_against).unwrap();

        // Assert default params
        let res =
            query_proposal_votes(&deps, active_proposal_id, Option::None, Option::None).unwrap();
        assert_eq!(res.votes.len(), 5);
        assert_eq!(res.proposal_id, active_proposal_id);

        // Assert corectly sorts asc
        assert_eq!(res.votes[0].voter_address, HumanAddr::from("voter1"));
        assert_eq!(res.votes[0].option, ProposalVoteOption::For);
        assert_eq!(res.votes[0].power, Uint128(100));
        assert_eq!(res.votes[4].voter_address, HumanAddr::from("voter5"));
        assert_eq!(res.votes[4].option, ProposalVoteOption::Against);
        assert_eq!(res.votes[4].power, Uint128(500));

        // Assert start_after
        let res = query_proposal_votes(
            &deps,
            active_proposal_id,
            Option::from(HumanAddr::from("voter4")),
            Option::None,
        )
        .unwrap();
        assert_eq!(res.votes.len(), 1);
        assert_eq!(res.votes[0].voter_address, HumanAddr::from("voter5"));

        // Assert take
        let res =
            query_proposal_votes(&deps, active_proposal_id, Option::None, Option::from(1)).unwrap();
        assert_eq!(res.votes.len(), 1);
        assert_eq!(res.votes[0].voter_address, HumanAddr::from("voter1"));
    }

    #[test]
    fn test_invalid_end_proposals() {
        let mut deps = th_setup(&[]);

        let active_proposal_id = 1_u64;
        let executed_proposal_id = 2_u64;

        deps.querier
            .set_xmars_address(HumanAddr::from("xmars_token"));
        deps.querier.set_xmars_total_supply_at(99_999, Uint128(100));

        th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Active,
                end_height: 100_000,
                ..Default::default()
            },
        );
        th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: executed_proposal_id,
                status: ProposalStatus::Executed,
                ..Default::default()
            },
        );

        // cannot end a proposal that has not ended its voting period
        let msg = HandleMsg::EndProposal {
            proposal_id: active_proposal_id,
        };
        let env = mock_env(
            "ender",
            MockEnvParams {
                block_height: 100_000,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Voting period has not ended");

        // cannot end a non active proposal
        let msg = HandleMsg::EndProposal {
            proposal_id: executed_proposal_id,
        };
        let env = mock_env(
            "ender",
            MockEnvParams {
                block_height: 100_001,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Proposal is not active");
    }

    #[test]
    fn test_end_proposal() {
        let mut deps = th_setup(&[]);

        deps.querier
            .set_xmars_address(HumanAddr::from("xmars_token"));
        deps.querier
            .set_xmars_total_supply_at(89_999, Uint128(100_000));
        let proposal_threshold = Decimal::from_ratio(51_u128, 100_u128);
        let proposal_quorum = Decimal::from_ratio(2_u128, 100_u128);
        let proposal_end_height = 100_000u64;

        state::config(&mut deps.storage)
            .update(|mut config| {
                config.proposal_required_threshold = proposal_threshold;
                config.proposal_required_quorum = proposal_quorum;
                Ok(config)
            })
            .unwrap();

        // end passed proposal
        let initial_passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 1,
                status: ProposalStatus::Active,
                for_votes: Uint128(11_000),
                against_votes: Uint128(10_000),
                start_height: 90_000,
                end_height: proposal_end_height + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 1 };

        let env = mock_env(
            "sender",
            MockEnvParams {
                block_height: initial_passed_proposal.end_height + 1,
                ..Default::default()
            },
        );

        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "end_proposal"),
                log("proposal_id", 1),
                log("proposal_result", "passed"),
            ]
        );

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: HumanAddr::from("submitter"),
                    amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
                })
                .unwrap(),
            }),]
        );

        let final_passed_proposal = state::proposals_read(&deps.storage)
            .load(&1u64.to_be_bytes())
            .unwrap();
        assert_eq!(final_passed_proposal.status, ProposalStatus::Passed);

        // end rejected proposal (no quorum)
        let initial_passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 2,
                status: ProposalStatus::Active,
                for_votes: Uint128(11),
                against_votes: Uint128(10),
                end_height: proposal_end_height + 1,
                start_height: 90_000,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 2 };

        let env = mock_env(
            "sender",
            MockEnvParams {
                block_height: initial_passed_proposal.end_height + 1,
                ..Default::default()
            },
        );

        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "end_proposal"),
                log("proposal_id", 2),
                log("proposal_result", "rejected"),
            ]
        );

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: HumanAddr::from("staking"),
                    amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
                })
                .unwrap(),
                send: vec![],
            })]
        );

        let final_passed_proposal = state::proposals_read(&deps.storage)
            .load(&2u64.to_be_bytes())
            .unwrap();
        assert_eq!(final_passed_proposal.status, ProposalStatus::Rejected);

        // end rejected proposal (no threshold)
        let initial_passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 3,
                status: ProposalStatus::Active,
                for_votes: Uint128(10_000),
                against_votes: Uint128(11_000),
                start_height: 90_000,
                end_height: proposal_end_height + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 3 };

        let env = mock_env(
            "sender",
            MockEnvParams {
                block_height: initial_passed_proposal.end_height + 1,
                ..Default::default()
            },
        );

        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![
                log("action", "end_proposal"),
                log("proposal_id", 3),
                log("proposal_result", "rejected"),
            ]
        );

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: HumanAddr::from("staking"),
                    amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
                })
                .unwrap(),
                send: vec![],
            })]
        );

        let final_passed_proposal = state::proposals_read(&deps.storage)
            .load(&3u64.to_be_bytes())
            .unwrap();
        assert_eq!(final_passed_proposal.status, ProposalStatus::Rejected);
    }

    #[test]
    fn test_invalid_execute_proposals() {
        let mut deps = th_setup(&[]);

        let passed_proposal_id = 1_u64;
        let executed_proposal_id = 2_u64;

        let passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: passed_proposal_id,
                status: ProposalStatus::Passed,
                end_height: 100_000,
                ..Default::default()
            },
        );
        let executed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: executed_proposal_id,
                status: ProposalStatus::Executed,
                ..Default::default()
            },
        );

        // cannot execute a non Passed proposal
        let msg = HandleMsg::ExecuteProposal {
            proposal_id: executed_proposal_id,
        };
        let env = mock_env(
            "executer",
            MockEnvParams {
                block_height: executed_proposal.end_height + TEST_PROPOSAL_EFFECTIVE_DELAY + 1,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(
            response,
            "Proposal has not passed or has already been executed",
        );

        // cannot execute a proposal before the effective delay has passed
        let msg = HandleMsg::ExecuteProposal {
            proposal_id: passed_proposal_id,
        };
        let env = mock_env(
            "executer",
            MockEnvParams {
                block_height: passed_proposal.end_height + 1,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Proposal has not ended its delay period");

        // cannot execute an expired proposal
        let msg = HandleMsg::ExecuteProposal {
            proposal_id: passed_proposal_id,
        };
        let env = mock_env(
            "executer",
            MockEnvParams {
                block_height: passed_proposal.end_height
                    + TEST_PROPOSAL_EFFECTIVE_DELAY
                    + TEST_PROPOSAL_EXPIRATION_PERIOD
                    + 1,
                ..Default::default()
            },
        );
        let response = handle(&mut deps, env, msg);
        assert_generic_error_message(response, "Proposal has expired");
    }

    #[test]
    fn test_execute_proposals() {
        let mut deps = th_setup(&[]);
        let (contract_address, contract_canonical_address) =
            get_test_addresses(&deps.api, MOCK_CONTRACT_ADDR);
        let (other_address, other_canonical_address) = get_test_addresses(&deps.api, "other");

        let binary_msg = Binary::from(br#"{"key": 123}"#);
        let initial_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 1,
                status: ProposalStatus::Passed,
                end_height: 100_000,
                execute_calls: Some(vec![
                    ProposalExecuteCall {
                        execution_order: 2,
                        msg: binary_msg.clone(),
                        target_contract_canonical_address: other_canonical_address,
                    },
                    ProposalExecuteCall {
                        execution_order: 3,
                        msg: to_binary(&HandleMsg::UpdateConfig {
                            config: CreateOrUpdateConfig::default(),
                        })
                        .unwrap(),
                        target_contract_canonical_address: contract_canonical_address.clone(),
                    },
                    ProposalExecuteCall {
                        execution_order: 1,
                        msg: to_binary(&HandleMsg::UpdateConfig {
                            config: CreateOrUpdateConfig::default(),
                        })
                        .unwrap(),
                        target_contract_canonical_address: contract_canonical_address,
                    },
                ]),
                ..Default::default()
            },
        );

        let env = mock_env(
            "executer",
            MockEnvParams {
                block_height: initial_proposal.end_height + TEST_PROPOSAL_EFFECTIVE_DELAY + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::ExecuteProposal { proposal_id: 1 };

        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            res.log,
            vec![log("action", "execute_proposal"), log("proposal_id", 1),]
        );

        assert_eq!(
            res.messages,
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract_address.clone(),
                    send: vec![],
                    msg: to_binary(&HandleMsg::UpdateConfig {
                        config: CreateOrUpdateConfig::default()
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: other_address,
                    send: vec![],
                    msg: binary_msg,
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract_address,
                    send: vec![],
                    msg: to_binary(&HandleMsg::UpdateConfig {
                        config: CreateOrUpdateConfig::default()
                    })
                    .unwrap(),
                }),
            ]
        );

        let final_proposal = state::proposals_read(&deps.storage)
            .load(&1_u64.to_be_bytes())
            .unwrap();

        assert_eq!(ProposalStatus::Executed, final_proposal.status);
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let config = CreateOrUpdateConfig {
            address_provider_address: Some(HumanAddr::from("address_provider")),

            proposal_voting_period: Some(TEST_PROPOSAL_VOTING_PERIOD),
            proposal_effective_delay: Some(TEST_PROPOSAL_EFFECTIVE_DELAY),
            proposal_expiration_period: Some(TEST_PROPOSAL_EXPIRATION_PERIOD),
            proposal_required_deposit: Some(TEST_PROPOSAL_REQUIRED_DEPOSIT),
            proposal_required_quorum: Some(Decimal::one()),
            proposal_required_threshold: Some(Decimal::one()),
        };
        let msg = InitMsg { config };
        let env = mock_env("owner", MockEnvParams::default());
        init(&mut deps, env, msg).unwrap();

        deps
    }

    #[derive(Debug)]
    struct MockProposal {
        id: u64,
        status: ProposalStatus,
        for_votes: Uint128,
        against_votes: Uint128,
        start_height: u64,
        end_height: u64,
        execute_calls: Option<Vec<ProposalExecuteCall>>,
    }

    impl Default for MockProposal {
        fn default() -> Self {
            MockProposal {
                id: 1,
                status: ProposalStatus::Active,
                for_votes: Uint128::zero(),
                against_votes: Uint128::zero(),
                start_height: 1,
                end_height: 1,
                execute_calls: None,
            }
        }
    }

    fn th_build_mock_proposal(
        deps: &mut Extern<MockStorage, MockApi, MarsMockQuerier>,
        mock_proposal: MockProposal,
    ) -> Proposal {
        let (_, canonical_address) = get_test_addresses(&deps.api, "submitter");

        let proposal = Proposal {
            submitter_canonical_address: canonical_address,
            status: mock_proposal.status,
            for_votes: mock_proposal.for_votes,
            against_votes: mock_proposal.against_votes,
            start_height: mock_proposal.start_height,
            end_height: mock_proposal.end_height,
            title: "A valid title".to_string(),
            description: "A description".to_string(),
            link: None,
            execute_calls: mock_proposal.execute_calls,
            deposit_amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
        };

        state::proposals(&mut deps.storage)
            .save(&mock_proposal.id.to_be_bytes(), &proposal)
            .unwrap();

        proposal
    }
}
