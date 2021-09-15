use mars::helpers::all_conditions_valid;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal, StdError, StdResult};
use cw_storage_plus::{Item, Map};

use crate::types::AssetConfig;

pub const CONFIG: Item<Config> = Item::new("config");
pub const ASSET_CONFIG: Map<&[u8], AssetConfig> = Map::new("assets");

/// Global configuration
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    /// Contract owner
    pub owner: Addr,
    /// Address provider returns addresses for all protocol contracts
    pub address_provider_address: Addr,
    /// Percentage of fees that are sent to the safety fund
    pub safety_fund_fee_share: Decimal,
    /// Percentage of fees that are sent to the treasury
    pub treasury_fee_share: Decimal,
    /// Terraswap factory contract address
    pub terraswap_factory_address: Addr,
    /// Terraswap max spread
    pub terraswap_max_spread: Decimal,
}

impl Config {
    pub fn validate(&self) -> StdResult<()> {
        let conditions_and_names = vec![
            (
                Self::less_or_equal_one(&self.safety_fund_fee_share),
                "safety_fund_fee_share",
            ),
            (
                Self::less_or_equal_one(&self.treasury_fee_share),
                "treasury_fee_share",
            ),
        ];
        all_conditions_valid(conditions_and_names)?;

        let combined_fee_share = self.safety_fund_fee_share + self.treasury_fee_share;
        // Combined fee shares cannot exceed one
        if combined_fee_share > Decimal::one() {
            return Err(StdError::generic_err(
                "Invalid fee share amounts. Sum of safety fund and treasury fee shares exceeds one",
            ));
        }

        Ok(())
    }

    fn less_or_equal_one(value: &Decimal) -> bool {
        value.le(&Decimal::one())
    }
}
