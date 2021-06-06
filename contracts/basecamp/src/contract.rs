use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Decimal, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Order, Querier,
    QueryRequest, StdError, StdResult, Storage, Uint128, WasmMsg, WasmQuery,
};

use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::cw20_token;
use mars::helpers::read_be_u64;
use mars::xmars_token;

use crate::msg::{
    ConfigResponse, HandleMsg, InitMsg, MigrateMsg, MsgExecuteCall, ProposalInfo,
    ProposalsListResponse, QueryMsg, ReceiveMsg,
};
use crate::state::{
    basecamp_state, basecamp_state_read, config_state, config_state_read, proposal_votes_state,
    proposal_votes_state_read, proposals_state, proposals_state_read, Basecamp, Config, Proposal,
    ProposalExecuteCall, ProposalStatus, ProposalVote, ProposalVoteOption,
};

// CONSTANTS
const MIN_TITLE_LENGTH: usize = 4;
const MAX_TITLE_LENGTH: usize = 64;
const MIN_DESC_LENGTH: usize = 4;
const MAX_DESC_LENGTH: usize = 1024;
const MIN_LINK_LENGTH: usize = 12;
const MAX_LINK_LENGTH: usize = 128;

// INIT

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // initialize Config
    let config = Config {
        owner: deps.api.canonical_address(&env.message.sender)?,
        mars_token_address: CanonicalAddr::default(),
        xmars_token_address: deps.api.canonical_address(&msg.xmars_token_address)?,
        staking_contract_address: deps.api.canonical_address(&msg.staking_contract_address)?,

        proposal_voting_period: msg.proposal_voting_period,
        proposal_effective_delay: msg.proposal_effective_delay,
        proposal_expiration_period: msg.proposal_expiration_period,
        proposal_required_deposit: msg.proposal_required_deposit,
        proposal_required_quorum: msg.proposal_required_quorum,
        proposal_required_threshold: msg.proposal_required_threshold,
    };

    config_state(&mut deps.storage).save(&config)?;

    // initialize State
    basecamp_state(&mut deps.storage).save(&Basecamp { proposal_count: 0 })?;

    // Prepare response, should instantiate Mars and use the Register hook
    Ok(InitResponse {
        log: vec![],
        // TODO: Mars token is initialized here. Evaluate doing this outside of the contract
        messages: vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
            code_id: msg.cw20_code_id,
            msg: to_binary(&cw20_token::msg::InitMsg {
                name: "Mars token".to_string(),
                symbol: "Mars".to_string(),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: HumanAddr::from(env.contract.address.as_str()),
                    cap: None,
                }),
                init_hook: Some(cw20_token::msg::InitHook {
                    msg: to_binary(&HandleMsg::InitTokenCallback {})?,
                    contract_addr: env.contract.address,
                }),
            })?,
            send: vec![],
            label: None,
        })],
    })
}

// HANDLERS

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Receive(cw20_msg) => handle_receive_cw20(deps, env, cw20_msg),
        HandleMsg::InitTokenCallback {} => handle_init_mars_callback(deps, env),

        HandleMsg::CastVote { proposal_id, vote } => handle_cast_vote(deps, env, proposal_id, vote),

        HandleMsg::EndProposal { proposal_id } => handle_end_proposal(deps, env, proposal_id),
        HandleMsg::ExecuteProposal { proposal_id } => {
            handle_execute_proposal(deps, env, proposal_id)
        }

        HandleMsg::UpdateConfig {} => Ok(HandleResponse::default()), //TODO

        HandleMsg::MintMars { recipient, amount } => handle_mint_mars(deps, env, recipient, amount),
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

/// Submit new proposal

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

    let config = config_state_read(&deps.storage).load()?;

    if env.message.sender != deps.api.human_address(&config.mars_token_address)? {
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
    let mut basecamp_singleton = basecamp_state(&mut deps.storage);
    let mut basecamp = basecamp_singleton.load()?;
    basecamp.proposal_count += 1;
    basecamp_singleton.save(&basecamp)?;

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
    proposals_state(&mut deps.storage)
        .save(&basecamp.proposal_count.to_be_bytes(), &new_proposal)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![
            log("action", "submit_proposal"),
            log("proposal_submitter", &submitter_address),
            log("proposal_id", &basecamp.proposal_count),
            log("proposal_end_height", &new_proposal.end_height),
        ],
        data: None,
    })
}

