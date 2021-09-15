use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct AssetConfig {
    pub enabled_for_distribution: bool,
}

impl Default for AssetConfig {
    fn default() -> Self {
        AssetConfig {
            enabled_for_distribution: false,
        }
    }
}
