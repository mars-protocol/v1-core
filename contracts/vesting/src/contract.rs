#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_binary, to_binary, Addr, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, Uint128, WasmMsg,
};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg};

use mars::helpers::{cw20_get_balance, cw20_get_total_supply};
use mars::vesting::msg::{
    AllocationResponse, ExecuteMsg, InstantiateMsg, QueryMsg, ReceiveMsg, SimulateWithdrawResponse,
};
use mars::vesting::{AllocationParams, AllocationStatus, Config, Stake};

use staking::msg::ReceiveMsg as MarsStakingReceiveMsg;

use crate::state::{CONFIG, PARAMS, STATUS, VOTING_POWER_SNAPSHOTS};

//----------------------------------------------------------------------------------------
// Entry Points
//----------------------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    CONFIG.save(
        deps.storage,
        &Config {
            owner: deps.api.addr_validate(&msg.owner)?,
            refund_recipient: deps.api.addr_validate(&msg.refund_recipient)?,
            mars_token: deps.api.addr_validate(&msg.mars_token)?,
            xmars_token: deps.api.addr_validate(&msg.xmars_token)?,
            mars_staking: deps.api.addr_validate(&msg.mars_staking)?,
            default_unlock_schedule: msg.default_unlock_schedule,
        },
    )?;
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, env: Env, info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::Receive(cw20_msg) => execute_receive_cw20(deps, env, info, cw20_msg),
        ExecuteMsg::Stake {} => execute_stake(deps, env, info),
        ExecuteMsg::Withdraw {} => execute_withdraw(deps, env, info),
        ExecuteMsg::Terminate {} => execute_terminate(deps, env, info),
        ExecuteMsg::TransferOwnership {
            new_owner,
            new_refund_recipient,
        } => execute_transfer_ownership(deps, env, info, new_owner, new_refund_recipient),
    }
}

fn execute_receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> StdResult<Response> {
    match from_binary(&cw20_msg.msg)? {
        ReceiveMsg::CreateAllocations { allocations } => execute_create_allocations(
            deps,
            env,
            info.clone(),
            cw20_msg.sender,
            info.sender,
            cw20_msg.amount,
            allocations,
        ),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps, env)?),
        QueryMsg::Allocation { account } => to_binary(&query_allocation(deps, env, account)?),
        QueryMsg::SimulateWithdraw { account } => {
            to_binary(&query_simulate_withdraw(deps, env, account)?)
        }
        QueryMsg::VotingPowerAt { account, block } => {
            to_binary(&query_voting_power(deps, env, account, block)?)
        }
    }
}

//----------------------------------------------------------------------------------------
// Execute Points
//----------------------------------------------------------------------------------------

fn execute_create_allocations(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    creator: String,
    deposit_token: Addr,
    deposit_amount: Uint128,
    allocations: Vec<(String, AllocationParams)>,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;

    if deps.api.addr_validate(&creator)? != config.owner {
        return Err(StdError::generic_err("Only owner can create allocations"));
    }

    if deposit_token != config.mars_token {
        return Err(StdError::generic_err("Only Mars token can be deposited"));
    }

    if deposit_amount != allocations.iter().map(|params| params.1.amount).sum() {
        return Err(StdError::generic_err("Deposit amount mismatch"));
    }

    for allocation in allocations {
        let (user_unchecked, params) = allocation;

        let user = deps.api.addr_validate(&user_unchecked)?;

        match PARAMS.load(deps.storage, &user) {
            Ok(..) => {
                return Err(StdError::generic_err("Allocation already exists for user"));
            }
            Err(..) => {
                PARAMS.save(deps.storage, &user, &params)?;
            }
        }

        match STATUS.load(deps.storage, &user) {
            Ok(..) => {
                return Err(StdError::generic_err("Allocation already exists for user"));
            }
            Err(..) => {
                STATUS.save(deps.storage, &user, &AllocationStatus::new())?;
            }
        }

        match VOTING_POWER_SNAPSHOTS.load(deps.storage, &user) {
            Ok(..) => {
                return Err(StdError::generic_err(
                    "Voting power history exists for user",
                ));
            }
            Err(..) => {
                VOTING_POWER_SNAPSHOTS.save(
                    deps.storage,
                    &user,
                    &vec![(env.block.height, Uint128::zero())],
                )?;
            }
        }
    }

    Ok(Response::default())
}

