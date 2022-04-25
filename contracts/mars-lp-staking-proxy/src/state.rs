use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{Addr, Decimal};
use cw_storage_plus::Item;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub redbank_addr: Addr,
    pub astro_generator_addr: Addr,
    pub token_addr: Addr,
    pub ma_token_addr: Option<Addr>,
    pub pool_addr: Addr,
    pub astro_token_addr: Addr,
    pub astro_treasury_fee: Decimal,
    pub proxy_token_reward_addr: Option<Addr>,
    pub proxy_token_treasury_fee: Decimal,
}

pub const CONFIG: Item<Config> = Item::new("config");
