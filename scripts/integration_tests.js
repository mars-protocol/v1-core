import {Coin, LocalTerra, MsgExecuteContract} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract} from "./helpers.mjs";

async function testReserveQuery(terra, address, symbol) {
  console.log("### Testing Reserve...")
  let queryMsg = {"reserve": {"symbol": symbol}};
  let result = await queryContract(terra, address, queryMsg);

  if (!result.hasOwnProperty("ma_token_address")) {
    throw new Error(`Reserve Query for symbol ${symbol} failed`)
  }

  console.log(`Reserve Query for symbol ${symbol}:`);
  console.log(result);
}

async function testConfigQuery(terra, address) {
  console.log("### Testing Config...")
  let queryMsg = {"config": {}};
  let result = await queryContract(terra, address, queryMsg);

  if (!result.hasOwnProperty("ma_token_code_id")) {
    throw new Error("Config query failed. Result has no property ma_token_code_id.")
  }

  console.log("Config Query:");
  console.log(result);
}

async function testDeposit(terra, wallet, contractAddress) {
  console.log("### Testing Deposit...")
  const depositMsg = {"deposit_native": {"symbol": "luna"}};
  const coins = new Coin("uluna", 1000);
  const executeMsg = new MsgExecuteContract(wallet.key.accAddress, contractAddress, depositMsg, [coins]);
  await performTransaction(terra, wallet, executeMsg);

  let reserveQueryMsg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await queryContract(terra, contractAddress, reserveQueryMsg);

  const balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let result = await queryContract(terra, ma_token_address, balanceQueryMsg);

  if (result.balance !== "1000") {
    throw new Error(`[Deposit]: expected to have balance = 1000 for address ${contractAddress}, got ${result.balance}`);
  }

  console.log("Query Result: ");
  console.log(result);
}

async function testRedeem(terra, wallet, lpContractAddress) {
  let reserveQueryMsg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await queryContract(terra, lpContractAddress, reserveQueryMsg);

  const executeMsg = {
    "send": {
      "contract": lpContractAddress,
      "amount": "500",
      "msg": toEncodedBinary({ "redeem": {"id": "luna"} }),
    }
  };


  const sendMsg = new MsgExecuteContract(wallet.key.accAddress, ma_token_address, executeMsg);
  await performTransaction(terra, wallet, sendMsg);

  const balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let result = await queryContract(terra, ma_token_address, balanceQueryMsg);
  console.log(result);
  // verify ma_balance is 500 and uluna is 500 (converted 1:1)
}

function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
const lpContractAddress = await deploy(terra, wallet);

await testReserveQuery(terra, lpContractAddress, "usd")
await testReserveQuery(terra, lpContractAddress, "luna");
await testConfigQuery(terra, lpContractAddress);
await testDeposit(terra, wallet, lpContractAddress);
await testRedeem(terra, wallet, lpContractAddress);
