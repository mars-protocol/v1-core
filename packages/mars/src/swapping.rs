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
    // Having the same asset as offer and ask asset doesn't make any sense
    if offer_asset_info == ask_asset_info {
        return Err(StdError::generic_err(format!(
            "Cannot swap an asset into itself. Both offer and ask assets were specified as {}",
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
    use crate::testing::{
        assert_generic_error_message, mock_dependencies, mock_env, MockEnvParams,
    };
    use cosmwasm_std::testing::MOCK_CONTRACT_ADDR;

    #[test]
    fn test_cannot_swap_same_assets() {
        let mut deps = mock_dependencies(20, &[]);
        let env = mock_env("owner", MockEnvParams::default());

        let assets = vec![
            (
                "somecoin_addr",
                AssetInfo::Token {
                    contract_addr: HumanAddr::from("somecoin_addr"),
                },
            ),
            (
                "uluna",
                AssetInfo::NativeToken {
                    denom: "uluna".to_string(),
                },
            ),
        ];
        for (asset_name, asset_info) in assets {
            let response = handle_swap(
                &mut deps,
                env.clone(),
                asset_info.clone(),
                asset_info,
                None,
                HumanAddr::from("terraswap_factory"),
                None,
            );
            assert_generic_error_message(
                response,
                &format!("Cannot swap an asset into itself. Both offer and ask assets were specified as {}", asset_name),
            );
        }
    }

    #[test]
    fn test_cannot_swap_asset_with_zero_balance() {
        let mut deps = mock_dependencies(20, &[]);
        let env = mock_env("owner", MockEnvParams::default());

        let cw20_contract_address = HumanAddr::from("cw20_zero");
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), Uint128::zero())],
        );

        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address,
        };
        let ask_asset_info = AssetInfo::NativeToken {
            denom: "uusd".to_string(),
        };

        let response = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            HumanAddr::from("terraswap_factory"),
            None,
        );
        assert_generic_error_message(response, "Contract has no balance for the asset cw20_zero")
    }

    #[test]
    fn test_cannot_swap_more_than_contract_balance() {
        let mut deps = mock_dependencies(
            20,
            &[Coin {
                denom: "somecoin".to_string(),
                amount: Uint128(1_000_000),
            }],
        );
        let env = mock_env("owner", MockEnvParams::default());

        let offer_asset_info = AssetInfo::NativeToken {
            denom: "somecoin".to_string(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: HumanAddr::from("cw20_token"),
        };

        let response = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128(1_000_001)),
            HumanAddr::from("terraswap_factory"),
            None,
        );
        assert_generic_error_message(
            response,
            "The amount requested for swap exceeds contract balance for the asset somecoin",
        )
    }

    #[test]
    fn test_swap_contract_token_partial_balance() {
        let mut deps = mock_dependencies(20, &[]);
        let env = mock_env("owner", MockEnvParams::default());

        let cw20_contract_address = HumanAddr::from("cw20");
        let contract_asset_balance = Uint128(1_000_000);
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(HumanAddr::from(MOCK_CONTRACT_ADDR), contract_asset_balance)],
        );

        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.clone(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: HumanAddr::from("mars"),
        };

        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: HumanAddr::from("pair_cw20_mars"),
            liquidity_token: HumanAddr::from("lp_cw20_mars"),
        });

        let res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128(999)),
            HumanAddr::from("terraswap_factory"),
            None,
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
                            max_spread: None,
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

    #[test]
    fn test_swap_native_token_total_balance() {
        let contract_asset_balance = Uint128(1_234_567);
        let mut deps = mock_dependencies(
            20,
            &[Coin {
                denom: "uusd".to_string(),
                amount: contract_asset_balance,
            }],
        );
        let env = mock_env("owner", MockEnvParams::default());

        let offer_asset_info = AssetInfo::NativeToken {
            denom: "uusd".to_string(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: HumanAddr::from("mars"),
        };

        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: HumanAddr::from("pair_uusd_mars"),
            liquidity_token: HumanAddr::from("lp_uusd_mars"),
        });

        let res = handle_swap(
            &mut deps,
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            HumanAddr::from("terraswap_factory"),
            Some(Decimal::from_ratio(1u128, 100u128)),
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: HumanAddr::from("pair_uusd_mars"),
                msg: to_binary(&TerraswapPairHandleMsg::Swap {
                    offer_asset: TerraswapAsset {
                        info: AssetInfo::NativeToken {
                            denom: "uusd".to_string(),
                        },
                        amount: contract_asset_balance,
                    },
                    belief_price: None,
                    max_spread: Some(Decimal::from_ratio(1u128, 100u128)),
                    to: None,
                })
                .unwrap(),
                send: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: contract_asset_balance,
                }],
            })]
        );

        assert_eq!(res.log, vec![log("action", "swap"), log("asset", "uusd")]);
    }
}
