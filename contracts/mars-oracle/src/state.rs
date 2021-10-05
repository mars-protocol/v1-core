use cw_storage_plus::{Item, Map};

use crate::{AstroportTwapData, Config, PriceConfig};

/// Stores config at the given key
pub const CONFIG: Item<Config> = Item::new("config");
pub const PRICE_CONFIGS: Map<&[u8], PriceConfig> = Map::new("price_configs");

pub const ASTROPORT_TWAP_DATA: Map<&[u8], AstroportTwapData> = Map::new("twap_data");
