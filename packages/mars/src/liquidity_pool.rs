pub mod msg {
    use cosmwasm_bignumber::{Decimal256, Uint256};
    use cosmwasm_std::{HumanAddr, StdError, StdResult, Uint128};
    use cw20::Cw20ReceiveMsg;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitMsg {
        pub treasury_contract_address: HumanAddr,
        pub insurance_fund_contract_address: HumanAddr,
        pub staking_contract_address: HumanAddr,
        pub insurance_fund_fee_share: Decimal256,
        pub treasury_fee_share: Decimal256,
        pub ma_token_code_id: u64,
        pub close_factor: Decimal256,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum HandleMsg {
        /// Implementation of cw20 receive msg
        Receive(Cw20ReceiveMsg),

        /// Initialize an asset on the money market
        InitAsset {
            /// Asset related info
            asset: Asset,
            /// Asset parameters
            asset_params: InitOrUpdateAssetParams,
        },
        /// Update an asset on the money market
        UpdateAsset {
            /// Asset related info
            asset: Asset,
            /// Asset parameters
            asset_params: InitOrUpdateAssetParams,
        },
        /// Callback sent from maToken contract after instantiated
        InitAssetTokenCallback {
            /// Either the denom for a terra native asset or address for a cw20 token
            reference: Vec<u8>,
        },
        /// Deposit Terra native coins
        DepositNative {
            /// Denom used in Terra (e.g: uluna, uusd)
            denom: String,
        },
        /// Borrow Terra native coins
        Borrow {
            /// Denom used in Terra (e.g: uluna, uusd)
            asset: Asset,
            amount: Uint256,
        },
        /// Repay Terra native coins loan
        RepayNative {
            /// Denom used in Terra (e.g: uluna, uusd)
            denom: String,
        },
        /// Liquidate under-collateralized native loans
        LiquidateNative {
            /// Details for collateral asset
            collateral_asset: Asset,
            /// Denom used in Terra (e.g: uluna, uusd) of the debt asset
            debt_asset: String,
            /// The address of the borrower getting liquidated
            user_address: HumanAddr,
            /// Sends maAsset to liquidator if true and underlying collateral asset if false
            receive_ma_token: bool,
        },
        /// Called by liquidity token. Validate liquidity token transfer is valid
        /// and update collateral status
        FinalizeLiquidityTokenTransfer {
            sender_address: HumanAddr,
            recipient_address: HumanAddr,
            sender_previous_balance: Uint128,
            recipient_previous_balance: Uint128,
            amount: Uint128,
        },
        /// Update uncollateralized loan limit
        UpdateUncollateralizedLoanLimit {
            user_address: HumanAddr,
            asset: Asset,
            new_limit: Uint128,
        },
        /// Update (enable / disable) asset as collateral
        UpdateUserCollateralAssetStatus { asset: Asset, enable: bool },
        /// Distribute protocol income to the treasury, insurance fund, and staking contracts protocol contracts
        DistributeProtocolIncome {
            /// Asset reserve fees to distribute
            asset: Asset,
            /// Amount to distribute to protocol contracts, defaults to full amount if not specified
            amount: Option<Uint256>,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReceiveMsg {
        /// Redeem the sent tokens for the underlying asset
        Redeem {},
        /// Deposit the sent cw20 tokens
        DepositCw20 {},
        /// Repay the sent cw20 tokens
        RepayCw20 {},
        /// Use the sent cw20 tokens to pay off a specified user's under-collateralized cw20 loan
        LiquidateCw20 {
            /// Details for collateral asset
            collateral_asset: Asset,
            /// Token address of the debt asset
            debt_asset_address: HumanAddr,
            /// The address of the borrower getting liquidated
            user_address: HumanAddr,
            /// Sends maAsset to liquidator if true and underlying collateral asset if false
            receive_ma_token: bool,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        Config {},
        Reserve { asset: Asset },
        ReservesList {},
        Debt { address: HumanAddr },
    }

    // We define a custom struct for each query response
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: HumanAddr,
        pub treasury_contract_address: HumanAddr,
        pub insurance_fund_contract_address: HumanAddr,
        pub insurance_fund_fee_share: Decimal256,
        pub treasury_fee_share: Decimal256,
        pub ma_token_code_id: u64,
        pub reserve_count: u32,
        pub close_factor: Decimal256,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ReserveResponse {
        pub ma_token_address: HumanAddr,
        pub borrow_index: Decimal256,
        pub liquidity_index: Decimal256,
        pub borrow_rate: Decimal256,
        pub liquidity_rate: Decimal256,
        pub borrow_slope: Decimal256,
        pub loan_to_value: Decimal256,
        pub interests_last_updated: u64,
        pub debt_total_scaled: Uint256,
        pub asset_type: AssetType,
        pub liquidation_threshold: Decimal256,
        pub liquidation_bonus: Decimal256,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ReservesListResponse {
        pub reserves_list: Vec<ReserveInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ReserveInfo {
        pub denom: String,
        pub ma_token_address: HumanAddr,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct DebtResponse {
        pub debts: Vec<DebtInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct DebtInfo {
        pub denom: String,
        pub amount: Uint256,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitOrUpdateAssetParams {
        /// Borrow slope to calculate borrow rate
        pub borrow_slope: Option<Decimal256>,
        /// Max percentage of collateral that can be borrowed
        pub loan_to_value: Option<Decimal256>,
        /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
        pub reserve_factor: Option<Decimal256>,
        /// Percentage at which the loan is defined as under-collateralized
        pub liquidation_threshold: Option<Decimal256>,
        /// Bonus on the price of assets of the collateral when liquidators purchase it
        pub liquidation_bonus: Option<Decimal256>,
    }

    impl InitOrUpdateAssetParams {
        /// Validate availability of all params. Function used during initialization.
        pub fn validate_availability_of_all_params(&self) -> StdResult<()> {
            // Destructuring a struct’s fields into separate variables in order to force
            // compile error if we add more params
            let InitOrUpdateAssetParams {
                borrow_slope,
                loan_to_value,
                reserve_factor,
                liquidation_threshold,
                liquidation_bonus,
            } = self;

            // All fields should be available
            let available = borrow_slope.is_some()
                && loan_to_value.is_some()
                && reserve_factor.is_some()
                && liquidation_threshold.is_some()
                && liquidation_bonus.is_some();

            if !available {
                Err(StdError::generic_err(
                    "All params should be available during initialization",
                ))
            } else {
                Ok(())
            }
        }

        /// Validate params used during initialization.
        pub fn validate_for_initialization(&self) -> StdResult<()> {
            self.validate(Decimal256::zero(), Decimal256::zero())
        }

        /// Validate params used during update.
        pub fn validate_for_update(
            &self,
            old_ltv: Decimal256,
            old_liquidation_threshold: Decimal256,
        ) -> StdResult<()> {
            self.validate(old_ltv, old_liquidation_threshold)
        }

        fn validate(
            &self,
            old_ltv: Decimal256,
            old_liquidation_threshold: Decimal256,
        ) -> StdResult<()> {
            // Destructuring a struct’s fields into separate variables in order to force
            // compile error if we add more params
            let InitOrUpdateAssetParams {
                borrow_slope: _,
                loan_to_value,
                reserve_factor,
                liquidation_threshold,
                liquidation_bonus,
            } = self;

            // loan_to_value, reserve_factor, liquidation_threshold and liquidation_bonus should be less or equal 1
            let conditions_and_names = vec![
                (Self::less_or_equal_one(loan_to_value), "loan_to_value"),
                (Self::less_or_equal_one(reserve_factor), "reserve_factor"),
                (
                    Self::less_or_equal_one(liquidation_threshold),
                    "liquidation_threshold",
                ),
                (
                    Self::less_or_equal_one(liquidation_bonus),
                    "liquidation_bonus",
                ),
            ];
            // Filter params which don't meet criteria
            let invalid_params: Vec<_> = conditions_and_names
                .into_iter()
                .filter(|elem| !elem.0)
                .map(|elem| elem.1)
                .collect();
            if !invalid_params.is_empty() {
                return Err(StdError::generic_err(format!(
                    "loan_to_value, reserve_factor, liquidation_threshold and liquidation_bonus should be less or equal 1. \
                    Invalid params: [{}]",
                    invalid_params.join(", ")
                )));
            }

            // liquidation_threshold should be greater than loan_to_value
            let new_ltv = loan_to_value.unwrap_or(old_ltv);
            let new_liquidation_threshold =
                liquidation_threshold.unwrap_or(old_liquidation_threshold);
            if new_liquidation_threshold <= new_ltv {
                return Err(StdError::generic_err(format!(
                    "liquidation_threshold should be greater than loan_to_value. \
                    old_liquidation_threshold: {}, \
                    old_loan_to_value: {}, \
                    new_liquidation_threshold: {}, \
                    new_loan_to_value: {}",
                    old_liquidation_threshold, old_ltv, new_liquidation_threshold, new_ltv
                )));
            }

            Ok(())
        }

        fn less_or_equal_one(value: &Option<Decimal256>) -> bool {
            value.unwrap_or(Decimal256::zero()).le(&Decimal256::one())
        }
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Asset {
        Cw20 { contract_addr: HumanAddr },
        Native { denom: String },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum AssetType {
        Cw20,
        Native,
    }
}
