pub mod cw20_token;
pub mod helpers;
pub mod liquidity_pool;
pub mod ma_token;
pub mod storage;
pub mod swapping;
pub mod xmars_token;

#[cfg(not(target_arch = "wasm32"))]
pub mod testing;
