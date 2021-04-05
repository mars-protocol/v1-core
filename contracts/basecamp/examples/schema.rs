use std::env::current_dir;
use std::fs::create_dir_all;

use basecamp::msg::{ConfigResponse, HandleMsg, InitMsg, MigrateMsg, QueryMsg};
use cosmwasm_schema::{export_schema, remove_schemas, schema_for};

use basecamp::state::Config;

fn main() {
    let mut out_dir = current_dir().unwrap();
    out_dir.push("schema");
    create_dir_all(&out_dir).unwrap();
    remove_schemas(&out_dir).unwrap();

    export_schema(&schema_for!(InitMsg), &out_dir);
    export_schema(&schema_for!(HandleMsg), &out_dir);
    export_schema(&schema_for!(QueryMsg), &out_dir);
    export_schema(&schema_for!(Config), &out_dir);
    export_schema(&schema_for!(ConfigResponse), &out_dir);
    export_schema(&schema_for!(MigrateMsg), &out_dir);
}