fn execute_stake(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let params = PARAMS.load(deps.storage, &info.sender)?;
    let mut status = STATUS.load(deps.storage, &info.sender)?;
    let mut snapshots = VOTING_POWER_SNAPSHOTS.load(deps.storage, &info.sender)?;

    // The amount available to be staked is: the amount of MARS vested so far, minus the amount
    // of MARS that have already been staked or withdrawan
    let mars_vested = helpers::compute_vested_or_unlocked_amount(
        env.block.time.seconds(),
        params.amount,
        &params.vest_schedule,
    );
    let mars_to_stake = mars_vested - status.mars_staked - status.mars_withdrawn_as_mars;

    // Calculate how many xMARS will be minted
    // https://github.com/mars-protocol/protocol/blob/master/contracts/staking/src/contract.rs#L187
    let mars_total_staked = cw20_get_balance(
        &deps.querier,
        config.mars_token.clone(),
        config.mars_staking.clone(),
    )?;
    let xmars_total_supply = cw20_get_total_supply(&deps.querier, config.xmars_token.clone())?;

    let xmars_to_mint =
        if mars_total_staked == Uint128::zero() || xmars_total_supply == Uint128::zero() {
            mars_to_stake
        } else {
            mars_to_stake.multiply_ratio(xmars_total_supply, mars_total_staked)
        };

    // Update status
    status.mars_staked += mars_to_stake;
    status.stakes.push(Stake {
        mars_staked: mars_to_stake,
        xmars_minted: xmars_to_mint,
    });
    STATUS.save(deps.storage, &info.sender, &status)?;

    // Update voting power snapshots
    let last_voting_power = snapshots[snapshots.len() - 1].1;
    snapshots.push((env.block.height, last_voting_power + xmars_to_mint));
    VOTING_POWER_SNAPSHOTS.save(deps.storage, &info.sender, &snapshots)?;

    Ok(Response::new().add_message(WasmMsg::Execute {
        contract_addr: config.mars_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.mars_staking.to_string(),
            amount: mars_to_stake,
            msg: to_binary(&MarsStakingReceiveMsg::Stake { recipient: None })?,
        })?,
        funds: vec![],
    }))
}

fn execute_withdraw(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let params = PARAMS.load(deps.storage, &info.sender)?;
    let mut status = STATUS.load(deps.storage, &info.sender)?;
    let mut snapshots = VOTING_POWER_SNAPSHOTS.load(deps.storage, &info.sender)?;

    let SimulateWithdrawResponse {
        mars_to_withdraw,
        mars_to_withdraw_as_xmars,
        xmars_to_withdraw,
    } = helpers::compute_withdraw_amounts(
        env.block.time.seconds(),
        &params,
        &mut status,
        config.default_unlock_schedule,
    );

    // Update status
    STATUS.save(deps.storage, &info.sender, &status)?;

    // Update snapshots
    let last_voting_power = snapshots[snapshots.len() - 1].1;
    snapshots.push((env.block.height, last_voting_power - xmars_to_withdraw));
    VOTING_POWER_SNAPSHOTS.save(deps.storage, &info.sender, &snapshots)?;

    let mut msgs: Vec<WasmMsg> = vec![];

    if !mars_to_withdraw.is_zero() {
        msgs.push(WasmMsg::Execute {
            contract_addr: config.mars_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: mars_to_withdraw,
            })?,
            funds: vec![],
        });
    }

    if !xmars_to_withdraw.is_zero() {
        msgs.push(WasmMsg::Execute {
            contract_addr: config.xmars_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: info.sender.to_string(),
                amount: xmars_to_withdraw,
            })?,
            funds: vec![],
        });
    }

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("mars_withdrawn", mars_to_withdraw)
        .add_attribute("mars_withdrawn_as_xmars", mars_to_withdraw_as_xmars)
        .add_attribute("xmars_withdrawn", xmars_to_withdraw))
}

fn execute_terminate(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let mut params = PARAMS.load(deps.storage, &info.sender)?;

    let timestamp = env.block.time.seconds();
    let mars_vested =
        helpers::compute_vested_or_unlocked_amount(timestamp, params.amount, &params.vest_schedule);

    // Refund the unvested MARS tokens to owner
    let mars_to_refund = params.amount - mars_vested;

    // Set the total allocation amount to the current vested amount, and vesting end time
    // to now. This will effectively end vesting and prevent more tokens to be vested
    params.amount = mars_vested;
    params.vest_schedule.duration = timestamp - params.vest_schedule.start_time;

    PARAMS.save(deps.storage, &info.sender, &params)?;

    let msg = WasmMsg::Execute {
        contract_addr: config.mars_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Transfer {
            recipient: config.refund_recipient.to_string(),
            amount: mars_to_refund,
        })?,
        funds: vec![],
    };

    Ok(Response::new()
        .add_message(msg)
        .add_attribute("mars_refunded", mars_to_refund)
        .add_attribute("new_amount", params.amount)
        .add_attribute(
            "new_vest_duration",
            format!("{}", params.vest_schedule.duration),
        ))
}

fn execute_transfer_ownership(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    new_owner: String,
    new_refund_recipient: String,
) -> StdResult<Response> {
    let mut config = CONFIG.load(deps.storage)?;

    if info.sender != config.owner {
        return Err(StdError::generic_err("Only owner can transfer ownership"));
    }

    config.owner = deps.api.addr_validate(&new_owner)?;
    config.refund_recipient = deps.api.addr_validate(&new_refund_recipient)?;

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new())
}

//----------------------------------------------------------------------------------------
// Query Functions
//----------------------------------------------------------------------------------------

fn query_config(deps: Deps, _env: Env) -> StdResult<Config<Addr>> {
    CONFIG.load(deps.storage)
}

fn query_allocation(deps: Deps, _env: Env, account: String) -> StdResult<AllocationResponse> {
    let account_checked = deps.api.addr_validate(&account)?;

    Ok(AllocationResponse {
        params: PARAMS.load(deps.storage, &account_checked)?,
        status: STATUS.load(deps.storage, &account_checked)?,
        voting_power_snapshots: VOTING_POWER_SNAPSHOTS.load(deps.storage, &account_checked)?,
    })
}