/// Handles Mars post-initialization storing the address in config
pub fn handle_init_mars_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config_singleton = config_state(&mut deps.storage);
    let mut config = config_singleton.load()?;

    if config.mars_token_address == CanonicalAddr::default() {
        config.mars_token_address = deps.api.canonical_address(&env.message.sender)?;
        config_singleton.save(&config)?;
        Ok(HandleResponse {
            messages: vec![],
            log: vec![
                log("action", "init_mars_token"),
                log("token_address", &env.message.sender),
            ],
            data: None,
        })
    } else {
        // Can do this only once
        Err(StdError::unauthorized())
    }
}

pub fn handle_cast_vote<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    proposal_id: u64,
    vote_option: ProposalVoteOption,
) -> StdResult<HandleResponse> {
    let mut proposal = proposals_state_read(&deps.storage).load(&proposal_id.to_be_bytes())?;
    if proposal.status != ProposalStatus::Active {
        return Err(StdError::generic_err("Proposal is not active"));
    }

    if env.block.height > proposal.end_height {
        return Err(StdError::generic_err("Proposal has expired"));
    }

    let voter_canonical_address = deps.api.canonical_address(&env.message.sender)?;
    if proposal_votes_state_read(&deps.storage, proposal_id)
        .may_load(voter_canonical_address.as_slice())?
        .is_some()
    {
        return Err(StdError::generic_err(
            "User has already voted in this proposal",
        ));
    }

    let config = config_state_read(&deps.storage).load()?;

    let voting_power = xmars_get_balance_at(
        &deps.querier,
        deps.api.human_address(&config.xmars_token_address)?,
        env.message.sender.clone(),
        proposal.start_height - 1,
    )?;

    match vote_option {
        ProposalVoteOption::For => proposal.for_votes += voting_power,
        ProposalVoteOption::Against => proposal.against_votes += voting_power,
    };

    proposal_votes_state(&mut deps.storage, proposal_id).save(
        voter_canonical_address.as_slice(),
        &ProposalVote {
            option: vote_option.clone(),
            power: voting_power,
        },
    )?;

    proposals_state(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

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
    let config = config_state_read(&deps.storage).load()?;

    let proposals_bucket = proposals_state(&mut deps.storage);
    let mut proposal = proposals_bucket.load(&proposal_id.to_be_bytes())?;

    if proposal.status != ProposalStatus::Active {
        return Err(StdError::generic_err("Proposal is not active"));
    }

    if env.block.height <= proposal.end_height {
        return Err(StdError::generic_err("Voting period has not ended"));
    }

    // Compute proposal quorum and threshold
    let for_votes = proposal.for_votes;
    let against_votes = proposal.against_votes;
    let total_votes = for_votes + against_votes;
    let total_voting_power = xmars_get_total_supply_at(
        &deps.querier,
        deps.api.human_address(&config.xmars_token_address)?,
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
    let mut new_proposal_status = ProposalStatus::Rejected;
    let mut log_proposal_result = "rejected";
    let mut handle_response_messages = vec![];

    if proposal_quorum >= config.proposal_required_quorum
        && proposal_threshold >= config.proposal_required_threshold
    {
        new_proposal_status = ProposalStatus::Passed;
        log_proposal_result = "passed";
        // refund deposit amount to submitter
        handle_response_messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.mars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: deps
                    .api
                    .human_address(&proposal.submitter_canonical_address)?,
                amount: proposal.deposit_amount,
            })?,
        }));
    }

    if new_proposal_status == ProposalStatus::Rejected {
        handle_response_messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.mars_token_address)?,
            msg: to_binary(&Cw20HandleMsg::Transfer {
                recipient: deps.api.human_address(&config.staking_contract_address)?,
                amount: proposal.deposit_amount,
            })?,
            send: vec![],
        }))
    }

    // Update proposal status
    proposal.status = new_proposal_status;
    proposals_state(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

    Ok(HandleResponse {
        messages: handle_response_messages,
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
    let mut proposal = proposals_state_read(&deps.storage).load(&proposal_id.to_be_bytes())?;

    if proposal.status != ProposalStatus::Passed {
        return Err(StdError::generic_err(
            "Proposal has not passed or has already been executed",
        ));
    }

    let config = config_state_read(&deps.storage).load()?;
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
    proposals_state(&mut deps.storage).save(&proposal_id.to_be_bytes(), &proposal)?;

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

/// Mints Mars token to receiver (Temp action for testing)
pub fn handle_mint_mars<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    recipient: HumanAddr,
    amount: Uint128,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    // Only owner can trigger a mint
    if deps.api.canonical_address(&env.message.sender)? != config.owner {
        return Err(StdError::unauthorized());
    }

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.mars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint { recipient, amount }).unwrap(),
        })],
        log: vec![],
        data: None,
    })
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
        QueryMsg::Latest {} => to_binary(&query_latest_executed_proposal(deps)?),
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        mars_token_address: deps.api.human_address(&config.mars_token_address)?,
        xmars_token_address: deps.api.human_address(&config.xmars_token_address)?,
        proposal_required_deposit: config.proposal_required_deposit,
    })
}

