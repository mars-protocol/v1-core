pub mod msg {
    use cosmwasm_bignumber::{Decimal256, Uint256};
    use cosmwasm_std::{Addr, Uint128};
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
        pub insurance_fund_fee_share: Option<Decimal256>,
        pub treasury_fee_share: Option<Decimal256>,
        pub ma_token_code_id: Option<u64>,
        pub close_factor: Option<Decimal256>,
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
            user_address: String,
            /// Sends maAsset to liquidator if true and underlying collateral asset if false
            receive_ma_token: bool,
        },

        /// Called by liquidity token. Validate liquidity token transfer is valid
        /// and update collateral status
        FinalizeLiquidityTokenTransfer {
            sender_address: String,
            recipient_address: String,
            sender_previous_balance: Uint128,
            recipient_previous_balance: Uint128,
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
            amount: Option<Uint256>,
        },

        /// Withdraw asset
        Withdraw {
            asset: Asset,
            amount: Option<Uint256>,
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
        Config {},
        Market { asset: Asset },
        MarketsList {},
        Debt { address: String },
        UncollateralizedLoanLimit { user_address: String, asset: Asset },
        Collateral { address: String },
    }

    // We define a custom struct for each query response
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: Addr,
        pub address_provider_address: Addr,
        pub insurance_fund_fee_share: Decimal256,
        pub treasury_fee_share: Decimal256,
        pub ma_token_code_id: u64,
        pub market_count: u32,
        pub close_factor: Decimal256,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketResponse {
        pub ma_token_address: Addr,
        pub borrow_index: Decimal256,
        pub liquidity_index: Decimal256,
        pub borrow_rate: Decimal256,
        pub liquidity_rate: Decimal256,
        pub max_loan_to_value: Decimal256,
        pub interests_last_updated: u64,
        pub debt_total_scaled: Uint256,
        pub asset_type: AssetType,
        pub maintenance_margin: Decimal256,
        pub liquidation_bonus: Decimal256,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketsListResponse {
        pub markets_list: Vec<MarketInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MarketInfo {
        pub denom: String,
        pub ma_token_address: Addr,
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

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralResponse {
        pub collateral: Vec<CollateralInfo>,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct CollateralInfo {
        pub denom: String,
        pub enabled: bool,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct UncollateralizedLoanLimitResponse {
        pub limit: Uint128,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InitOrUpdateAssetParams {
        /// Initial borrow rate
        pub initial_borrow_rate: Option<Decimal256>,
        /// Min borrow rate
        pub min_borrow_rate: Option<Decimal256>,
        /// Max borrow rate
        pub max_borrow_rate: Option<Decimal256>,
        /// Max percentage of collateral that can be borrowed
        pub max_loan_to_value: Option<Decimal256>,
        /// Portion of the borrow rate that is sent to the treasury, insurance fund, and rewards
        pub reserve_factor: Option<Decimal256>,
        /// Percentage at which the loan is defined as under-collateralized
        pub maintenance_margin: Option<Decimal256>,
        /// Bonus on the price of assets of the collateral when liquidators purchase it
        pub liquidation_bonus: Option<Decimal256>,
        /// Proportional parameter for the PID controller
        pub kp_1: Option<Decimal256>,
        /// Optimal utilization
        pub optimal_utilization_rate: Option<Decimal256>,
        /// Min error that triggers Kp augmentation
        pub kp_augmentation_threshold: Option<Decimal256>,
        /// Kp value when error threshold is exceeded
        pub kp_2: Option<Decimal256>,
    }

    /// Represents either a native asset or a cw20. Meant to be used as part of a msg
    /// in a contract call and not to be used internally
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Asset {
        Cw20 { contract_addr: String },
        Native { denom: String },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum AssetType {
        Cw20,
        Native,
    }
}
