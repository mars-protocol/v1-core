use cosmwasm_std::{Addr, Decimal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource<A> {
    Native { denom: String },
    TerraswapUusdPair { pair_address: A },
    Fixed { price: Decimal },
}

pub type PriceSourceUnchecked = PriceSource<String>;
pub type PriceSourceChecked = PriceSource<Addr>;

pub mod msg {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::PriceSourceUnchecked;
    use crate::asset::Asset;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct InstantiateMsg {
        pub owner: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum ExecuteMsg {
        /// Update contract config
        UpdateConfig { owner: Option<String> },

        /// Specify parameters to query asset price
        SetAsset {
            asset: Asset,
            price_source: PriceSourceUnchecked,
        },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Query contract config
        Config {},
        /// Query asset price given it's internal reference
        /// (meant to be used by protocol contracts only)
        AssetPrice { asset_reference: Vec<u8> },
        /// Get asset's price config
        AssetPriceConfig { asset: Asset },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: String,
    }

    /// We currently take no arguments for migrations
    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct MigrateMsg {}
}

pub mod helpers {
    use cosmwasm_std::{
        to_binary, Addr, Decimal, QuerierWrapper, QueryRequest, StdResult, WasmQuery,
    };

    use crate::asset::AssetType;

    use super::msg::QueryMsg;

    pub fn query_price(
        querier: QuerierWrapper,
        oracle_address: Addr,
        asset_label: &str,
        asset_reference: Vec<u8>,
        asset_type: AssetType,
    ) -> StdResult<Decimal> {
        let query: Decimal = if asset_type == AssetType::Native && asset_label == "uusd" {
            Decimal::one()
        } else {
            querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                contract_addr: oracle_address.into(),
                msg: to_binary(&QueryMsg::AssetPrice { asset_reference })?,
            }))?
        };

        Ok(query)
    }
}
