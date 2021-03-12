import {LocalTerra} from "@terra-money/terra.js";
import { deploy_local } from "./deploy_local.mjs";
import { query_contract } from "./helpers.mjs";

async function test_reserve_query(terra, address, symbol) {
  let query_msg = {"reserve": {"symbol": symbol}};
  let result = await query_contract(terra, address, query_msg);

  if (!result.hasOwnProperty("ma_token_address")) {
    throw new Error("Reserve Query for symbol {symbol} failed")
  }

  console.log(`Reserve Query for symbol ${symbol}:`);
  console.log(result);
}

async function test_config_query(terra, address) {
  let query_msg = {"config": {}};
  let result = await query_contract(terra, address, query_msg);

  if (!result.hasOwnProperty("ma_token_code_id")) {
    throw new Error("Config query failed")
  }

  console.log("Config Query:");
  console.log(result);
}

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
const contract_address = await deploy_local(terra, wallet);

await test_reserve_query(terra, contract_address, "usd")
await test_reserve_query(terra, contract_address, "luna");
await test_config_query(terra, contract_address);
