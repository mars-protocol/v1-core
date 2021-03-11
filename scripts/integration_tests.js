import { LocalEnv } from "./deploy_local.mjs";

async function test_reserve_query(address, symbol) {
  let query_msg = {"reserve": {"symbol": symbol}};
  let result = await local.query_contract(address, query_msg);

  if (!result.hasOwnProperty("ma_token_address")) {
    throw new Error("Reserve Query for symbol {symbol} failed")
  }

  console.log(`Reserve Query for symbol ${symbol}:`);
  console.log(result);
}

async function test_config_query(address) {
  let query_msg = {"config": {}};
  let result = await local.query_contract(address, query_msg);

  if (!result.hasOwnProperty("ma_token_code_id")) {
    throw new Error("Config query failed")
  }

  console.log("Config Query:");
  console.log(result);
}

const local = new LocalEnv();
const contract_address = await local.deploy_local();
await test_reserve_query(contract_address, "usd")
await test_reserve_query(contract_address, "luna");
await test_config_query(contract_address);
