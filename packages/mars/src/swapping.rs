use crate::helpers::cw20_get_balance;
use cosmwasm_std::{
    log, to_binary, Api, Coin, CosmosMsg, Decimal, Empty, Env, Extern, HandleResponse, HumanAddr,
    Querier, StdError, StdResult, Storage, Uint128, WasmMsg,
};
use cw20::Cw20HandleMsg;
use terraswap::asset::{Asset as TerraswapAsset, AssetInfo, PairInfo};
use terraswap::pair::HandleMsg as TerraswapPairHandleMsg;
use terraswap::querier::query_pair_info;

/// Swap assets via terraswap
pub fn handle_swap<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    offer_asset_info: AssetInfo,
    ask_asset_info: AssetInfo,
    amount: Option<Uint128>,
    terraswap_factory_human_addr: HumanAddr,
    terraswap_max_spread: Option<Decimal>,
) -> StdResult<HandleResponse> {
    // swapping the same assets doesn't make any sense
    if offer_asset_info == ask_asset_info {
        return Err(StdError::generic_err(format!(
            "Cannot swap the same assets {}",
            offer_asset_info
        )));
    }

    let (contract_asset_balance, asset_label) = match offer_asset_info.clone() {
        AssetInfo::NativeToken { denom } => (
            deps.querier
                .query_balance(env.contract.address, denom.as_str())?
                .amount,
            denom,
        ),
        AssetInfo::Token { contract_addr } => {
            let asset_label = String::from(contract_addr.as_str());
            (
                cw20_get_balance(&deps.querier, contract_addr, env.contract.address)?,
                asset_label,
            )
        }
    };

    if contract_asset_balance.is_zero() {
        return Err(StdError::generic_err(format!(
            "Contract has no balance for the asset {}",
            asset_label
        )));
    }

    let amount_to_swap = match amount {
        Some(amount) if amount > contract_asset_balance => {
            return Err(StdError::generic_err(format!(
                "The amount requested for swap exceeds contract balance for the asset {}",
                asset_label
            )));
        }
        Some(amount) => amount,
        None => contract_asset_balance,
    };

    let pair_info: PairInfo = query_pair_info(
        &deps,
        &terraswap_factory_human_addr,
        &[offer_asset_info.clone(), ask_asset_info],
    )?;

    let offer_asset = TerraswapAsset {
        info: offer_asset_info,
        amount: amount_to_swap,
    };
    let send_msg = asset_into_swap_msg(
        deps,
        pair_info.contract_addr,
        offer_asset,
        terraswap_max_spread,
    )?;

    Ok(HandleResponse {
        messages: vec![send_msg],
        log: vec![log("action", "swap"), log("asset", asset_label)],
        data: None,
    })
}

