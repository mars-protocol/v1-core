import { Helpers } from "./helpers.mjs";
import { LocalTerra } from "@terra-money/terra.js";

async function deploy() {
  const lp_code_id = await helpers.upload_contract(test1, '../artifacts/liquidity_pool.wasm');
  const ma_code_id = await helpers.upload_contract(test1, '../artifacts/ma_token.wasm');
  console.log("LP Code ID: " + lp_code_id);
  console.log("MA Code ID: " + ma_code_id);
  const lp_init_msg = {"ma_token_code_id": ma_code_id};
  const lp_contract_address = await helpers.instantiate_contract(test1, lp_code_id, lp_init_msg);
  console.log("LP contract_address: " + lp_contract_address);
  const lp_luna_execute_msg = {"init_asset": {"symbol": "luna"}};
  const lp_usd_execute_msg = {"init_asset": {"symbol": "usd"}};
  let luna_result = await helpers.execute_contract(test1, lp_contract_address, lp_luna_execute_msg);
  let usd_result = await helpers.execute_contract(test1, lp_contract_address, lp_usd_execute_msg);
  console.log("Luna result: " + luna_result);
  console.log("USD result: " + usd_result);
  let query_reserve = await helpers.query_contract(lp_contract_address, {"query_reserve": {"symbol":"usd"}});
  console.log(query_reserve);
}

const terra = new LocalTerra();
const test1 = terra.wallets.test1;
let helpers = new Helpers(terra);
deploy();
