use cosmwasm_std::{Addr, Decimal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource<A> {
    /// Returns a fixed value; used for UST
    Fixed { price: Decimal },
    /// Native Terra stablecoins transaction rate quoted in UST
    Native { denom: String },
    /// TerraSwap spot price quoted in the other asset of the pair
    Spot { pair_address: A, asset_address: A },
    /// Astroport TWAP price quoted in the other asset of the pair
    Twap {
        pair_address: A,
        asset_address: A,
        period: u64,
    },
}

pub type PriceSourceUnchecked = PriceSource<String>;
pub type PriceSourceChecked = PriceSource<Addr>;

pub mod msg {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use cosmwasm_std::Decimal;

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
        SetConfig { owner: Option<String> },
        /// Specify parameters to query asset price
        SetAsset {
            asset: Asset,
            price_source: PriceSourceUnchecked,
        },
        /// Fetch cumulative price from Astroport pair and record in contract storage
        UpdateTwapData { assets: Vec<Asset> },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum QueryMsg {
        /// Query contract config
        Config {},
        /// Get asset's price config
        AssetConfig { asset: Asset },
        /// Query asset price given an asset
        AssetPrice { asset: Asset },
        /// Query asset price given it's internal reference
        /// (meant to be used by protocol contracts only)
        AssetPriceByReference { asset_reference: Vec<u8> },
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct ConfigResponse {
        pub owner: String,
    }

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
    pub struct AssetPriceResponse {
        pub price: Decimal,
        pub last_updated: u64,
    }
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
                msg: to_binary(&QueryMsg::AssetPriceByReference { asset_reference })?,
            }))?
        };

        Ok(query)
    }
}
