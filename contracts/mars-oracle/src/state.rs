use cw_storage_plus::{Item, Map};

use crate::{AstroportTwapSnapshot, Config, PriceConfig};

pub const CONFIG: Item<Config> = Item::new("config");
pub const PRICE_CONFIGS: Map<&[u8], PriceConfig> = Map::new("price_configs");
pub const ASTROPORT_TWAP_SNAPSHOTS: Map<&[u8], Vec<AstroportTwapSnapshot>> = Map::new("snapshots");
