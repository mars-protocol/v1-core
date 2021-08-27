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
        /// Update LP config
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
            amount: Uint128,
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
            user_address: String,
            /// Sends maAsset to liquidator if true and underlying collateral asset if false
            receive_ma_token: bool,
        },

        /// Called by liquidity token. Validate liquidity token transfer is valid
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

        /// Update uncollateralized loan limit
        UpdateUncollateralizedLoanLimit {
            user_address: String,
            asset: Asset,
            new_limit: Uint128,
        },

        /// Update (enable / disable) asset as collateral
        UpdateUserCollateralAssetStatus { asset: Asset, enable: bool },

        /// Distribute protocol income to the treasury, insurance fund, and staking contracts protocol contracts
        DistributeProtocolIncome {
            /// Asset market fees to distribute
            asset: Asset,
            /// Amount to distribute to protocol contracts, defaults to full amount if not specified
            amount: Option<Uint128>,
        },

        /// Withdraw an amount of the asset burning an equivalent amount of maTokens
        Withdraw {
            asset: Asset,
            /// Amount to be withdrawn. If None is specified, the full maToken balance will be
            /// burned in exchange for the equivalent asset amount.
            amount: Option<Uint128>,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ReceiveMsg {
        /// Deposit the sent cw20 tokens
        DepositCw20 {},
        /// Repay the sent cw20 tokens
        RepayCw20 {},
        /// Use the sent cw20 tokens to pay off a specified user's under-collateralized cw20 loan
        LiquidateCw20 {
            /// Details for collateral asset
            collateral_asset: Asset,
            /// Token address of the debt asset
            debt_asset_address: String,
            /// The address of the borrower getting liquidated
            user_address: String,
            /// Sends maAsset to liquidator if true and underlying collateral asset if false
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
        Debt { address: String },
        /// Get all collateral positions for a user. Returns CollateralResponse
        Collateral { address: String },
        /// Get user position. Returns UserPositionResponse
        UserPosition { address: String },
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
        pub amount: Uint128,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralResponse {
        pub collateral: Vec<CollateralInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralInfo {
        /// Either denom for a native token or asset address for a cw20
        pub denom: String,
        /// Scaled amount
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
        /// Max percentage of collateral that can be borrowed
        pub max_loan_to_value: Option<Decimal>,
        /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
        pub reserve_factor: Option<Decimal>,
        /// Percentage at which the loan is defined as under-collateralized
        pub maintenance_margin: Option<Decimal>,
        /// Bonus on the price of assets of the collateral when liquidators purchase it
        pub liquidation_bonus: Option<Decimal>,
        /// Interest rate strategy to calculate borrow_rate and liquidity_rate
        pub interest_rate_strategy: Option<InterestRateStrategy>,
    }
}
