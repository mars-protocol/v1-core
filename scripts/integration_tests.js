import {Coin, LocalTerra, MsgExecuteContract} from "@terra-money/terra.js";
import {deploy, perform_transaction, query_contract} from "./helpers.mjs";

async function test_reserve_query(terra, address, symbol) {
  console.log("### Testing Reserve...")
  let query_msg = {"reserve": {"symbol": symbol}};
  let result = await query_contract(terra, address, query_msg);

  if (!result.hasOwnProperty("ma_token_address")) {
    throw new Error(`Reserve Query for symbol ${symbol} failed`)
  }

  console.log(`Reserve Query for symbol ${symbol}:`);
  console.log(result);
}

async function test_config_query(terra, address) {
  console.log("### Testing Config...")
  let query_msg = {"config": {}};
  let result = await query_contract(terra, address, query_msg);

  if (!result.hasOwnProperty("ma_token_code_id")) {
    throw new Error("Config query failed. Result has no property ma_token_code_id.")
  }

  console.log("Config Query:");
  console.log(result);
}

async function test_deposit(terra, wallet, contract_address) {
  console.log("### Testing Deposit...")
  const deposit_msg = {"deposit_native": {"symbol": "luna"}};
  const coins = new Coin("uluna", 1000);
  const execute_msg = new MsgExecuteContract(wallet.key.accAddress, contract_address, deposit_msg, [coins]);
  await perform_transaction(terra, wallet, execute_msg);

  let reserve_query_msg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await query_contract(terra, contract_address, reserve_query_msg);

  const balance_query_msg = {"balance": {"address": wallet.key.accAddress}};
  let result = await query_contract(terra, ma_token_address, balance_query_msg);

  if (result.balance !== "1000") {
    throw new Error(`[Deposit]: expected to have balance = 1000 for address ${contract_address}, got ${result.balance}`);
  }

  console.log("Query Result: ");
  console.log(result);
}

async function test_redeem(terra, wallet, lp_contract_address) {
  let reserve_query_msg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await query_contract(terra, lp_contract_address, reserve_query_msg);

  const execute_msg = {
    "send": {
      "contract": lp_contract_address,
      "amount": "500",
      "msg": toEncodedBinary({ "redeem": {"id": "luna"} }),
    }
  };


  const send_msg = new MsgExecuteContract(wallet.key.accAddress, ma_token_address, execute_msg);
  const res = await perform_transaction(terra, wallet, send_msg);

  const balance_query_msg = {"balance": {"address": wallet.key.accAddress}};
  let result = await query_contract(terra, ma_token_address, balance_query_msg);
  console.log(result);
  // verify ma_balance is 500 and uluna is 500 (converted 1:1)
}

function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
const lp_contract_address = await deploy(terra, wallet);


await test_reserve_query(terra, lp_contract_address, "usd")
await test_reserve_query(terra, lp_contract_address, "luna");
await test_config_query(terra, lp_contract_address);
await test_deposit(terra, wallet, lp_contract_address);
await test_redeem(terra, wallet, lp_contract_address);
