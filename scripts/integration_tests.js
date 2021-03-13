import {Coin, LocalTerra, MsgExecuteContract} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract} from "./helpers.mjs";

function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

async function testReserveQuery(terra, address, symbol) {
  console.log("### Testing Reserve...")
  let queryMsg = {"reserve": {"symbol": symbol}};
  let result = await queryContract(terra, address, queryMsg);

  if (!result.hasOwnProperty("ma_token_address")) {
    throw new Error(`[Reserve]: Reserve Query for symbol ${symbol} failed. Result has no property ma_token_address.`)
  }

  console.log(`Reserve Query for symbol ${symbol}:`);
  console.log(result);
}

async function main() {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;

  const lpContractAddress = await deploy(terra, wallet);

  await testReserveQuery(terra, lpContractAddress, "usd")
  await testReserveQuery(terra, lpContractAddress, "luna");

  console.log("### Testing Config...")
  let queryMsg = {"config": {}};
  let configResult = await queryContract(terra, lpContractAddress, queryMsg);

  if (!configResult.hasOwnProperty("ma_token_code_id")) {
    throw new Error("[Config]: Config query failed. Result has no property ma_token_code_id.")
  }

  console.log("Config Query:");
  console.log(configResult);


  console.log("### Testing Deposit...")
  const depositMsg = {"deposit_native": {"symbol": "luna"}};
  const coins = new Coin("uluna", 1000);
  const executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);
  await performTransaction(terra, wallet, executeDepositMsg);

  let reserveQueryMsg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await queryContract(terra, lpContractAddress, reserveQueryMsg);

  const balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let depositQueryResult = await queryContract(terra, ma_token_address, balanceQueryMsg);

  if (depositQueryResult.balance !== "1000") {
    throw new Error(`[Deposit]: expected to have balance = 1000 for address ${lpContractAddress}, got ${depositQueryResult.balance}`);
  }

  console.log("Query Result: ");
  console.log(depositQueryResult);


  console.log("### Testing Redeem...");
  const executeMsg = {
    "send": {
      "contract": lpContractAddress,
      "amount": "500",
      "msg": toEncodedBinary({ "redeem": {"id": "luna"} }),
    }
  };

  const sendMsg = new MsgExecuteContract(wallet.key.accAddress, ma_token_address, executeMsg);
  let redeemTxResult = await performTransaction(terra, wallet, sendMsg);
  const lunaRedeemed = redeemTxResult.logs[0].events[3].attributes[2].value;

  const senderBalanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let senderBalanceResult = await queryContract(terra, ma_token_address, senderBalanceQueryMsg);
  const maLunaBalance = senderBalanceResult.balance;

  if (lunaRedeemed !== "500uluna" || maLunaBalance !== "500") {
    throw new Error(`[Redeem]: expected to have received 500 uluna and a remaining balance of 500 maluna. Received \
  ${lunaRedeemed} uluna and have a remaining balance of ${maLunaBalance} maluna.`);
  }

  console.log(`uluna redeemed: ${lunaRedeemed}`);
  console.log(`remaining maluna balance: ${maLunaBalance}`);
}

main().catch(err => console.log(err));