fn query_proposals<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    start: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalsListResponse> {
    const DEFAULT_START: u64 = 1;
    const MAX_LIMIT: u32 = 30;
    const DEFAULT_LIMIT: u32 = 10;
    let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
    let start = start.unwrap_or(DEFAULT_START).to_be_bytes();
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let proposals = proposals_state_read(&deps.storage);
    let proposals_list: StdResult<Vec<_>> = proposals
        .range(Option::from(&start[..]), None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (k, v) = item?;

            Ok(ProposalInfo {
                proposal_id: read_be_u64(&mut k.as_slice()),
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
        proposal_count: basecamp.proposal_count,
        proposal_list: proposals_list?,
    })
}

fn query_proposal<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    proposal_id: u64,
) -> StdResult<ProposalInfo> {
    let proposal = proposals_state_read(&deps.storage).load(&proposal_id.to_be_bytes())?;

    Ok(ProposalInfo {
        proposal_id,
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
    let latest_execute_proposal = proposals_state_read(&deps.storage)
        .range(None, None, Order::Ascending)
        .filter(|proposal| {
            let (_, v) = proposal.as_ref().unwrap();
            v.status == ProposalStatus::Executed
        })
        .last();

    match latest_execute_proposal {
        Some(proposal) => {
            let (k, v) = proposal?;
            Ok(ProposalInfo {
                proposal_id: read_be_u64(&mut k.as_slice()),
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
        None => Result::Err(StdError::generic_err("No executed proposals found")),
    }
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
    use cosmwasm_std::{from_binary, Coin};
    use mars::testing::{
        get_test_addresses, mock_dependencies, mock_env, MarsMockQuerier, MockEnvParams,
    };

    use crate::state::{basecamp_state_read, proposals_state_read};

    const TEST_PROPOSAL_VOTING_PERIOD: u64 = 2000;
    const TEST_PROPOSAL_EFFECTIVE_DELAY: u64 = 200;
    const TEST_PROPOSAL_EXPIRATION_PERIOD: u64 = 300;
    const TEST_PROPOSAL_REQUIRED_DEPOSIT: Uint128 = Uint128(10000);

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            cw20_code_id: 11,
            xmars_token_address: HumanAddr::from("xmars_token"),
            staking_contract_address: HumanAddr::from("staking_contract"),

            proposal_voting_period: 1,
            proposal_effective_delay: 1,
            proposal_expiration_period: 1,
            proposal_required_deposit: Uint128(1),
            proposal_required_threshold: Decimal::one(),
            proposal_required_quorum: Decimal::one(),
        };
        let env = mock_env("owner", MockEnvParams::default());

        let res = init(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: 11,
                msg: to_binary(&cw20_token::msg::InitMsg {
                    name: "Mars token".to_string(),
                    symbol: "Mars".to_string(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        cap: None,
                    }),
                    init_hook: Some(cw20_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitTokenCallback {}).unwrap(),
                        contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                    }),
                })
                .unwrap(),
                send: vec![],
                label: None,
            }),],
            res.messages
        );

        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("owner"))
                .unwrap(),
            config.owner
        );
        assert_eq!(CanonicalAddr::default(), config.mars_token_address);

        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        assert_eq!(basecamp.proposal_count, 0);

        // mars token init callback
        let msg = HandleMsg::InitTokenCallback {};
        let env = mock_env("mars_token", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                log("action", "init_mars_token"),
                log("token_address", HumanAddr::from("mars_token")),
            ],
            res.log
        );
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        let xmars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("xmars_token"))
            .unwrap();
        assert_eq!(xmars_token_address, config.xmars_token_address);

        // trying again fails
        let msg = HandleMsg::InitTokenCallback {};
        let env = mock_env("mars_token_again", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );

        // query works now
        let res = query(&deps, QueryMsg::Config {}).unwrap();
        let config: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(HumanAddr::from("mars_token"), config.mars_token_address);
        assert_eq!(HumanAddr::from("xmars_token"), config.xmars_token_address);
    }

    #[test]
    fn test_mint_mars() {
        let mut deps = th_setup(&[]);

        // stake Mars -> should receive xMars
        let msg = HandleMsg::MintMars {
            recipient: HumanAddr::from("recipient"),
            amount: Uint128(3_500_000),
        };

        let env = mock_env("owner", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("mars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("recipient"),
                    amount: Uint128(3_500_000),
                })
                .unwrap(),
            })],
            res.messages
        );

        // mint by non owner -> Unauthorized
        let msg = HandleMsg::MintMars {
            recipient: HumanAddr::from("recipient"),
            amount: Uint128(3_500_000),
        };

        let env = mock_env("someoneelse", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();

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
        let _res = handle(&mut deps, env, msg).unwrap_err();
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

        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        assert_eq!(basecamp.proposal_count, 1);

        let proposal = proposals_state_read(&deps.storage)
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
                        msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
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

        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        assert_eq!(basecamp.proposal_count, 2);

        let proposal = proposals_state_read(&deps.storage)
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
                msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
            }])
        );
    }

    #[test]
    fn test_invalid_cast_votes() {
        let mut deps = th_setup(&[]);
        let voter_address = HumanAddr::from("voter");

        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(voter_address, Uint128(100))],
        );

        let active_proposal_id = 1_u64;
        let executed_proposal_id = 2_u64;
        th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Active,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );
        th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: active_proposal_id,
                status: ProposalStatus::Executed,
                start_height: 100_000,
                end_height: 100_100,
                ..Default::default()
            },
        );

        let msgs = vec![
            // voting a non-existent proposal should fail
            (
                HandleMsg::CastVote {
                    proposal_id: 3,
                    vote: ProposalVoteOption::For,
                },
                100_001,
            ),
            // voting a an inactive proposal should fail
            (
                HandleMsg::CastVote {
                    proposal_id: executed_proposal_id,
                    vote: ProposalVoteOption::For,
                },
                100_001,
            ),
            // voting after proposal end should fail
            (
                HandleMsg::CastVote {
                    proposal_id: active_proposal_id,
                    vote: ProposalVoteOption::For,
                },
                100_200,
            ),
        ];

        for (msg, block_height) in msgs {
            let env = mock_env(
                "voter",
                MockEnvParams {
                    block_height,
                    ..Default::default()
                },
            );
            handle(&mut deps, env, msg).unwrap_err();
        }
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
        proposals_state(&mut deps.storage)
            .save(&active_proposal_id.to_be_bytes(), &active_proposal)
            .unwrap();

        // Add another vote on an extra proposal to voter to validate voting on multiple proposals
        // is valid
        proposal_votes_state(&mut deps.storage, 4_u64)
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

        let proposal = proposals_state_read(&deps.storage)
            .load(&active_proposal_id.to_be_bytes())
            .unwrap();
        assert_eq!(proposal.for_votes, Uint128(100));
        assert_eq!(proposal.against_votes, Uint128(0));

        let proposal_vote = proposal_votes_state_read(&deps.storage, active_proposal_id)
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
        handle(&mut deps, env, msg).unwrap_err();

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

        let proposal = proposals_state_read(&deps.storage)
            .load(&active_proposal_id.to_be_bytes())
            .unwrap();
        assert_eq!(proposal.for_votes, Uint128(100 + 300));
        assert_eq!(proposal.against_votes, Uint128(200 + 400));
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

        let msgs = vec![
            // cannot end a proposal that has not ended its voting period
            (
                HandleMsg::EndProposal {
                    proposal_id: active_proposal_id,
                },
                100_000,
            ),
            // cannot end a non active proposal
            (
                HandleMsg::EndProposal {
                    proposal_id: executed_proposal_id,
                },
                100_001,
            ),
        ];

        for (msg, block_height) in msgs {
            let env = mock_env(
                "ender",
                MockEnvParams {
                    block_height,
                    ..Default::default()
                },
            );
            handle(&mut deps, env, msg).unwrap_err();
        }
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

        config_state(&mut deps.storage)
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

        let final_passed_proposal = proposals_state_read(&deps.storage)
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
                    recipient: HumanAddr::from("staking_contract"),
                    amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
                })
                .unwrap(),
                send: vec![],
            })]
        );

        let final_passed_proposal = proposals_state_read(&deps.storage)
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
                    recipient: HumanAddr::from("staking_contract"),
                    amount: TEST_PROPOSAL_REQUIRED_DEPOSIT,
                })
                .unwrap(),
                send: vec![],
            })]
        );

        let final_passed_proposal = proposals_state_read(&deps.storage)
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

        let msgs = vec![
            // cannot execute a non Passed proposal
            (
                HandleMsg::ExecuteProposal {
                    proposal_id: executed_proposal_id,
                },
                executed_proposal.end_height + TEST_PROPOSAL_EFFECTIVE_DELAY + 1,
            ),
            // cannot execute a proposal before the effective delay has passed
            (
                HandleMsg::ExecuteProposal {
                    proposal_id: passed_proposal_id,
                },
                passed_proposal.end_height + 1,
            ),
            // cannot execute an expired proposal
            (
                HandleMsg::ExecuteProposal {
                    proposal_id: passed_proposal_id,
                },
                passed_proposal.end_height
                    + TEST_PROPOSAL_EFFECTIVE_DELAY
                    + TEST_PROPOSAL_EXPIRATION_PERIOD
                    + 1,
            ),
        ];

        for (msg, block_height) in msgs {
            let env = mock_env(
                "executer",
                MockEnvParams {
                    block_height,
                    ..Default::default()
                },
            );
            handle(&mut deps, env, msg).unwrap_err();
        }
    }

    #[test]
    fn test_execute_proposals() {
        let mut deps = th_setup(&[]);
        let (contract_address, contract_canonical_address) =
            get_test_addresses(&deps.api, MOCK_CONTRACT_ADDR);

        let initial_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 1,
                status: ProposalStatus::Passed,
                end_height: 100_000,
                execute_calls: Some(vec![
                    ProposalExecuteCall {
                        execution_order: 2,
                        msg: to_binary(&HandleMsg::MintMars {
                            recipient: HumanAddr::from("someone"),
                            amount: Uint128(1000),
                        })
                        .unwrap(),
                        target_contract_canonical_address: contract_canonical_address.clone(),
                    },
                    ProposalExecuteCall {
                        execution_order: 3,
                        msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
                        target_contract_canonical_address: contract_canonical_address.clone(),
                    },
                    ProposalExecuteCall {
                        execution_order: 1,
                        msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
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
                    msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract_address.clone(),
                    send: vec![],
                    msg: to_binary(&HandleMsg::MintMars {
                        recipient: HumanAddr::from("someone"),
                        amount: Uint128(1000)
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract_address,
                    send: vec![],
                    msg: to_binary(&HandleMsg::UpdateConfig {}).unwrap(),
                }),
            ]
        );

        let final_proposal = proposals_state_read(&deps.storage)
            .load(&1_u64.to_be_bytes())
            .unwrap();

        assert_eq!(ProposalStatus::Executed, final_proposal.status);
    }

    // TEST HELPERS
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, MarsMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let msg = InitMsg {
            cw20_code_id: 1,
            xmars_token_address: HumanAddr::from("xmars_token"),
            staking_contract_address: HumanAddr::from("staking_contract"),

            proposal_voting_period: TEST_PROPOSAL_VOTING_PERIOD,
            proposal_effective_delay: TEST_PROPOSAL_EFFECTIVE_DELAY,
            proposal_expiration_period: TEST_PROPOSAL_EXPIRATION_PERIOD,
            proposal_required_deposit: TEST_PROPOSAL_REQUIRED_DEPOSIT,
            proposal_required_quorum: Decimal::one(),
            proposal_required_threshold: Decimal::one(),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let _res = init(&mut deps, env, msg).unwrap();

        let mut config_singleton = config_state(&mut deps.storage);
        let mut config = config_singleton.load().unwrap();
        config.mars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("mars_token"))
            .unwrap();
        config.xmars_token_address = deps
            .api
            .canonical_address(&HumanAddr::from("xmars_token"))
            .unwrap();
        config_singleton.save(&config).unwrap();

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

        proposals_state(&mut deps.storage)
            .save(&mock_proposal.id.to_be_bytes(), &proposal)
            .unwrap();

        proposal
    }
}
