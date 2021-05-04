use cosmwasm_std::{
    from_binary, log, to_binary, Api, Binary, CanonicalAddr, CosmosMsg, Decimal, Env, Extern,
    HandleResponse, HumanAddr, InitResponse, MigrateResponse, MigrateResult, Querier, StdError,
    StdResult, Storage, Uint128, WasmMsg,
};

use cw20::{Cw20HandleMsg, Cw20ReceiveMsg, MinterResponse};
use mars::cw20_token;
use mars::helpers::{cw20_get_balance, cw20_get_total_supply};

use crate::msg::{
    ConfigResponse, HandleMsg, InitMsg, MigrateMsg, MsgExecuteCall, QueryMsg, ReceiveMsg,
};
use crate::state::{
    basecamp_state, basecamp_state_read, config_state, config_state_read, cooldowns_state,
    proposal_votes_state, proposal_votes_state_read, proposals_state, proposals_state_read,
    Basecamp, Config, Cooldown, Proposal, ProposalExecuteCall, ProposalStatus, ProposalVote,
    ProposalVoteOption,
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
        xmars_token_address: CanonicalAddr::default(),
        cooldown_duration: msg.cooldown_duration,
        unstake_window: msg.unstake_window,

        proposal_voting_period: msg.proposal_voting_period,
        proposal_effective_delay: msg.proposal_effective_delay,
        proposal_expiration_period: msg.proposal_expiration_period,
        proposal_required_deposit: msg.proposal_required_deposit,
        proposal_required_quorum: msg.proposal_required_quorum,
        proposal_required_threshold: msg.proposal_required_threshold,
    };

    config_state(&mut deps.storage).save(&config)?;

    // initialize State
    basecamp_state(&mut deps.storage).save(&Basecamp {
        proposal_count: 0,
        proposal_total_deposits: Uint128(0),
    })?;

    // Prepare response, should instantiate Mars and xMars
    // and use the Register hook
    Ok(InitResponse {
        log: vec![],
        // TODO: Tokens are initialized here. Evaluate doing this outside of
        // the contract
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Instantiate {
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
                        msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 0 })?,
                        contract_addr: env.contract.address.clone(),
                    }),
                })?,
                send: vec![],
                label: None,
            }),
            CosmosMsg::Wasm(WasmMsg::Instantiate {
                code_id: msg.cw20_code_id,
                msg: to_binary(&cw20_token::msg::InitMsg {
                    name: "xMars token".to_string(),
                    symbol: "xMars".to_string(),
                    decimals: 6,
                    initial_balances: vec![],
                    mint: Some(MinterResponse {
                        minter: HumanAddr::from(env.contract.address.as_str()),
                        cap: None,
                    }),
                    init_hook: Some(cw20_token::msg::InitHook {
                        msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 1 })?,
                        contract_addr: env.contract.address,
                    }),
                })?,
                send: vec![],
                label: None,
            }),
        ],
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
        HandleMsg::InitTokenCallback { token_id } => {
            handle_init_token_callback(deps, env, token_id)
        }

        HandleMsg::Cooldown {} => handle_cooldown(deps, env),

        HandleMsg::CastVote {
            proposal_id,
            vote,
            voting_power,
        } => handle_cast_vote(deps, env, proposal_id, vote, voting_power),

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
            ReceiveMsg::Stake => handle_stake(deps, env, cw20_msg.sender, cw20_msg.amount),
            ReceiveMsg::Unstake => handle_unstake(deps, env, cw20_msg.sender, cw20_msg.amount),
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

