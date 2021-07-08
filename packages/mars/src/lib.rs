pub mod address_provider;
pub mod cw20_token;
pub mod helpers;
pub mod incentives;
pub mod ma_token;
pub mod red_bank;
pub mod staking;
pub mod storage;
pub mod swapping;
pub mod xmars_token;

#[cfg(not(target_arch = "wasm32"))]
pub mod testing;
