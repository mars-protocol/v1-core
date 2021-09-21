use cosmwasm_std::{Addr, Uint128};
use cw_storage_plus::{Item, Map};

use mars::vesting::Config;
use mars::vesting::{AllocationParams, AllocationStatus};

pub const CONFIG: Item<Config<Addr>> = Item::new("config");
pub const PARAMS: Map<&Addr, AllocationParams> = Map::new("params");
pub const STATUS: Map<&Addr, AllocationStatus> = Map::new("status");
pub const VOTING_POWER_SNAPSHOTS: Map<&Addr, Vec<(u64, Uint128)>> = Map::new("snapshots");
