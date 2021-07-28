pub mod msg {
    use cosmwasm_std::{Binary, Uint128};
    use cw20::{Cw20Coin, Expiration, MinterResponse};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, JsonSchema)]
    pub struct InstantiateMsg {
        // cw20_base params
        pub name: String,
        pub symbol: String,
        pub decimals: u8,
        pub initial_balances: Vec<Cw20Coin>,
        pub mint: Option<MinterResponse>,

        // custom_params
        pub red_bank_address: String,
        pub incentives_address: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Transfer is a base message to move tokens to another account. Requires to be finalized
        /// by the money market.
        Transfer { recipient: String, amount: Uint128 },

        /// Forced transfer called by the money market when an account is being liquidated
        TransferOnLiquidation {
            sender: String,
            recipient: String,
            amount: Uint128,
        },

        /// Burns tokens from user. Only money market can call this.
        /// Used when user is being liquidated
        Burn { user: String, amount: Uint128 },

        /// Send is a base message to transfer tokens to a contract and trigger an action
        /// on the receiving contract.
        Send {
            contract: String,
            amount: Uint128,
            msg: Binary,
        },

        /// Only with the "mintable" extension. If authorized, creates amount new tokens
        /// and adds to the recipient balance.
        Mint { recipient: String, amount: Uint128 },

        /// Only with "approval" extension. Allows spender to access an additional amount tokens
        /// from the owner's (env.sender) account. If expires is Some(), overwrites current allowance
        /// expiration with this one.
        IncreaseAllowance {
            spender: String,
            amount: Uint128,
            expires: Option<Expiration>,
        },
        /// Only with "approval" extension. Lowers the spender's access of tokens
        /// from the owner's (env.sender) account by amount. If expires is Some(), overwrites current
        /// allowance expiration with this one.
        DecreaseAllowance {
            spender: String,
            amount: Uint128,
            expires: Option<Expiration>,
        },
        /// Only with "approval" extension. Transfers amount tokens from owner -> recipient
        /// if `env.sender` has sufficient pre-approval.
        TransferFrom {
            owner: String,
            recipient: String,
            amount: Uint128,
        },
        /// Only with "approval" extension. Sends amount tokens from owner -> contract
        /// if `env.sender` has sufficient pre-approval.
        SendFrom {
            owner: String,
            contract: String,
            amount: Uint128,
            msg: Binary,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Returns the current balance of the given address, 0 if unset.
        /// Return type: BalanceResponse.
        Balance {
            address: String,
        },
        /// Returns both balance (0 if unset) and total supply
        /// Used by incentives contract when computing unclaimed rewards
        /// Return type: BalanceAndTotalSupplyResponse
        BalanceAndTotalSupply {
            address: String,
        },
        /// Returns metadata on the contract - name, decimals, supply, etc.
        /// Return type: TokenInfoResponse.
        TokenInfo {},
        Minter {},
        /// Only with "allowance" extension.
        /// Returns how much spender can use from owner account, 0 if unset.
        /// Return type: AllowanceResponse.
        Allowance {
            owner: String,
            spender: String,
        },
        /// Only with "enumerable" extension (and "allowances")
        /// Returns all allowances this owner has approved. Supports pagination.
        /// Return type: AllAllowancesResponse.
        AllAllowances {
            owner: String,
            start_after: Option<String>,
            limit: Option<u32>,
        },
        /// Only with "enumerable" extension
        /// Returns all accounts that have balances. Supports pagination.
        /// Return type: AllAccountsResponse.
        AllAccounts {
            start_after: Option<String>,
            limit: Option<u32>,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct BalanceAndTotalSupplyResponse {
        pub balance: Uint128,
        pub total_supply: Uint128,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}