/// Mint xMars tokens to staker
pub fn handle_stake<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    stake_amount: Uint128,
) -> StdResult<HandleResponse> {
    // check stake is valid
    let config = config_state_read(&deps.storage).load()?;
    // Has to send Mars tokens
    if deps.api.canonical_address(&env.message.sender)? != config.mars_token_address {
        return Err(StdError::unauthorized());
    }
    if stake_amount == Uint128(0) {
        return Err(StdError::generic_err("Stake amount must be greater than 0"));
    }

    let total_mars_in_basecamp = cw20_get_balance(
        deps,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )?;
    // Mars amount needs to be before the stake transaction (which is already in the basecamp's
    // balance so it needs to be deducted)
    // Mars deposited for proposals is not taken into account
    let basecamp = basecamp_state_read(&deps.storage).load()?;
    let mars_to_deduct = basecamp.proposal_total_deposits + stake_amount;
    let net_total_mars_in_basecamp = (total_mars_in_basecamp - mars_to_deduct)?;

    let total_xmars_supply =
        cw20_get_total_supply(deps, deps.api.human_address(&config.xmars_token_address)?)?;

    let mint_amount =
        if net_total_mars_in_basecamp == Uint128(0) || total_xmars_supply == Uint128(0) {
            stake_amount
        } else {
            stake_amount.multiply_ratio(total_xmars_supply, net_total_mars_in_basecamp)
        };

    Ok(HandleResponse {
        messages: vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.human_address(&config.xmars_token_address)?,
            send: vec![],
            msg: to_binary(&Cw20HandleMsg::Mint {
                recipient: staker.clone(),
                amount: mint_amount,
            })?,
        })],
        log: vec![
            log("action", "stake"),
            log("user", staker),
            log("mars_staked", stake_amount),
            log("xmars_minted", mint_amount),
        ],
        data: None,
    })
}

/// Burn xMars tokens and send corresponding Mars
pub fn handle_unstake<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    staker: HumanAddr,
    burn_amount: Uint128,
) -> StdResult<HandleResponse> {
    // check unstake is valid
    let config = config_state_read(&deps.storage).load()?;
    if deps.api.canonical_address(&env.message.sender)? != config.xmars_token_address {
        return Err(StdError::unauthorized());
    }
    if burn_amount == Uint128(0) {
        return Err(StdError::generic_err(
            "Unstake amount must be greater than 0",
        ));
    }

    // check valid cooldown
    //
    let mut cooldowns_bucket = cooldowns_state(&mut deps.storage);
    let staker_canonical_addr = deps.api.canonical_address(&staker)?;
    match cooldowns_bucket.may_load(staker_canonical_addr.as_slice())? {
        Some(mut cooldown) => {
            if burn_amount > cooldown.amount {
                return Err(StdError::generic_err(
                    "Unstake amount must not be greater than cooldown amount",
                ));
            }
            if env.block.time < cooldown.timestamp + config.cooldown_duration {
                return Err(StdError::generic_err("Cooldown has not finished"));
            }
            if env.block.time
                > cooldown.timestamp + config.cooldown_duration + config.unstake_window
            {
                return Err(StdError::generic_err("Cooldown has expired"));
            }

            if burn_amount == cooldown.amount {
                cooldowns_bucket.remove(staker_canonical_addr.as_slice());
            } else {
                cooldown.amount = (cooldown.amount - burn_amount)?;
                cooldowns_bucket.save(staker_canonical_addr.as_slice(), &cooldown)?;
            }
        }

        None => {
            return Err(StdError::generic_err(
                "Address must have a valid cooldown to unstake",
            ))
        }
    };

    let total_mars_in_basecamp = cw20_get_balance(
        deps,
        deps.api.human_address(&config.mars_token_address)?,
        env.contract.address,
    )?;

    // Mars from proposal deposits is not taken into account
    let basecamp = basecamp_state_read(&deps.storage).load()?;
    let net_total_mars_in_basecamp = (total_mars_in_basecamp - basecamp.proposal_total_deposits)?;

    let total_xmars_supply =
        cw20_get_total_supply(deps, deps.api.human_address(&config.xmars_token_address)?)?;

    let unstake_amount = burn_amount.multiply_ratio(net_total_mars_in_basecamp, total_xmars_supply);

    Ok(HandleResponse {
        messages: vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.xmars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Burn {
                    amount: burn_amount,
                })?,
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: deps.api.human_address(&config.mars_token_address)?,
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Transfer {
                    recipient: staker.clone(),
                    amount: unstake_amount,
                })?,
            }),
        ],
        log: vec![
            log("action", "unstake"),
            log("user", staker),
            log("mars_unstaked", unstake_amount),
            log("xmars_burned", burn_amount),
        ],
        data: None,
    })
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
    basecamp.proposal_total_deposits += deposit_amount;
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
        title: title,
        description: description,
        link: option_link,
        execute_calls: option_proposal_execute_calls,
        deposit_amount: deposit_amount,
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

