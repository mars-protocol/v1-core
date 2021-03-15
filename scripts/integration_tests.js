import {Coin, LocalTerra, MsgExecuteContract} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract} from "./helpers.mjs";


function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

async function testReserveQuery(terra, address, symbol) {
  console.log("### Testing Reserve...")
  let reserveQueryMsg = {"reserve": {"symbol": symbol}};
  let reserveResult = await queryContract(terra, address, reserveQueryMsg);

  if (!reserveResult.hasOwnProperty("ma_token_address")) {
    throw new Error(`[Reserve]: Reserve Query for symbol ${symbol} failed. Result has no property ma_token_address.`)
  }

  console.log("Reserve Query Sent:");
  console.log(reserveQueryMsg);
}


async function main() {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;

  const lpContractAddress = await deploy(terra, wallet);

  await testReserveQuery(terra, lpContractAddress, "usd")
  await testReserveQuery(terra, lpContractAddress, "luna");

  console.log("### Testing Config...")
  let configQueryMsg = {"config": {}};
  let configResult = await queryContract(terra, lpContractAddress, configQueryMsg);

  if (!configResult.hasOwnProperty("ma_token_code_id")) {
    throw new Error("[Config]: Config query failed. Result has no property ma_token_code_id.")
  }

  console.log("Config Query Sent:");
  console.log(configQueryMsg);


  console.log("### Testing Deposit...");
  let {_coins: {uluna: {amount: depositorStartingBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  let reserveQueryMsg = {"reserve": {"symbol": "luna"}};
  let { ma_token_address } = await queryContract(terra, lpContractAddress, reserveQueryMsg);
  const balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  const { balance: depositContractStartingBalance } = await queryContract(terra, ma_token_address, balanceQueryMsg);

  const depositMsg = {"deposit_native": {"symbol": "luna"}};
  const depositAmount = 10000;
  const coins = new Coin("uluna", depositAmount);
  const executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);
  const depositTxResult = await performTransaction(terra, wallet, executeDepositMsg);

  const { balance: depositContractEndingBalance } = await queryContract(terra, ma_token_address, balanceQueryMsg);
  const depositContractDiff = depositContractEndingBalance - depositContractStartingBalance;

  if (depositContractDiff !== depositAmount) {
    throw new Error(`[Deposit]: expected luna balance to increase by ${depositAmount} for address \
    ${lpContractAddress}, got ${depositContractDiff}`);
  }

  let txInfo = await terra.tx.txInfo(depositTxResult.txhash);
  const depositTxFee = Number(txInfo.tx.fee.amount._coins.uluna.amount);

  let {_coins: {uluna: {amount: depositorEndingBalance}}} = await terra.bank.balance(wallet.key.accAddress);
  let depositorBalanceDiff = depositorStartingBalance - depositorEndingBalance;

  if (depositorBalanceDiff !== (depositAmount + depositTxFee)) {
    throw new Error(`[Deposit]: expected depositor's balance to decrease by ${depositAmount + depositTxFee}, \
    got ${depositorBalanceDiff}`);
  }

  console.log("Deposit Message Sent: ");
  console.log(executeDepositMsg);


  console.log("### Testing Redeem...");
  let {_coins: {uluna: {amount: redeemerStartingLunaBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  const senderMaLunaBalanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let { balance: redeemerStartingMaLunaBalance} = await queryContract(terra, ma_token_address, senderMaLunaBalanceQueryMsg);

  const redeemAmount = 5000;
  const executeMsg = {
    "send": {
      "contract": lpContractAddress,
      "amount": redeemAmount.toString(),
      "msg": toEncodedBinary({ "redeem": {"id": "luna"} }),
    }
  };

  const redeemSendMsg = new MsgExecuteContract(wallet.key.accAddress, ma_token_address, executeMsg);
  let redeemTxResult = await performTransaction(terra, wallet, redeemSendMsg);
  let redeemTxInfo = await terra.tx.txInfo(redeemTxResult.txhash);
  const redeemTxFee = Number(redeemTxInfo.tx.fee.amount._coins.uluna.amount);

  let { balance: redeemerEndingMaLunaBalance} = await queryContract(terra, ma_token_address, senderMaLunaBalanceQueryMsg);
  const maLunaBalanceDiff = redeemerStartingMaLunaBalance - redeemerEndingMaLunaBalance;

  if (maLunaBalanceDiff !== redeemAmount) {
    throw new Error(`[Redeem]: expected maluna balance to decrease by ${redeemAmount}, got ${maLunaBalanceDiff}`);
  }

  let {_coins: {uluna: {amount: redeemerEndingLunaBalance}}} = await terra.bank.balance(wallet.key.accAddress);
  const redeemerLunaBalanceDiff = redeemerEndingLunaBalance - redeemerStartingLunaBalance;

  if (redeemerLunaBalanceDiff !== (redeemAmount - redeemTxFee)) {
    throw new Error(`[Redeem]: expected depositor's balance to increase by ${redeemAmount - redeemTxFee}, \
    got ${redeemerLunaBalanceDiff}`);
  }

  console.log("Redeem Message Sent:");
  console.log(redeemSendMsg);
}

main().catch(err => console.log(err));
