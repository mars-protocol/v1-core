use cosmwasm_std::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserHealthStatus {
    NotBorrowing,
    Borrowing(Decimal),
}

pub mod msg {
    use super::UserHealthStatus;
    use crate::asset::{Asset, AssetType};
    use crate::interest_rate_models::InterestRateStrategy;
    use cosmwasm_std::{Addr, Decimal, Uint128};
    use cw20::Cw20ReceiveMsg;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InstantiateMsg {
        pub config: CreateOrUpdateConfig,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CreateOrUpdateConfig {
        pub owner: Option<String>,
        pub address_provider_address: Option<String>,
        pub insurance_fund_fee_share: Option<Decimal>,
        pub treasury_fee_share: Option<Decimal>,
        pub ma_token_code_id: Option<u64>,
        pub close_factor: Option<Decimal>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Update contract config
        UpdateConfig { config: CreateOrUpdateConfig },

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
            /// in bytes
            reference: Vec<u8>,
        },

        /// Deposit Terra native coins. Deposited coins must be sent in the transaction
        /// this call is made
        DepositNative {
            /// Denom used in Terra (e.g: uluna, uusd)
            denom: String,
        },

        /// Borrow Terra native coins. If borrow allowed, amount is added to caller's debt
        /// and sent to the address. If asset is a Terra native token, the amount sent
        /// is selected so that the sum of the transfered amount plus the stability tax
        /// payed is equal to the borrowed amount.
        Borrow {
            /// Asset to borrow
            asset: Asset,
            /// Amount to borrow
            amount: Uint128,
        },

        /// Repay Terra native coins loan. Coins used to repay must be sent in the
        /// transaction this call is made.
        RepayNative {
            /// Denom used in Terra (e.g: uluna, uusd)
            denom: String,
        },

        /// Liquidate under-collateralized native loans. Coins used to repay must be sent in the
        /// transaction this call is made.
        LiquidateNative {
            /// Collateral asset liquidator gets from the borrower
            collateral_asset: Asset,
            /// Denom used in Terra (e.g: uluna, uusd) of the debt asset
            debt_asset_denom: String,
            /// The address of the borrower getting liquidated
            user_address: String,
            /// Whether the liquidator gets liquidated collateral in maToken (true) or
            /// the underlying collateral asset (false)
            receive_ma_token: bool,
        },

        /// Called by liquidity token (maToken). Validate liquidity token transfer is valid
        /// and update collateral status
        FinalizeLiquidityTokenTransfer {
            /// Token sender. Address is trusted because it should have been verified in
            /// the token contract
            sender_address: Addr,
            /// Token recipient. Address is trusted because it should have been verified in
            /// the token contract
            recipient_address: Addr,
            /// Sender's balance before the token transfer
            sender_previous_balance: Uint128,
            /// Recipient's balance before the token transfer
            recipient_previous_balance: Uint128,
            /// Transfer amount
            amount: Uint128,
        },

        /// Update uncollateralized loan limit for a given user and asset.
        /// Overrides previous value if any. A limit of zero means no
        /// uncollateralized limit and the debt in that asset needs to be
        /// collateralized.
        UpdateUncollateralizedLoanLimit {
            /// Address that receives the credit
            user_address: String,
            /// Asset the user receives the credit in
            asset: Asset,
            /// Limit for the uncolateralize loan.
            new_limit: Uint128,
        },

        /// Update (enable / disable) asset as collateral for the caller
        UpdateUserCollateralAssetStatus {
            /// Asset to update status for
            asset: Asset,
            /// Option to enable (true) / disable (false) asset as collateral
            enable: bool,
        },

        /// Distribute the accrued protocol income to the treasury, insurance fund, and staking contracts
        /// according to the split set in config.
        /// Will transfer underlying asset to insurance fund and staking while minting maTokens to
        /// the treasury.
        /// Callable by any address, will fail if red bank has no liquidity.
        DistributeProtocolIncome {
            /// Asset market fees to distribute
            asset: Asset,
            /// Amount to distribute to protocol contracts, defaults to full amount if not specified
            amount: Option<Uint128>,
        },

        /// Withdraw an amount of the asset burning an equivalent amount of maTokens.
        /// If asset is a Terra native token, the amount sent to the user
        /// is selected so that the sum of the transfered amount plus the stability tax
        /// payed is equal to the withdrawn amount.
        Withdraw {
            /// Asset to withdraw
            asset: Asset,
            /// Amount to be withdrawn. If None is specified, the full maToken balance will be
            /// burned in exchange for the equivalent asset amount.
            amount: Option<Uint128>,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReceiveMsg {
        /// Deposit sent cw20 tokens
        DepositCw20 {},
        /// Repay sent cw20 tokens
        RepayCw20 {},
        /// Liquidate under-collateralized cw20 loan using the sent cw20 tokens.
        LiquidateCw20 {
            /// Collateral asset liquidator gets from the borrower
            collateral_asset: Asset,
            /// The address of the borrower getting liquidated
            user_address: String,
            /// Whether the liquidator gets liquidated collateral in maToken (true) or
            /// the underlying collateral asset (false)
            receive_ma_token: bool,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Get config parameters
        Config {},
        /// Get asset market parameters
        Market { asset: Asset },
        /// Get a list of all markets. Returns MarketsListResponse
        MarketsList {},
        /// Get uncollateralized limit for given asset and user.
        /// Returns UncollateralizedLoanLimitResponse
        UncollateralizedLoanLimit { user_address: String, asset: Asset },
        /// Get all debt positions for a user. Returns DebtResponse
        UserDebt { user_address: String },
        /// Get info about whether or not user is using each asset as collateral.
        /// Returns CollateralResponse
        UserCollateral { user_address: String },
        /// Get user position. Returns UserPositionResponse
        UserPosition { user_address: String },
        /// Get equivalent underlying asset amount for a maToken balance. Returns AmountResponse
        ScaledLiquidityAmount { asset: Asset, amount: Uint128 },
        /// Get equivalent underlying asset amount for a debt balance. Returns AmountResponse
        ScaledDebtAmount { asset: Asset, amount: Uint128 },
        /// Get equivalent maToken amount for a given underlying asset balance. Returns AmountResponse
        DescaledLiquidityAmount {
            ma_token_address: String,
            amount: Uint128,
        },
    }

    // We define a custom struct for each query response
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: Addr,
        pub address_provider_address: Addr,
        pub insurance_fund_fee_share: Decimal,
        pub treasury_fee_share: Decimal,
        pub ma_token_code_id: u64,
        pub market_count: u32,
        pub close_factor: Decimal,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketResponse {
        pub ma_token_address: Addr,
        pub borrow_index: Decimal,
        pub liquidity_index: Decimal,
        pub borrow_rate: Decimal,
        pub liquidity_rate: Decimal,
        pub max_loan_to_value: Decimal,
        pub interests_last_updated: u64,
        pub debt_total_scaled: Uint128,
        pub asset_type: AssetType,
        pub maintenance_margin: Decimal,
        pub liquidation_bonus: Decimal,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketsListResponse {
        pub markets_list: Vec<MarketInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketInfo {
        /// Either denom for a native token or asset address for a cw20
        pub denom: String,
        /// Address for the corresponding maToken
        pub ma_token_address: Addr,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct DebtResponse {
        pub debts: Vec<DebtInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct DebtInfo {
        /// Either denom for a native token or asset address for a cw20
        pub denom: String,
        /// Scaled amount
        pub amount_scaled: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralResponse {
        pub collateral: Vec<CollateralInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralInfo {
        /// Either denom for a native token or asset address for a cw20
        pub denom: String,
        /// Wether the user is using asset as collateral or not
        pub enabled: bool,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct UncollateralizedLoanLimitResponse {
        /// Limit an address has for an uncollateralized loan for a specific asset.
        /// 0 limit means no collateral.
        pub limit: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct AmountResponse {
        pub amount: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct UserPositionResponse {
        pub total_collateral_in_uusd: Uint128,
        pub total_debt_in_uusd: Uint128,
        /// Total debt minus the uncollateralized debt
        pub total_collateralized_debt_in_uusd: Uint128,
        pub max_debt_in_uusd: Uint128,
        pub weighted_maintenance_margin_in_uusd: Uint128,
        pub health_status: UserHealthStatus,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitOrUpdateAssetParams {
        /// Initial borrow rate
        pub initial_borrow_rate: Option<Decimal>,

        /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
        pub reserve_factor: Option<Decimal>,
        /// Max percentage of collateral that can be borrowed
        pub max_loan_to_value: Option<Decimal>,
        /// Percentage at which the loan is defined as under-collateralized
        pub maintenance_margin: Option<Decimal>,
        /// Bonus on the price of assets of the collateral when liquidators purchase it
        pub liquidation_bonus: Option<Decimal>,

        /// Interest rate strategy to calculate borrow_rate and liquidity_rate
        pub interest_rate_strategy: Option<InterestRateStrategy>,
    }
}