/// Handles token post initialization storing the addresses
/// in config
/// token is a byte: 0 = Mars, 1 = xMars, others are not authorized
pub fn handle_init_token_callback<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    token_id: u8,
) -> StdResult<HandleResponse> {
    let mut config_singleton = config_state(&mut deps.storage);
    let mut config = config_singleton.load()?;

    return match token_id {
        // Mars
        0 => {
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
        // xMars
        1 => {
            if config.xmars_token_address == CanonicalAddr::default() {
                config.xmars_token_address = deps.api.canonical_address(&env.message.sender)?;
                config_singleton.save(&config)?;
                Ok(HandleResponse {
                    messages: vec![],
                    log: vec![
                        log("action", "init_xmars_token"),
                        log("token_address", &env.message.sender),
                    ],
                    data: None,
                })
            } else {
                // Can do this only once
                Err(StdError::unauthorized())
            }
        }
        _ => Err(StdError::unauthorized()),
    };
}

/// Handles cooldown. if staking non zero amount, activates a cooldown for that amount.
/// If a cooldown exists and amount has changed it computes the weighted average
/// for the cooldown
pub fn handle_cooldown<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config = config_state_read(&deps.storage).load()?;

    // get total mars in contract before the stake transaction
    let xmars_balance = cw20_get_balance(
        deps,
        deps.api.human_address(&config.xmars_token_address)?,
        env.message.sender.clone(),
    )?;

    if xmars_balance.is_zero() {
        return Err(StdError::unauthorized());
    }

    let mut cooldowns_bucket = cooldowns_state(&mut deps.storage);
    let sender_canonical_address = deps.api.canonical_address(&env.message.sender)?;

    // compute new cooldown timestamp
    let new_cooldown_timestamp =
        match cooldowns_bucket.may_load(sender_canonical_address.as_slice())? {
            Some(cooldown) => {
                let minimal_valid_cooldown_timestamp =
                    env.block.time - config.cooldown_duration - config.unstake_window;

                if cooldown.timestamp < minimal_valid_cooldown_timestamp {
                    env.block.time
                } else {
                    let mut extra_amount: u128 = 0;
                    if xmars_balance > cooldown.amount {
                        extra_amount = xmars_balance.u128() - cooldown.amount.u128();
                    };

                    (((cooldown.timestamp as u128) * cooldown.amount.u128()
                        + (env.block.time as u128) * extra_amount)
                        / (cooldown.amount.u128() + extra_amount)) as u64
                }
            }

            None => env.block.time,
        };

    cooldowns_bucket.save(
        &sender_canonical_address.as_slice(),
        &Cooldown {
            amount: xmars_balance,
            timestamp: new_cooldown_timestamp,
        },
    )?;

    Ok(HandleResponse {
        log: vec![
            log("action", "cooldown"),
            log("user", env.message.sender),
            log("cooldown_amount", xmars_balance),
            log("cooldown_timestamp", new_cooldown_timestamp),
        ],
        data: None,
        messages: vec![],
    })
}

