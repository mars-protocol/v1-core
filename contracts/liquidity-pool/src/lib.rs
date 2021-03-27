pub mod contract;
mod mock_querier;
pub mod msg;
pub mod state;

#[cfg(all(target_arch = "wasm32", not(feature = "library")))]
cosmwasm_std::create_entry_points_with_migration!(contract);