fn query_simulate_withdraw(
    deps: Deps,
    env: Env,
    account: String,
) -> StdResult<SimulateWithdrawResponse> {
    let account_checked = deps.api.addr_validate(&account)?;

    let config = CONFIG.load(deps.storage)?;
    let params = PARAMS.load(deps.storage, &account_checked)?;
    let mut status = STATUS.load(deps.storage, &account_checked)?;

    Ok(helpers::compute_withdraw_amounts(
        env.block.time.seconds(),
        &params,
        &mut status,
        config.default_unlock_schedule,
    ))
}

fn query_voting_power(deps: Deps, _env: Env, account: String, block: u64) -> StdResult<Uint128> {
    let account_checked = deps.api.addr_validate(&account)?;
    let snapshots = VOTING_POWER_SNAPSHOTS.load(deps.storage, &account_checked)?;

    Ok(helpers::binary_search(&snapshots, block))
}

//----------------------------------------------------------------------------------------
// Helper Functions
//----------------------------------------------------------------------------------------

mod helpers {
    use cosmwasm_std::Uint128;

    use mars::vesting::msg::SimulateWithdrawResponse;
    use mars::vesting::{AllocationParams, AllocationStatus, Schedule};

    use std::cmp;

    /// Adapted from Aave's implementation:
    /// https://github.com/aave/aave-token-v2/blob/master/contracts/token/base/GovernancePowerDelegationERC20.sol#L207
    pub fn binary_search(snapshots: &[(u64, Uint128)], block: u64) -> Uint128 {
        let mut lower = 0usize;
        let mut upper = snapshots.len() - 1;

        if block < snapshots[lower].0 {
            return Uint128::zero();
        }

        if snapshots[upper].0 < block {
            return snapshots[upper].1;
        }

        while lower < upper {
            let center = upper - (upper - lower) / 2;
            let snapshot = snapshots[center];

            #[allow(clippy::comparison_chain)]
            if snapshot.0 == block {
                return snapshot.1;
            } else if snapshot.0 < block {
                lower = center;
            } else {
                upper = center - 1;
            }
        }

        snapshots[lower].1
    }

    pub fn compute_vested_or_unlocked_amount(
        timestamp: u64,
        amount: Uint128,
        schedule: &Schedule,
    ) -> Uint128 {
        // Before the end of cliff period, no token will be vested/unlocked
        if timestamp < schedule.start_time + schedule.cliff {
            Uint128::zero()
        // After the end of cliff, tokens vest/unlock linearly between start time and end time
        } else if timestamp < schedule.start_time + schedule.duration {
            amount.multiply_ratio(timestamp - schedule.start_time, schedule.duration)
        // After end time, all tokens are fully vested/unlocked
        } else {
            amount
        }
    }

    pub fn compute_withdraw_amounts(
        timestamp: u64,
        params: &AllocationParams,
        status: &mut AllocationStatus,
        default_unlock_schedule: Schedule,
    ) -> SimulateWithdrawResponse {
        let unlock_schedule = match &params.unlock_schedule {
            Some(schedule) => schedule,
            None => &default_unlock_schedule,
        };

        // "Free" amount is the smaller between vested amount and unlocked amount
        let mars_vested =
            compute_vested_or_unlocked_amount(timestamp, params.amount, &params.vest_schedule);
        let mars_unlocked =
            compute_vested_or_unlocked_amount(timestamp, params.amount, unlock_schedule);

        let mars_free = cmp::min(mars_vested, mars_unlocked);

        // Withdrawable amount is unlocked amount minus the amount already withdrawn
        let mars_withdrawn = status.mars_withdrawn_as_mars + status.mars_withdrawn_as_xmars;
        let mars_withdrawable = mars_free - mars_withdrawn;

        // Find out how many MARS and xMARS to withdraw, respectively
        let mut mars_to_withdraw = mars_withdrawable;
        let mut xmars_to_withdraw = Uint128::zero();

        while !status.stakes.is_empty() {
            // We start from the earliest available stake
            // If more MARS is to be withdrawn than there is available in this stake, we empty
            // this stake and move on the to next one
            if mars_to_withdraw >= status.stakes[0].mars_staked {
                mars_to_withdraw -= status.stakes[0].mars_staked;
                xmars_to_withdraw += status.stakes[0].xmars_minted;

                status.stakes.remove(0);
            }
            // If there are more MARS in this stake than that is to be withdrawn, we deduct the
            // appropriate amounts from this stake, and break the loop
            else {
                let xmars_to_deduct = status.stakes[0]
                    .xmars_minted
                    .multiply_ratio(mars_to_withdraw, status.stakes[0].mars_staked);

                status.stakes[0].mars_staked -= mars_to_withdraw;
                status.stakes[0].xmars_minted -= xmars_to_deduct;

                mars_to_withdraw = Uint128::zero();
                xmars_to_withdraw += xmars_to_deduct;

                break;
            }
        }

        let mars_to_withdraw_as_xmars = mars_withdrawable - mars_to_withdraw;

        status.mars_withdrawn_as_mars += mars_to_withdraw;
        status.mars_withdrawn_as_xmars += mars_to_withdraw_as_xmars;

        SimulateWithdrawResponse {
            mars_to_withdraw,
            xmars_to_withdraw,
            mars_to_withdraw_as_xmars,
        }
    }
}

