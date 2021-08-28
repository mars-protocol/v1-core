use crate::helpers::cw20_get_balance;
use cosmwasm_std::{
    attr, to_binary, Addr, Coin, CosmosMsg, Decimal, DepsMut, Empty, Env, Response, StdError,
    StdResult, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use terraswap::asset::{Asset as TerraswapAsset, AssetInfo, PairInfo};
use terraswap::pair::ExecuteMsg as TerraswapPairExecuteMsg;
use terraswap::querier::query_pair_info;

/// Swap assets via terraswap
pub fn execute_swap(
    deps: DepsMut,
    env: Env,
    offer_asset_info: AssetInfo,
    ask_asset_info: AssetInfo,
    amount: Option<Uint128>,
    terraswap_factory_addr: Addr,
    terraswap_max_spread: Option<Decimal>,
) -> StdResult<Response> {
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
                cw20_get_balance(
                    &deps.querier,
                    deps.api.addr_validate(&contract_addr)?,
                    env.contract.address,
                )?,
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
        &deps.querier,
        terraswap_factory_addr,
        &[offer_asset_info.clone(), ask_asset_info],
    )?;

    let offer_asset = TerraswapAsset {
        info: offer_asset_info,
        amount: amount_to_swap,
    };
    let send_msg = asset_into_swap_msg(
        deps.api.addr_validate(&pair_info.contract_addr)?,
        offer_asset,
        terraswap_max_spread,
    )?;

    let response = Response::new()
        .add_message(send_msg)
        .add_attributes(vec![attr("action", "swap"), attr("asset", asset_label)]);

    Ok(response)
}

/// Construct terraswap message in order to swap assets
fn asset_into_swap_msg(
    pair_contract: Addr,
    offer_asset: TerraswapAsset,
    max_spread: Option<Decimal>,
) -> StdResult<CosmosMsg<Empty>> {
    let message = match offer_asset.info.clone() {
        AssetInfo::NativeToken { denom } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_contract.to_string(),
            msg: to_binary(&TerraswapPairExecuteMsg::Swap {
                offer_asset: offer_asset.clone(),
                belief_price: None,
                max_spread,
                to: None,
            })?,
            funds: vec![Coin {
                denom,
                amount: offer_asset.amount,
            }],
        }),
        AssetInfo::Token { contract_addr } => CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: pair_contract.to_string(),
                amount: offer_asset.amount,
                msg: to_binary(&TerraswapPairExecuteMsg::Swap {
                    offer_asset,
                    belief_price: None,
                    max_spread,
                    to: None,
                })?,
            })?,
            funds: vec![],
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
    use cosmwasm_std::SubMsg;

    #[test]
    fn test_cannot_swap_same_assets() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let assets = vec![
            (
                "somecoin_addr",
                AssetInfo::Token {
                    contract_addr: String::from("somecoin_addr"),
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
            let response = execute_swap(
                deps.as_mut(),
                env.clone(),
                asset_info.clone(),
                asset_info,
                None,
                Addr::unchecked("terraswap_factory"),
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
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let cw20_contract_address = Addr::unchecked("cw20_zero");
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(Addr::unchecked(MOCK_CONTRACT_ADDR), Uint128::zero())],
        );

        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.to_string(),
        };
        let ask_asset_info = AssetInfo::NativeToken {
            denom: "uusd".to_string(),
        };

        let response = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            Addr::unchecked("terraswap_factory"),
            None,
        );
        assert_generic_error_message(response, "Contract has no balance for the asset cw20_zero")
    }

    #[test]
    fn test_cannot_swap_more_than_contract_balance() {
        let mut deps = mock_dependencies(&[Coin {
            denom: "somecoin".to_string(),
            amount: Uint128::new(1_000_000),
        }]);
        let env = mock_env(MockEnvParams::default());

        let offer_asset_info = AssetInfo::NativeToken {
            denom: "somecoin".to_string(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: String::from("cw20_token"),
        };

        let response = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128::new(1_000_001)),
            Addr::unchecked("terraswap_factory"),
            None,
        );
        assert_generic_error_message(
            response,
            "The amount requested for swap exceeds contract balance for the asset somecoin",
        )
    }

    #[test]
    fn test_swap_contract_token_partial_balance() {
        let mut deps = mock_dependencies(&[]);
        let env = mock_env(MockEnvParams::default());

        let cw20_contract_address = Addr::unchecked("cw20");
        let contract_asset_balance = Uint128::new(1_000_000);
        deps.querier.set_cw20_balances(
            cw20_contract_address.clone(),
            &[(Addr::unchecked(MOCK_CONTRACT_ADDR), contract_asset_balance)],
        );

        let offer_asset_info = AssetInfo::Token {
            contract_addr: cw20_contract_address.to_string(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: String::from("mars"),
        };

        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: String::from("pair_cw20_mars"),
            liquidity_token: String::from("lp_cw20_mars"),
        });

        let res = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128::new(999)),
            Addr::unchecked("terraswap_factory"),
            None,
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_contract_address.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: String::from("pair_cw20_mars"),
                    amount: Uint128::new(999),
                    msg: to_binary(&TerraswapPairExecuteMsg::Swap {
                        offer_asset: TerraswapAsset {
                            info: AssetInfo::Token {
                                contract_addr: cw20_contract_address.to_string(),
                            },
                            amount: Uint128::new(999),
                        },
                        belief_price: None,
                        max_spread: None,
                        to: None,
                    })
                    .unwrap(),
                })
                .unwrap(),
                funds: vec![],
            }))]
        );

        assert_eq!(
            res.attributes,
            vec![
                attr("action", "swap"),
                attr("asset", cw20_contract_address.as_str()),
            ]
        );
    }

    #[test]
    fn test_swap_native_token_total_balance() {
        let contract_asset_balance = Uint128::new(1_234_567);
        let mut deps = mock_dependencies(&[Coin {
            denom: "uusd".to_string(),
            amount: contract_asset_balance,
        }]);
        let env = mock_env(MockEnvParams::default());

        let offer_asset_info = AssetInfo::NativeToken {
            denom: "uusd".to_string(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: String::from("mars"),
        };

        deps.querier.set_terraswap_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: String::from("pair_uusd_mars"),
            liquidity_token: String::from("lp_uusd_mars"),
        });

        let res = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            Addr::unchecked("terraswap_factory"),
            Some(Decimal::from_ratio(1u128, 100u128)),
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("pair_uusd_mars"),
                msg: to_binary(&TerraswapPairExecuteMsg::Swap {
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
                funds: vec![Coin {
                    denom: "uusd".to_string(),
                    amount: contract_asset_balance,
                }],
            }))]
        );

        assert_eq!(
            res.attributes,
            vec![attr("action", "swap"), attr("asset", "uusd")]
        );
    }
}
