use crate::helpers::cw20_get_balance;

use cosmwasm_std::{
    attr, to_binary, Addr, Coin, CosmosMsg, Decimal as StdDecimal, DepsMut, Empty, Env, Response,
    StdError, StdResult, Uint128, WasmMsg,
};
use cw20::Cw20ExecuteMsg;
use terraswap::asset::{AssetInfo, PairInfo};

/// Swap assets via Astroport
pub fn execute_swap(
    deps: DepsMut,
    env: Env,
    offer_asset_info: AssetInfo,
    ask_asset_info: AssetInfo,
    amount: Option<Uint128>,
    astroport_factory_addr: Addr,
    astroport_max_spread: Option<StdDecimal>,
) -> StdResult<Response> {
    panic!("#238");
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
                    contract_addr: Addr::unchecked("somecoin_addr"),
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
                Addr::unchecked("astroport_factory"),
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
            contract_addr: cw20_contract_address,
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
            Addr::unchecked("astroport_factory"),
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
            contract_addr: Addr::unchecked("cw20_token"),
        };

        let response = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128::new(1_000_001)),
            Addr::unchecked("astroport_factory"),
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
            contract_addr: cw20_contract_address.clone(),
        };
        let ask_asset_info = AssetInfo::Token {
            contract_addr: Addr::unchecked("mars"),
        };

        deps.querier.set_astroport_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: Addr::unchecked("pair_cw20_mars"),
            liquidity_token: Addr::unchecked("lp_cw20_mars"),
            pair_type: PairType::Xyk {},
        });

        let res = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            Some(Uint128::new(999)),
            Addr::unchecked("astroport_factory"),
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
                    msg: to_binary(&AstroportPairExecuteMsg::Swap {
                        offer_asset: AstroportAsset {
                            info: AssetInfo::Token {
                                contract_addr: cw20_contract_address.clone(),
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
                attr("offer_asset", cw20_contract_address.as_str()),
                attr("ask_asset", "mars"),
                attr("offer_asset_amount", "999"),
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
            contract_addr: Addr::unchecked("mars"),
        };

        deps.querier.set_astroport_pair(PairInfo {
            asset_infos: [offer_asset_info.clone(), ask_asset_info.clone()],
            contract_addr: Addr::unchecked("pair_uusd_mars"),
            liquidity_token: Addr::unchecked("lp_uusd_mars"),
            pair_type: PairType::Xyk {},
        });

        let res = execute_swap(
            deps.as_mut(),
            env,
            offer_asset_info,
            ask_asset_info,
            None,
            Addr::unchecked("astroport_factory"),
            Some(StdDecimal::from_ratio(1u128, 100u128)),
        )
        .unwrap();

        assert_eq!(
            res.messages,
            vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: String::from("pair_uusd_mars"),
                msg: to_binary(&AstroportPairExecuteMsg::Swap {
                    offer_asset: AstroportAsset {
                        info: AssetInfo::NativeToken {
                            denom: "uusd".to_string(),
                        },
                        amount: contract_asset_balance,
                    },
                    belief_price: None,
                    max_spread: Some(StdDecimal::from_ratio(1u128, 100u128)),
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
            vec![
                attr("action", "swap"),
                attr("offer_asset", "uusd"),
                attr("ask_asset", "mars"),
                attr("offer_asset_amount", contract_asset_balance.to_string()),
            ]
        );
    }
}