/// Construct terraswap message in order to swap assets
fn asset_into_swap_msg<S: Storage, A: Api, Q: Querier>(
    _deps: &Extern<S, A, Q>,
    pair_contract: HumanAddr,
    offer_asset: TerraswapAsset,
    max_spread: Option<Decimal>,
) -> StdResult<CosmosMsg<Empty>> {
    let message = match offer_asset.info.clone() {
        AssetInfo::NativeToken { denom } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_contract,
            msg: to_binary(&TerraswapPairHandleMsg::Swap {
                offer_asset: offer_asset.clone(),
                belief_price: None,
                max_spread,
                to: None,
            })?,
            send: vec![Coin {
                denom,
                amount: offer_asset.amount,
            }],
        }),
        AssetInfo::Token { contract_addr } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            msg: to_binary(&Cw20HandleMsg::Send {
                contract: pair_contract,
                amount: offer_asset.amount,
                msg: Some(to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset,
                    belief_price: None,
                    max_spread,
                    to: None,
                })?),
            })?,
            send: vec![],
        }),
    };
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{mock_dependencies, mock_env, MockEnvParams};
    use cosmwasm_std::testing::MOCK_CONTRACT_ADDR;

    /*#[test]
    fn test_swap_asset_to_uusd() {
        let contract_asset_balance = Uint128(1_000_000);
        let mut deps = th_setup(&[
            Coin {
                denom: "somecoin".to_string(),
                amount: contract_asset_balance,
            },
            Coin {
                denom: "zero".to_string(),
                amount: Uint128::zero(),
            },
        ]);

        // *
        // can't swap the same assets
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "uusd".to_string(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Cannot swap the same assets uusd")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap Mars
        // *
        let config = config_state(&mut deps.storage).load().unwrap();
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::Token {
                contract_addr: deps.api.human_address(&config.mars_token_address).unwrap(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(msg, "Cannot swap Mars"),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap asset with zero balance
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "zero".to_string(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Contract has no balance for the asset zero")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap amount greater than contract balance
        // *
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "somecoin".to_string(),
            },
            amount: Some(Uint128(1_000_001)),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle(&mut deps, env, msg);
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "The amount requested for swap exceeds contract balance for the asset somecoin"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // swap
        // *
        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [
                AssetInfo::NativeToken {
                    denom: "somecoin".to_string(),
                },
                AssetInfo::NativeToken {
                    denom: "uusd".to_string(),
                },
            ],
            contract_addr: HumanAddr::from("pair_somecoin_uusd"),
            liquidity_token: HumanAddr::from("lp_somecoin_uusd"),
        });

        // swap less than balance
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "somecoin".to_string(),
            },
            amount: Some(Uint128(999)),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("pair_somecoin_uusd"),
                msg: to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset: TerraswapAsset {
                        info: AssetInfo::NativeToken {
                            denom: "somecoin".to_string(),
                        },
                        amount: Uint128(999),
                    },
                    belief_price: None,
                    max_spread: Some(config.terraswap_max_spread),
                    to: None,
                })
                    .unwrap(),
                send: vec![Coin {
                    denom: "somecoin".to_string(),
                    amount: Uint128(999),
                }],
            })]
        );
        assert_eq!(
            res.log,
            vec![log("action", "swap"), log("asset", "somecoin")]
        );

        // swap all balance
        let msg = HandleMsg::SwapAssetToUusd {
            offer_asset_info: AssetInfo::NativeToken {
                denom: "somecoin".to_string(),
            },
            amount: None,
        };
        let env = mock_env("owner", MockEnvParams::default());
        let res = handle(&mut deps, env, msg).unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("pair_somecoin_uusd"),
                msg: to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset: TerraswapAsset {
                        info: AssetInfo::NativeToken {
                            denom: "somecoin".to_string(),
                        },
                        amount: contract_asset_balance,
                    },
                    belief_price: None,
                    max_spread: Some(config.terraswap_max_spread),
                    to: None,
                })
                    .unwrap(),
                send: vec![Coin {
                    denom: "somecoin".to_string(),
                    amount: contract_asset_balance,
                }],
            })]
        );
        assert_eq!(
            res.log,
            vec![log("action", "swap"), log("asset", "somecoin")]
        );
    }*/

    #[test]
    fn test_swap_asset_to_mars() {
        let mut deps = mock_dependencies(20, &[]);

        let terraswap_factory_human_addr = HumanAddr::from("terraswap_factory");
        let terraswap_max_spread: Option<Decimal> = None;

        // *
        // can't swap the same assets
        // *
        let contract_human_addr = HumanAddr::from("somecoin_addr");
        let offer_asset_info = AssetInfo::Token {
            contract_addr: contract_human_addr.clone(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: contract_human_addr.clone(),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info.clone(),
            None,
            terraswap_factory_human_addr.clone(),
            terraswap_max_spread,
        );
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                format!(
                    "Cannot swap the same assets {}",
                    contract_human_addr.as_str()
                )
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // can't swap asset with zero balance
        // *
        let cw20_contract_address = HumanAddr::from("cw20_zero");
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), Uint128::zero())],
        );

        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address,
        };

        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info.clone(),
            None,
            terraswap_factory_human_addr.clone(),
            terraswap_max_spread,
        );
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => {
                assert_eq!(msg, "Contract has no balance for the asset cw20_zero")
            }
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        let cw20_contract_address = HumanAddr::from("cw20_token");
        let contract_asset_balance = Uint128(1_000_000);
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), contract_asset_balance)],
        );

        // *
        // can't swap amount greater than contract balance
        // *
        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.clone(),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let error_res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info.clone(),
            Some(Uint128(1_000_001)),
            terraswap_factory_human_addr.clone(),
            terraswap_max_spread,
        );
        match error_res {
            Err(StdError::GenericErr { msg, .. }) => assert_eq!(
                msg,
                "The amount requested for swap exceeds contract balance for the asset cw20_token"
            ),
            other_err => panic!("Unexpected error: {:?}", other_err),
        }

        // *
        // swap
        // *
        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [
                AssetInfo::Token {
                    contract_addr: cw20_contract_address.clone(),
                },
                AssetInfo::Token {
                    contract_addr: contract_human_addr,
                },
            ],
            contract_addr: HumanAddr::from("pair_cw20_mars"),
            liquidity_token: HumanAddr::from("lp_cw20_mars"),
        });

        // swap less than balance
        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.clone(),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info.clone(),
            Some(Uint128(999)),
            terraswap_factory_human_addr.clone(),
            terraswap_max_spread,
        )
        .unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_address.clone(),
                msg: to_binary(&Cw20HandleMsg::Send {
                    contract: HumanAddr::from("pair_cw20_mars"),
                    amount: Uint128(999),
                    msg: Some(
                        to_binary(&TerraswapPairHandleMsg::Swap {
                            offer_asset: TerraswapAsset {
                                info: AssetInfo::Token {
                                    contract_addr: cw20_contract_address.clone(),
                                },
                                amount: Uint128(999),
                            },
                            belief_price: None,
                            max_spread: terraswap_max_spread,
                            to: None,
                        })
                        .unwrap()
                    ),
                })
                .unwrap(),
                send: vec![],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "swap"),
                log("asset", cw20_contract_address.as_str()),
            ]
        );

        // swap all balance
        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.clone(),
        };
        let env = mock_env("owner", MockEnvParams::default());
        let res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            terraswap_factory_human_addr,
            terraswap_max_spread,
        )
        .unwrap();
        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_address.clone(),
                msg: to_binary(&Cw20HandleMsg::Send {
                    contract: HumanAddr::from("pair_cw20_mars"),
                    amount: contract_asset_balance,
                    msg: Some(
                        to_binary(&TerraswapPairHandleMsg::Swap {
                            offer_asset: TerraswapAsset {
                                info: AssetInfo::Token {
                                    contract_addr: cw20_contract_address.clone(),
                                },
                                amount: contract_asset_balance,
                            },
                            belief_price: None,
                            max_spread: terraswap_max_spread,
                            to: None,
                        })
                        .unwrap()
                    ),
                })
                .unwrap(),
                send: vec![],
            })]
        );
        assert_eq!(
            res.log,
            vec![
                log("action", "swap"),
                log("asset", cw20_contract_address.as_str()),
            ]
        );
    }
}