pub fn handle_cast_vote<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    proposal_id: u64,
    vote_option: ProposalVoteOption,
    voting_power: Uint128,
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

    // TODO: this should get the balance at the proposal start block once the custom xMars
    // when snapshot balances is implemented
    let max_voting_power = cw20_get_balance(
        deps,
        deps.api.human_address(&config.xmars_token_address)?,
        env.message.sender.clone(),
    )?;

    if voting_power > max_voting_power {
        return Err(StdError::generic_err(
            "User does not have enough voting power",
        ));
    }

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
    // TODO: When implementing balance snapshots, this should get the total xmars supply
    // at the start of the proposal
    let total_voting_power =
        cw20_get_total_supply(deps, deps.api.human_address(&config.xmars_token_address)?)?;

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
        // refund deposit amount to sumbitter
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
    // TODO: If staking gets separated from basecamp, a transfer needs to be sent to the
    // contract that handles the staking

    // Update deposit totals
    basecamp_state(&mut deps.storage).update(|mut basecamp| {
        basecamp.proposal_total_deposits =
            (basecamp.proposal_total_deposits - proposal.deposit_amount)?;
        Ok(basecamp)
    })?;

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
        messages: messages,
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
    }
}

fn query_config<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<ConfigResponse> {
    let config = config_state_read(&deps.storage).load()?;
    Ok(ConfigResponse {
        mars_token_address: deps.api.human_address(&config.mars_token_address)?,
        xmars_token_address: deps.api.human_address(&config.xmars_token_address)?,
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

// TESTS

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{MockApi, MockStorage, MOCK_CONTRACT_ADDR};
    use cosmwasm_std::{from_binary, Coin};
    use mars::testing::{
        get_test_addresses, mock_dependencies, mock_env, MockEnvParams, WasmMockQuerier,
    };

    use crate::state::{basecamp_state_read, cooldowns_state_read, proposals_state_read};

    const TEST_COOLDOWN_DURATION: u64 = 1000;
    const TEST_UNSTAKE_WINDOW: u64 = 100;
    const TEST_PROPOSAL_VOTING_PERIOD: u64 = 2000;
    const TEST_PROPOSAL_EFFECTIVE_DELAY: u64 = 200;
    const TEST_PROPOSAL_EXPIRATION_PERIOD: u64 = 300;
    const TEST_PROPOSAL_REQUIRED_DEPOSIT: Uint128 = Uint128(10000);

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(20, &[]);

        let msg = InitMsg {
            cw20_code_id: 11,
            cooldown_duration: 20,
            unstake_window: 10,

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
            vec![
                CosmosMsg::Wasm(WasmMsg::Instantiate {
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
                            msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 0 }).unwrap(),
                            contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        }),
                    })
                    .unwrap(),
                    send: vec![],
                    label: None,
                }),
                CosmosMsg::Wasm(WasmMsg::Instantiate {
                    code_id: 11,
                    msg: to_binary(&cw20_token::msg::InitMsg {
                        name: "xMars token".to_string(),
                        symbol: "xMars".to_string(),
                        decimals: 6,
                        initial_balances: vec![],
                        mint: Some(MinterResponse {
                            minter: HumanAddr::from(MOCK_CONTRACT_ADDR),
                            cap: None,
                        }),
                        init_hook: Some(cw20_token::msg::InitHook {
                            msg: to_binary(&HandleMsg::InitTokenCallback { token_id: 1 }).unwrap(),
                            contract_addr: HumanAddr::from(MOCK_CONTRACT_ADDR),
                        }),
                    })
                    .unwrap(),
                    send: vec![],
                    label: None,
                })
            ],
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
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        assert_eq!(basecamp.proposal_count, 0);

        // mars token init callback
        let msg = HandleMsg::InitTokenCallback { token_id: 0 };
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
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // trying again fails
        let msg = HandleMsg::InitTokenCallback { token_id: 0 };
        let env = mock_env("mars_token_again", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(CanonicalAddr::default(), config.xmars_token_address);

        // xmars token init callback
        let msg = HandleMsg::InitTokenCallback { token_id: 1 };
        let env = mock_env("xmars_token", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            vec![
                log("action", "init_xmars_token"),
                log("token_address", HumanAddr::from("xmars_token")),
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
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("xmars_token"))
                .unwrap(),
            config.xmars_token_address
        );

        // trying again fails
        let msg = HandleMsg::InitTokenCallback { token_id: 1 };
        let env = mock_env("xmars_token_again", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();
        let config = config_state_read(&deps.storage).load().unwrap();

        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("mars_token"))
                .unwrap(),
            config.mars_token_address
        );
        assert_eq!(
            deps.api
                .canonical_address(&HumanAddr::from("xmars_token"))
                .unwrap(),
            config.xmars_token_address
        );

        // query works now
        let res = query(&deps, QueryMsg::Config {}).unwrap();
        let config: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(HumanAddr::from("mars_token"), config.mars_token_address);
        assert_eq!(HumanAddr::from("xmars_token"), config.xmars_token_address);
    }

    #[test]
    fn test_staking() {
        let mut deps = th_setup(&[]);
        let staker_canonical_addr = deps
            .api
            .canonical_address(&HumanAddr::from("staker"))
            .unwrap();

        let mut basecamp_singleton = basecamp_state(&mut deps.storage);
        let mut basecamp = basecamp_singleton.load().unwrap();
        let proposal_total_deposits = Uint128(200_000);
        basecamp.proposal_total_deposits = proposal_total_deposits;
        basecamp_singleton.save(&basecamp).unwrap();

        // no Mars in pool (Except for a proposal deposit)
        // stake X Mars -> should receive X xMars
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), Uint128(2_200_000))],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), Uint128(0));

        let env = mock_env("mars_token", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("xmars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("staker"),
                    amount: Uint128(2_000_000),
                })
                .unwrap(),
            })],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "stake"),
                log("user", HumanAddr::from("staker")),
                log("mars_staked", 2_000_000),
                log("xmars_minted", 2_000_000),
            ],
            res.log
        );

        // Some Mars in pool and some xMars supply
        // stake Mars -> should receive less xMars
        let stake_amount = Uint128(2_000_000);
        let mars_in_basecamp = Uint128(4_000_000);
        let xmars_supply = Uint128(1_000_000);

        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: stake_amount,
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), mars_in_basecamp)],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), xmars_supply);

        let env = mock_env("mars_token", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();

        let expected_minted_xmars = stake_amount.multiply_ratio(
            xmars_supply,
            (mars_in_basecamp - (proposal_total_deposits + stake_amount)).unwrap(),
        );

        assert_eq!(
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("xmars_token"),
                send: vec![],
                msg: to_binary(&Cw20HandleMsg::Mint {
                    recipient: HumanAddr::from("staker"),
                    amount: expected_minted_xmars,
                })
                .unwrap(),
            })],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "stake"),
                log("user", HumanAddr::from("staker")),
                log("mars_staked", stake_amount),
                log("xmars_minted", expected_minted_xmars),
            ],
            res.log
        );

        // stake other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Stake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: Uint128(2_000_000),
        });

        let env = mock_env("other_token", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // setup variables for unstake
        let unstake_amount = Uint128(1_000_000);
        let unstake_mars_in_basecamp = Uint128(4_000_000);
        let unstake_xmars_supply = Uint128(3_000_000);
        let unstake_block_timestamp = 1_000_000_000;
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: unstake_amount,
        });

        deps.querier.set_cw20_balances(
            HumanAddr::from("mars_token"),
            &[(
                HumanAddr::from(MOCK_CONTRACT_ADDR),
                unstake_mars_in_basecamp,
            )],
        );

        deps.querier
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), unstake_xmars_supply);

        // unstake Mars no cooldown -> unauthorized
        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars expired cooldown -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp
                        - TEST_COOLDOWN_DURATION
                        - TEST_UNSTAKE_WINDOW
                        - 1,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars unfinished cooldown -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount,
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION + 1,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // unstake Mars cooldown with low amount -> unauthorized
        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: (unstake_amount - Uint128(1000)).unwrap(),
                    timestamp: unstake_block_timestamp - TEST_COOLDOWN_DURATION,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, msg.clone()).unwrap_err();

        // partial unstake Mars valid cooldown -> burn xMars, receive Mars back,
        // deduct cooldown amount
        let pending_cooldown_amount = Uint128(300_000);
        let pending_cooldown_timestamp = unstake_block_timestamp - TEST_COOLDOWN_DURATION;

        cooldowns_state(&mut deps.storage)
            .save(
                staker_canonical_addr.as_slice(),
                &Cooldown {
                    amount: unstake_amount + pending_cooldown_amount,
                    timestamp: pending_cooldown_timestamp,
                },
            )
            .unwrap();

        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, msg).unwrap();

        let net_unstake_mars_in_basecamp =
            (unstake_mars_in_basecamp - proposal_total_deposits).unwrap();
        let expected_returned_mars =
            unstake_amount.multiply_ratio(net_unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("xmars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: unstake_amount,
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("mars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: HumanAddr::from("staker"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                }),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "unstake"),
                log("user", HumanAddr::from("staker")),
                log("mars_unstaked", expected_returned_mars),
                log("xmars_burned", unstake_amount),
            ],
            res.log
        );

        let actual_cooldown = cooldowns_state_read(&deps.storage)
            .load(staker_canonical_addr.as_slice())
            .unwrap();

        assert_eq!(actual_cooldown.amount, pending_cooldown_amount);
        assert_eq!(actual_cooldown.timestamp, pending_cooldown_timestamp);

        // unstake other token -> Unauthorized
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: pending_cooldown_amount,
        });

        let env = mock_env(
            "other_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );

        let _res = handle(&mut deps, env, msg).unwrap_err();

        // unstake pending amount Mars -> cooldown is deleted
        let env = mock_env(
            "xmars_token",
            MockEnvParams {
                block_time: unstake_block_timestamp,
                ..Default::default()
            },
        );
        let msg = HandleMsg::Receive(Cw20ReceiveMsg {
            msg: Some(to_binary(&ReceiveMsg::Unstake).unwrap()),
            sender: HumanAddr::from("staker"),
            amount: pending_cooldown_amount,
        });
        let res = handle(&mut deps, env, msg).unwrap();

        // NOTE: In reality the mars/xmars amounts would change but since they are being
        // mocked it does not really matter here.
        let expected_returned_mars = pending_cooldown_amount
            .multiply_ratio(net_unstake_mars_in_basecamp, unstake_xmars_supply);

        assert_eq!(
            vec![
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("xmars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Burn {
                        amount: pending_cooldown_amount,
                    })
                    .unwrap(),
                }),
                CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: HumanAddr::from("mars_token"),
                    send: vec![],
                    msg: to_binary(&Cw20HandleMsg::Transfer {
                        recipient: HumanAddr::from("staker"),
                        amount: expected_returned_mars,
                    })
                    .unwrap(),
                }),
            ],
            res.messages
        );
        assert_eq!(
            vec![
                log("action", "unstake"),
                log("user", HumanAddr::from("staker")),
                log("mars_unstaked", expected_returned_mars),
                log("xmars_burned", pending_cooldown_amount),
            ],
            res.log
        );

        let actual_cooldown = cooldowns_state_read(&deps.storage)
            .may_load(staker_canonical_addr.as_slice())
            .unwrap();

        assert_eq!(actual_cooldown, None);
    }

    #[test]
    fn test_cooldown() {
        let mut deps = th_setup(&[]);

        let initial_block_time = 1_600_000_000;
        let ongoing_cooldown_block_time = initial_block_time + TEST_COOLDOWN_DURATION / 2;

        // staker with no xmars is unauthorized
        let msg = HandleMsg::Cooldown {};

        let env = mock_env("staker", MockEnvParams::default());
        let _res = handle(&mut deps, env, msg).unwrap_err();

        // staker with xmars gets a cooldown equal to the xmars balance
        let initial_xmars_balance = Uint128(1_000_000);
        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(HumanAddr::from("staker"), initial_xmars_balance)],
        );

        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: initial_block_time,
                ..Default::default()
            },
        );
        let res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);
        assert_eq!(
            vec![
                log("action", "cooldown"),
                log("user", "staker"),
                log("cooldown_amount", initial_xmars_balance),
                log("cooldown_timestamp", initial_block_time)
            ],
            res.log
        );

        // same amount does not alterate cooldown
        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: ongoing_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, initial_block_time);
        assert_eq!(cooldown.amount, initial_xmars_balance);

        // more amount gets a weighted average timestamp with the new amount
        let additional_xmars_balance = Uint128(500_000);

        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(
                HumanAddr::from("staker"),
                initial_xmars_balance + additional_xmars_balance,
            )],
        );
        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: ongoing_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        let expected_cooldown_timestamp =
            (((initial_block_time as u128) * initial_xmars_balance.u128()
                + (ongoing_cooldown_block_time as u128) * additional_xmars_balance.u128())
                / (initial_xmars_balance + additional_xmars_balance).u128()) as u64;
        assert_eq!(cooldown.timestamp, expected_cooldown_timestamp);
        assert_eq!(
            cooldown.amount,
            initial_xmars_balance + additional_xmars_balance
        );

        // expired cooldown with more amount gets a new timestamp (test lower and higher)
        let expired_cooldown_block_time =
            expected_cooldown_timestamp + TEST_COOLDOWN_DURATION + TEST_UNSTAKE_WINDOW + 1;
        let expired_balance = initial_xmars_balance + additional_xmars_balance + Uint128(800_000);
        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(HumanAddr::from("staker"), expired_balance)],
        );

        let env = mock_env(
            "staker",
            MockEnvParams {
                block_time: expired_cooldown_block_time,
                ..Default::default()
            },
        );
        let _res = handle(&mut deps, env, HandleMsg::Cooldown {}).unwrap();

        let cooldown = cooldowns_state_read(&deps.storage)
            .load(
                deps.api
                    .canonical_address(&HumanAddr::from("staker"))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        assert_eq!(cooldown.timestamp, expired_cooldown_block_time);
        assert_eq!(cooldown.amount, expired_balance);
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
        assert_eq!(
            basecamp.proposal_total_deposits,
            TEST_PROPOSAL_REQUIRED_DEPOSIT
        );

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
        assert_eq!(
            basecamp.proposal_total_deposits,
            TEST_PROPOSAL_REQUIRED_DEPOSIT + TEST_PROPOSAL_REQUIRED_DEPOSIT
        );

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
            &[(voter_address.clone(), Uint128(100))],
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
            // voting a non-existent proposal shold fail
            (
                HandleMsg::CastVote {
                    proposal_id: 3,
                    vote: ProposalVoteOption::For,
                    voting_power: Uint128(100),
                },
                100_001,
            ),
            // voting a an inactive proposal should fail
            (
                HandleMsg::CastVote {
                    proposal_id: executed_proposal_id,
                    vote: ProposalVoteOption::For,
                    voting_power: Uint128(100),
                },
                100_001,
            ),
            // voting after proposal end should fail
            (
                HandleMsg::CastVote {
                    proposal_id: active_proposal_id,
                    vote: ProposalVoteOption::For,
                    voting_power: Uint128(100),
                },
                100_200,
            ),
            // voting with more power than available should fail
            (
                HandleMsg::CastVote {
                    proposal_id: active_proposal_id,
                    vote: ProposalVoteOption::For,
                    voting_power: Uint128(101),
                },
                100_001,
            ),
        ];

        for (msg, block_height) in msgs {
            let env = mock_env(
                "voter",
                MockEnvParams {
                    block_height: block_height,
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

        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(voter_address.clone(), Uint128(100))],
        );

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
                    power: Uint128(2),
                },
            )
            .unwrap();

        // Valid vote for
        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
            voting_power: Uint128(100),
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
            voting_power: Uint128(100),
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
            voting_power: Uint128(200),
        };

        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[(HumanAddr::from("voter2"), Uint128(300))], // more balance just to check less can be used
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
        deps.querier.set_cw20_balances(
            HumanAddr::from("xmars_token"),
            &[
                (HumanAddr::from("voter3"), Uint128(300)),
                (HumanAddr::from("voter4"), Uint128(400)),
            ],
        );

        let msg = HandleMsg::CastVote {
            proposal_id: active_proposal_id,
            vote: ProposalVoteOption::For,
            voting_power: Uint128(300),
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
            voting_power: Uint128(400),
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
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), Uint128(100_000));

        basecamp_state(&mut deps.storage)
            .update(|mut basecamp| {
                basecamp.proposal_total_deposits = Uint128(100_000);
                Ok(basecamp)
            })
            .unwrap();

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
                    block_height: block_height,
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
            .set_cw20_total_supply(HumanAddr::from("xmars_token"), Uint128(100_000));

        let initial_proposal_deposits = Uint128(100_000);
        basecamp_state(&mut deps.storage)
            .update(|mut basecamp| {
                basecamp.proposal_total_deposits = initial_proposal_deposits;
                Ok(basecamp)
            })
            .unwrap();

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
                end_height: proposal_end_height + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 1 };

        let env = mock_env(
            "ender",
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
        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        let expected_proposal_total_deposits =
            (initial_proposal_deposits - TEST_PROPOSAL_REQUIRED_DEPOSIT).unwrap();
        assert_eq!(
            basecamp.proposal_total_deposits,
            expected_proposal_total_deposits
        );

        // end rejected proposal (no quorum)
        let initial_passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 2,
                status: ProposalStatus::Active,
                for_votes: Uint128(11),
                against_votes: Uint128(10),
                end_height: proposal_end_height + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 2 };

        let env = mock_env(
            "ender",
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

        assert_eq!(res.messages, vec![]);

        let final_passed_proposal = proposals_state_read(&deps.storage)
            .load(&2u64.to_be_bytes())
            .unwrap();
        assert_eq!(final_passed_proposal.status, ProposalStatus::Rejected);
        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        let expected_proposal_total_deposits =
            (expected_proposal_total_deposits - TEST_PROPOSAL_REQUIRED_DEPOSIT).unwrap();
        assert_eq!(
            basecamp.proposal_total_deposits,
            expected_proposal_total_deposits
        );

        // end rejected proposal (no threshold)
        let initial_passed_proposal = th_build_mock_proposal(
            &mut deps,
            MockProposal {
                id: 3,
                status: ProposalStatus::Active,
                for_votes: Uint128(10_000),
                against_votes: Uint128(11_000),
                end_height: proposal_end_height + 1,
                ..Default::default()
            },
        );

        let msg = HandleMsg::EndProposal { proposal_id: 3 };

        let env = mock_env(
            "ender",
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

        assert_eq!(res.messages, vec![]);

        let final_passed_proposal = proposals_state_read(&deps.storage)
            .load(&3u64.to_be_bytes())
            .unwrap();
        assert_eq!(final_passed_proposal.status, ProposalStatus::Rejected);
        let basecamp = basecamp_state_read(&deps.storage).load().unwrap();
        let expected_proposal_total_deposits =
            (expected_proposal_total_deposits - TEST_PROPOSAL_REQUIRED_DEPOSIT).unwrap();
        assert_eq!(
            basecamp.proposal_total_deposits,
            expected_proposal_total_deposits
        );
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
                    block_height: block_height,
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
                        target_contract_canonical_address: contract_canonical_address.clone(),
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
                    contract_addr: contract_address.clone(),
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
    fn th_setup(contract_balances: &[Coin]) -> Extern<MockStorage, MockApi, WasmMockQuerier> {
        let mut deps = mock_dependencies(20, contract_balances);

        // TODO: Do we actually need the init to happen on tests?
        let msg = InitMsg {
            cw20_code_id: 1,
            cooldown_duration: TEST_COOLDOWN_DURATION,
            unstake_window: TEST_UNSTAKE_WINDOW,

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
        deps: &mut Extern<MockStorage, MockApi, WasmMockQuerier>,
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
