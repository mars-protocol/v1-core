/// state: contains token state data structures not included in cw20_base
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::Addr;
use cw_storage_plus::Item;

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Config {
    pub red_bank_address: Addr,
    pub incentives_address: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");