//----------------------------------------------------------------------------------------
// Tests
//----------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use cosmwasm_std::{CosmosMsg, SubMsg, Timestamp, WasmMsg};

    use mars::testing::{
        assert_generic_error_message, mock_dependencies, mock_env, mock_env_at_block_height,
        mock_env_at_block_time, mock_info, MockEnvParams,
    };
    use mars::vesting::msg::InstantiateMsg;
    use mars::vesting::Schedule;

    use super::*;

    const DEFAULT_UNLOCK_SCHEDULE: Schedule = Schedule {
        start_time: 1635724800, // 2021-11-01
        cliff: 31536000,        // 1 year (365 days)
        duration: 94608000,     // 3 years (3 * 365 days)
    };

    const PARAMS_1: AllocationParams = AllocationParams {
        amount: Uint128::new(100_000_000_000),
        vest_schedule: Schedule {
            start_time: 1614556800, // 2021-03-01
            cliff: 15552000,        // 180 days
            duration: 94608000,     // 3 years
        },
        unlock_schedule: None,
    };
    const PARAMS_2: AllocationParams = AllocationParams {
        amount: Uint128::new(100_000_000_000),
        vest_schedule: Schedule {
            start_time: 1638316800, // 2021-12-01
            cliff: 15552000,        // 180 days
            duration: 94608000,     // 3 years
        },
        unlock_schedule: None,
    };

    #[test]
    fn test_binary_search() {
        let snapshots = vec![(10000, Uint128::zero())];
        assert_eq!(helpers::binary_search(&snapshots, 10005), Uint128::zero());
        assert_eq!(helpers::binary_search(&snapshots, 10000), Uint128::zero());
        assert_eq!(helpers::binary_search(&snapshots, 9995), Uint128::zero());

        let snapshots = vec![
            (10000, Uint128::zero()),
            (10010, Uint128::new(12345)),
            (10020, Uint128::new(69420)),
            (10030, Uint128::new(88888)),
        ];

        assert_eq!(
            helpers::binary_search(&snapshots, 10035),
            Uint128::new(88888)
        );
        assert_eq!(
            helpers::binary_search(&snapshots, 10030),
            Uint128::new(88888)
        );
        assert_eq!(
            helpers::binary_search(&snapshots, 10025),
            Uint128::new(69420)
        );
        assert_eq!(
            helpers::binary_search(&snapshots, 10020),
            Uint128::new(69420)
        );
        assert_eq!(
            helpers::binary_search(&snapshots, 10015),
            Uint128::new(12345)
        );
        assert_eq!(
            helpers::binary_search(&snapshots, 10010),
            Uint128::new(12345)
        );
        assert_eq!(helpers::binary_search(&snapshots, 10005), Uint128::zero());
        assert_eq!(helpers::binary_search(&snapshots, 10000), Uint128::zero());
        assert_eq!(helpers::binary_search(&snapshots, 9995), Uint128::zero());
    }

    #[test]
    fn test_proper_initialization() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("owner");

        let res = instantiate(
            deps.as_mut(),
            env.clone(),
            info,
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        assert_eq!(res.messages.len(), 0);

        let res = query(deps.as_ref(), env, QueryMsg::Config {}).unwrap();
        let value: Config<Addr> = from_binary(&res).unwrap();

        assert_eq!(
            value,
            Config {
                owner: Addr::unchecked("owner"),
                refund_recipient: Addr::unchecked("refund_recipient"),
                mars_token: Addr::unchecked("mars_token"),
                xmars_token: Addr::unchecked("xmars_token"),
                mars_staking: Addr::unchecked("mars_staking"),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            }
        )
    }

    #[test]
    fn test_create_allocations() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("owner");

        // Instantiate contract
        instantiate(
            deps.as_mut(),
            env.clone(),
            info,
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        // Prepare messages to be used in creating allocations
        let receive_msg = ReceiveMsg::CreateAllocations {
            allocations: vec![
                ("user_1".to_string(), PARAMS_1.clone()),
                ("user_2".to_string(), PARAMS_2.clone()),
            ],
        };

        // Try create allocations with a non-owner address; should fail
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "not_owner".to_string(), // !!!
            amount: Uint128::new(200_000_000_000),
            msg: to_binary(&receive_msg).unwrap(),
        });
        let res = execute(deps.as_mut(), env.clone(), mock_info("mars_token"), msg);

        assert_generic_error_message(res, "Only owner can create allocations");

        // Try create allocations with a deposit token other than MARS; should fail
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "owner".to_string(),
            amount: Uint128::new(200_000_000_000),
            msg: to_binary(&receive_msg).unwrap(),
        });
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("not_mars_token"), // !!!
            msg,
        );

        assert_generic_error_message(res, "Only Mars token can be deposited");

        // Try create allocations whose total amount does not match deposit; should fail
        let msg = ExecuteMsg::Receive(Cw20ReceiveMsg {
            sender: "owner".to_string(),
            amount: Uint128::new(199_000_000_000), // !!!
            msg: to_binary(&receive_msg).unwrap(),
        });
        let res = execute(deps.as_mut(), env.clone(), mock_info("mars_token"), msg);

        assert_generic_error_message(res, "Deposit amount mismatch");

        // Create allocations properly
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("mars_token"),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "owner".to_string(),
                amount: Uint128::new(200_000_000_000),
                msg: to_binary(&receive_msg).unwrap(),
            }),
        )
        .unwrap();

        assert_eq!(res.messages.len(), 0);

        // Verify allocation response is correct for user 1
        let value: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.params, PARAMS_1);
        assert_eq!(value.status, AllocationStatus::new());

        // Verify allocation response is correct for user 2
        let value: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_2".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.params, PARAMS_2);
        assert_eq!(value.status, AllocationStatus::new());

        // Try create a second allocation for the same user; should fail
        let res = execute(
            deps.as_mut(),
            env,
            mock_info("mars_token"),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "owner".to_string(),
                amount: Uint128::new(100_000_000_000),
                msg: to_binary(&ReceiveMsg::CreateAllocations {
                    allocations: vec![("user_1".to_string(), PARAMS_1)],
                })
                .unwrap(),
            }),
        );

        assert_generic_error_message(res, "Allocation already exists for user");
    }

    // Some simple tests without considering stakes and withdrawls. For more complex tests
    // see `stake_and_withdraw_and_voting_power` function.
    #[test]
    fn test_simple_vesting() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        // Instantiate contract
        instantiate(
            deps.as_mut(),
            env.clone(),
            mock_info("owner"),
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        // Create Allocations
        execute(
            deps.as_mut(),
            env,
            mock_info("mars_token"),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "owner".to_string(),
                amount: Uint128::new(200_000_000_000),
                msg: to_binary(&ReceiveMsg::CreateAllocations {
                    allocations: vec![
                        ("user_1".to_string(), PARAMS_1),
                        ("user_2".to_string(), PARAMS_2),
                    ],
                })
                .unwrap(),
            }),
        )
        .unwrap();

        //--------------------------------------------------------------------------------
        // 2021-08-01
        // Zero should have been vested for user 1 (still under cliff)
        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                mock_env_at_block_time(1627776000),
                QueryMsg::SimulateWithdraw {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.mars_to_withdraw, Uint128::zero());

        //--------------------------------------------------------------------------------
        // 2022-03-01
        // 1/3 should have been vested, but non withdrawable because unlocking still under cliff
        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                mock_env_at_block_time(1646092800),
                QueryMsg::SimulateWithdraw {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.mars_to_withdraw, Uint128::zero());

        //--------------------------------------------------------------------------------
        // 2022-11-01
        // Ending of unlocking cliff
        // For user 1, unlocking is slower than vesting; withdrawable amount is unlocked amount
        // For user 2, vesting is slower than unlocking; withdrawable amount is vested amount
        let env = mock_env_at_block_time(1667260800);

        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::SimulateWithdraw {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        // 100000000000 * (1667260800 - 1635724800) / 94608000 = 33333333333
        assert_eq!(value.mars_to_withdraw, Uint128::new(33333333333));

        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::SimulateWithdraw {
                    account: "user_2".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        // 100000000000 * (1667260800 - 1638316800) / 94608000 = 30593607305
        assert_eq!(value.mars_to_withdraw, Uint128::new(30593607305));

        //--------------------------------------------------------------------------------
        // 2024-12-01
        // Completely vested & unlocked for both users
        let env = mock_env_at_block_time(1733011200);

        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::SimulateWithdraw {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.mars_to_withdraw, PARAMS_1.amount);

        let value: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::SimulateWithdraw {
                    account: "user_2".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(value.mars_to_withdraw, PARAMS_2.amount);
    }

    #[test]
    fn test_complex_vesting() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env_at_block_height(10000);
        let info = mock_info("owner");

        deps.querier
            .set_cw20_symbol(Addr::unchecked("mars_token"), "MARS".to_string());
        deps.querier
            .set_cw20_symbol(Addr::unchecked("xmars_token"), "xMARS".to_string());

        // Instantiate contract
        instantiate(
            deps.as_mut(),
            env.clone(),
            info,
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        // Create allocation
        execute(
            deps.as_mut(),
            env.clone(),
            mock_info("mars_token"),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "owner".to_string(),
                amount: Uint128::new(200_000_000_000),
                msg: to_binary(&ReceiveMsg::CreateAllocations {
                    allocations: vec![
                        ("user_1".to_string(), PARAMS_1.clone()),
                        ("user_2".to_string(), PARAMS_2.clone()),
                    ],
                })
                .unwrap(),
            }),
        )
        .unwrap();

        //--------------------------------------------------------------------------------
        // 2021-12-01
        let env = mock_env(MockEnvParams {
            block_height: 10010,
            block_time: Timestamp::from_seconds(1638316800),
        });

        // Assume the staking contract has 120,000 MARS staked and 100,000 xMARS circulating (1 xMARS = 1.2 MARS)
        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked("mars_staking"), Uint128::new(120000000000))],
        );
        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::new(100000000000));

        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Stake {},
        )
        .unwrap();

        // Expected amount of MARS to be staked at thie time
        // MARS staked = 100000000000 * (1638316800 - 1614556800) / 94608000 = 25114155251
        // xMARS minted = 25114155251 * 100000000000 / 120000000000 = 20928462709
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "mars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: "mars_staking".to_string(),
                    amount: Uint128::new(25114155251),
                    msg: to_binary(&MarsStakingReceiveMsg::Stake { recipient: None }).unwrap(),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env,
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::zero(),
                mars_withdrawn_as_xmars: Uint128::zero(),
                mars_staked: Uint128::new(25114155251),
                stakes: vec![Stake {
                    mars_staked: Uint128::new(25114155251),
                    xmars_minted: Uint128::new(20928462709)
                }]
            }
        );

        //--------------------------------------------------------------------------------
        // 2022-09-01
        let env = mock_env(MockEnvParams {
            block_height: 10020,
            block_time: Timestamp::from_seconds(1661990400),
        });

        // Assume now there're 300,000 MARS staked, 200,000 xMARS supply (1 xMARS = 1.5 MARS)
        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked("mars_staking"), Uint128::new(300000000000))],
        );
        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::new(200000000000));

        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Stake {},
        )
        .unwrap();

        // Vested amount = 100000000000 * (1661990400 - 1614556800) / 94608000 = 50136986301
        // Mars to stake = 50136986301 - 25114155251 = 25022831050
        // xMars to mint: 25022831050 * 2 / 3 = 16681887366
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "mars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: "mars_staking".to_string(),
                    amount: Uint128::new(25022831050),
                    msg: to_binary(&MarsStakingReceiveMsg::Stake { recipient: None }).unwrap(),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::zero(),
                mars_withdrawn_as_xmars: Uint128::zero(),
                mars_staked: Uint128::new(50136986301),
                stakes: vec![
                    Stake {
                        mars_staked: Uint128::new(25114155251),
                        xmars_minted: Uint128::new(20928462709)
                    },
                    Stake {
                        mars_staked: Uint128::new(25022831050),
                        xmars_minted: Uint128::new(16681887366)
                    }
                ]
            }
        );

        //--------------------------------------------------------------------------------
        // 2022-12-01
        let env = mock_env(MockEnvParams {
            block_height: 10030,
            block_time: Timestamp::from_seconds(1669852800),
        });

        // Assume now there're 525,000 MARS staked, 300,000 xMARS supply (1 xMARS = 1.75 MARS)
        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked("mars_staking"), Uint128::new(525000000000))],
        );
        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::new(300000000000));

        // Attempt a withdrawal
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Withdraw {},
        )
        .unwrap();

        // Since unlocked percentage (~33%) is lower than staked percentage (~50%), only xMARS
        // (no MARS) should be withdrawn here
        //
        // Unlocked amount = 100000000000 * (1669852800 - 1635724800) / 94608000
        // = 36073059360 uMARS
        //
        // Currently available stakes:
        // 1) 25114155251 uMARS in the form of 20928462709 uxMARS
        // 2) 25022831050 uMARS in the form of 16681887366 uxMARS
        //
        // Should withdraw all 20928462709 uxMARS from stake (1), and
        // 16681887366 * ((36073059360 - 25114155251) / 25022831050) = 7305936072 uxMARS from stake (2)
        //
        // Total xMARS to be withdrawn: 20928462709 + 7305936072 = 28234398781 uxMARS
        // Available stakes after withdrawal:
        // 1) 25022831050 - (36073059360 - 25114155251) = 14063926941 uMARS
        // in the form of 16681887366 - 7305936072 = 9375951294 xMARS
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "xmars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "user_1".to_string(),
                    amount: Uint128::new(28234398781),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::zero(),
                mars_withdrawn_as_xmars: Uint128::new(36073059360),
                mars_staked: Uint128::new(50136986301), // mars_staked_1 + mars_staked_2,
                stakes: vec![Stake {
                    mars_staked: Uint128::new(14063926941),
                    xmars_minted: Uint128::new(9375951294)
                }]
            }
        );

        // We attempt to stake immediately after the withdrawal
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Stake {},
        )
        .unwrap();

        // Vested amount = 100000000000 * (1669852800 - 1614556800) / 94608000 = 58447488584
        // Stakable amount = vested amount - MARS already staked - MARS withdrawn as naked MARS
        // = 58447488584 - 50136986301 - 0
        // = 8310502283
        // xMARS to be minted: 8310502283 * 300000000000 / 525000000000 = 4748858447
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "mars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: "mars_staking".to_string(),
                    amount: Uint128::new(8310502283),
                    msg: to_binary(&MarsStakingReceiveMsg::Stake { recipient: None }).unwrap(),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        //--------------------------------------------------------------------------------
        // 2024-03-01
        let env = mock_env(MockEnvParams {
            block_height: 10040,
            block_time: Timestamp::from_seconds(1709251200),
        });

        // Assume now there're 740,000 MARS staked, 400,000 xMARS supply (1 xMARS = 1.85 MARS)
        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked("mars_staking"), Uint128::new(740000000000))],
        );
        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::new(400000000000));

        // First let's make sure QueryMsg::SimulateWithdraw returns correct result
        //
        // Unlocked amount = 100000000000 * (1709251200 - 1635724800) / 94608000
        // = 77716894977 uMARS
        //
        // Withdrawable MARS = unlocked amount - MARS withdrawn as MARS - MARS withdrawn as xMARS
        // = 77716894977 - 0 - 36073059360
        // = 41643835617
        //
        // Currently available stakes:
        // 1) 14063926941 uMARS in the form of 9375951294 uxMARS
        // 2) 8310502283 uMARS in the form of 4748858447 uxMARS
        //
        // 41643835617 > 22374429224 (14063926941 + 8310502283) so all xMARS will be withdrawn
        // xMARS withdraw amount = 9375951294 + 4748858447 = 14124809741 uxMARS
        //
        // Then, 41643835617 - 22374429224 = 19269406393 uMARS will be withdrawn
        let res: SimulateWithdrawResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::SimulateWithdraw {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            res,
            SimulateWithdrawResponse {
                mars_to_withdraw: Uint128::new(19269406393),
                mars_to_withdraw_as_xmars: Uint128::new(22374429224),
                xmars_to_withdraw: Uint128::new(14124809741)
            }
        );

        // Not let's attempt the actual withdrawal
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Withdraw {},
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "mars_token".to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: "user_1".to_string(),
                        amount: Uint128::new(19269406393),
                    })
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "xmars_token".to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: "user_1".to_string(),
                        amount: Uint128::new(14124809741),
                    })
                    .unwrap(),
                    funds: vec![],
                }))
            ]
        );

        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        // Total amount of MARS withdrawn in the form of xMARS so far: 36073059360 + 22374429224 = 58447488584
        // Total amount of MARS staked so far: 50136986301 + 8310502283 = 58447488584
        // There should be no available stakes as they have all been withdrawn
        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::new(19269406393),
                mars_withdrawn_as_xmars: Uint128::new(58447488584),
                mars_staked: Uint128::new(58447488584),
                stakes: vec![]
            }
        );

        // We attempt to stake immediately after the withdrawal
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Stake {},
        )
        .unwrap();

        // Vested amount = 100000000000 (completely vested)
        // Stakable amount = vested amount - MARS already staked - MARS withdrawn as naked MARS
        // = 100000000000 - 58447488584 - 19269406393
        // = 22283105023 uMARS
        // xMARS to be minted: 22283105023 * 400000000000 / 740000000000 = 12044921634 uxMARS
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "mars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: "mars_staking".to_string(),
                    amount: Uint128::new(22283105023),
                    msg: to_binary(&MarsStakingReceiveMsg::Stake { recipient: None }).unwrap(),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        //--------------------------------------------------------------------------------
        // 2077-01-01
        let env = mock_env(MockEnvParams {
            block_height: 10050,
            block_time: Timestamp::from_seconds(3376684800),
        });

        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Withdraw {},
        )
        .unwrap();

        // All xMARS should be withdrawn
        // There's no MARS left to be withdrawn
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "xmars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "user_1".to_string(),
                    amount: Uint128::new(12044921634),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        // MARS withdrawn as xMARS = 58447488584 + 22283105023 = 80730593607
        // Total amount of MARS withdrawn = 19269406393 + 80730593607 = 100000000000 (equals the total amount, good)
        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::new(19269406393),
                mars_withdrawn_as_xmars: Uint128::new(80730593607),
                mars_staked: Uint128::new(80730593607),
                stakes: vec![]
            }
        );

        //--------------------------------------------------------------------------------
        // Test query voting powers
        let blocks: Vec<u64> = vec![
            9995, 10000, 10005, 10010, 10015, 10020, 10025, 10030, 10035, 10040, 10045, 10050,
            10055,
        ];

        let voting_powers: Vec<Uint128> = blocks
            .iter()
            .map(|block| {
                from_binary(
                    &query(
                        deps.as_ref(),
                        env.clone(),
                        QueryMsg::VotingPowerAt {
                            account: "user_1".to_string(),
                            block: *block,
                        },
                    )
                    .unwrap(),
                )
                .unwrap()
            })
            .collect();

        assert_eq!(
            voting_powers,
            vec![
                Uint128::new(0),           // 9995
                Uint128::new(0),           // 10000
                Uint128::new(0),           // 10005
                Uint128::new(20928462709), // 10010: mint 20928462709 uxMARS
                Uint128::new(20928462709), // 10015
                Uint128::new(37610350075), // 10020: mint 16681887366 uxMARS
                Uint128::new(37610350075), // 10025
                Uint128::new(14124809741), // 10030: withdraw 28234398781, then mint 4748858447
                Uint128::new(14124809741), // 10035
                Uint128::new(12044921634), // 10040: withdraw all, then mint 12044921634
                Uint128::new(12044921634), // 10045
                Uint128::new(0),           // 10050: withdraw all
                Uint128::new(0)            // 10055
            ]
        );
    }

    #[test]
    fn test_terminate() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("owner");

        deps.querier
            .set_cw20_symbol(Addr::unchecked("mars_token"), "MARS".to_string());
        deps.querier
            .set_cw20_symbol(Addr::unchecked("xmars_token"), "xMARS".to_string());

        // Instantiate contract
        instantiate(
            deps.as_mut(),
            env.clone(),
            info,
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        // Create allocation
        execute(
            deps.as_mut(),
            env.clone(),
            mock_info("mars_token"),
            ExecuteMsg::Receive(Cw20ReceiveMsg {
                sender: "owner".to_string(),
                amount: Uint128::new(100_000_000_000),
                msg: to_binary(&ReceiveMsg::CreateAllocations {
                    allocations: vec![("user_1".to_string(), PARAMS_1.clone())],
                })
                .unwrap(),
            }),
        )
        .unwrap();

        // Before terminating the allocation, we first do some staking and withdrawals to complicate the matter

        //--------------------------------------------------------------------------------
        // 2022-09-01
        let env = mock_env_at_block_time(1661990400);

        // Assume now there're 300,000 MARS staked, 200,000 xMARS supply (1 xMARS = 1.5 MARS)
        deps.querier.set_cw20_balances(
            Addr::unchecked("mars_token"),
            &[(Addr::unchecked("mars_staking"), Uint128::new(300000000000))],
        );
        deps.querier
            .set_cw20_total_supply(Addr::unchecked("xmars_token"), Uint128::new(200000000000));

        // Vested amount = 100000000000 * (1661990400 - 1614556800) / 94608000 = 50136986301
        // Will stake 50136986301 uMARS, and get back 50136986301 * 2 / 3 = 33424657534 uxMARS
        execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Stake {},
        )
        .unwrap();

        //--------------------------------------------------------------------------------
        // 2022-12-01
        let env = mock_env_at_block_time(1669852800);

        // Unlocked amount = 100000000000 * (1669852800 - 1635724800) / 94608000 = 36073059360
        // xMARS to be withdrawn: 36073059360 * 33424657534 / 50136986301 = 24048706240 uxMARS
        // Remaining stakes:
        // 50136986301 - 36073059360 = 14063926941 uMARS in the form of
        // 33424657534 - 24048706240 = 9375951294 uxMARS
        execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Withdraw {},
        )
        .unwrap();

        //--------------------------------------------------------------------------------
        // 2023-03-01
        let env = mock_env_at_block_time(1677628800);

        // Verify the status before termination
        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::zero(),
                mars_withdrawn_as_xmars: Uint128::new(36073059360),
                mars_staked: Uint128::new(50136986301),
                stakes: vec![Stake {
                    mars_staked: Uint128::new(14063926941),
                    xmars_minted: Uint128::new(9375951294)
                }]
            }
        );

        // Attempt to terminate the allocation
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Terminate {},
        )
        .unwrap();

        // Vested amount = 100000000000 * (1677628800 - 1614556800) / 94608000 = 66666666666
        // Unvested amount = 100000000000 - 66666666666 = 33333333334
        // Unvested tokens should be refunded to `refund_recipient`
        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: "mars_token".to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: "refund_recipient".to_string(),
                    amount: Uint128::new(33333333334),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        // Verify the status after termination
        let res: AllocationResponse = from_binary(
            &query(
                deps.as_ref(),
                env.clone(),
                QueryMsg::Allocation {
                    account: "user_1".to_string(),
                },
            )
            .unwrap(),
        )
        .unwrap();

        // Parameters should have been adjusted
        assert_eq!(
            res.params,
            AllocationParams {
                amount: Uint128::new(66666666666),
                vest_schedule: Schedule {
                    start_time: 1614556800,
                    cliff: 15552000,
                    duration: 63072000 // 1677628800 - 1614556800
                },
                unlock_schedule: None
            }
        );

        // Status should be unchanged
        assert_eq!(
            res.status,
            AllocationStatus {
                mars_withdrawn_as_mars: Uint128::zero(),
                mars_withdrawn_as_xmars: Uint128::new(36073059360),
                mars_staked: Uint128::new(50136986301),
                stakes: vec![Stake {
                    mars_staked: Uint128::new(14063926941),
                    xmars_minted: Uint128::new(9375951294)
                }]
            }
        );

        //--------------------------------------------------------------------------------
        // 2077-01-01
        let env = mock_env_at_block_time(3376684800);

        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("user_1"),
            ExecuteMsg::Withdraw {},
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "mars_token".to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: "user_1".to_string(),
                        amount: Uint128::new(16529680365), // 66666666666 - 50136986301
                    })
                    .unwrap(),
                    funds: vec![],
                })),
                SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: "xmars_token".to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::Transfer {
                        recipient: "user_1".to_string(),
                        amount: Uint128::new(9375951294), // the remaining stake
                    })
                    .unwrap(),
                    funds: vec![],
                }))
            ]
        );
    }

    #[test]
    fn test_transfer_ownership() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());
        let info = mock_info("owner");

        deps.querier
            .set_cw20_symbol(Addr::unchecked("mars_token"), "MARS".to_string());
        deps.querier
            .set_cw20_symbol(Addr::unchecked("xmars_token"), "xMARS".to_string());

        // Instantiate contract
        instantiate(
            deps.as_mut(),
            env.clone(),
            info,
            InstantiateMsg {
                owner: "owner".to_string(),
                refund_recipient: "refund_recipient".to_string(),
                mars_token: "mars_token".to_string(),
                xmars_token: "xmars_token".to_string(),
                mars_staking: "mars_staking".to_string(),
                default_unlock_schedule: DEFAULT_UNLOCK_SCHEDULE,
            },
        )
        .unwrap();

        // Try to transfer ownership as an unauthorized person; should fail
        let res = execute(
            deps.as_mut(),
            env.clone(),
            mock_info("random_person"),
            ExecuteMsg::TransferOwnership {
                new_owner: "ngmi".to_string(),
                new_refund_recipient: "hfsp".to_string(),
            },
        );

        assert_generic_error_message(res, "Only owner can transfer ownership");
    }
}
