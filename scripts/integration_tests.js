import {Coin, LocalTerra, MsgExecuteContract} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract, setup} from "./helpers.mjs";


function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

async function testReserveQuery(terra, address, denom) {
  console.log("### Testing Reserve...")
  let reserveQueryMsg = {"reserve": {"denom": denom}};
  let reserveResult = await queryContract(terra, address, reserveQueryMsg);

  if (!reserveResult.hasOwnProperty("ma_token_address")) {
    throw new Error(`[Reserve]: Reserve Query for symbol ${denom} failed. Result has no property ma_token_address.`)
  }
}


async function main() {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;

  const lpContractAddress = await deploy(terra, wallet);
  const initialAssets = ["uluna", "uusd"];
  await setup(terra, wallet, lpContractAddress, {initialAssets});

  await testReserveQuery(terra, lpContractAddress, "uusd")
  await testReserveQuery(terra, lpContractAddress, "uluna");

  console.log("### Testing Config...")
  let configQueryMsg = {"config": {}};
  let configResult = await queryContract(terra, lpContractAddress, configQueryMsg);

  console.log("Config Query Sent:");
  console.log(configQueryMsg);

  if (!configResult.hasOwnProperty("ma_token_code_id")) {
    throw new Error("[Config]: Config query failed. Result has no property ma_token_code_id.")
  }


  console.log("### Testing Deposit...");
  let {_coins: {uluna: {amount: depositorStartingBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let { ma_token_address } = await queryContract(terra, lpContractAddress, reserveQueryMsg);
  let balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  const { balance: depositContractStartingBalance } = await queryContract(terra, ma_token_address, balanceQueryMsg);

  const depositMsg = {"deposit_native": {"denom": "uluna"}};
  const depositAmount = 10_000_000;
  const coins = new Coin("uluna", depositAmount);
  const executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);
  const depositTxResult = await performTransaction(terra, wallet, executeDepositMsg);

  console.log("Deposit Message Sent: ");
  console.log(executeDepositMsg);

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


  console.log("### Testing Redeem...");
  let {_coins: {uluna: {amount: redeemerStartingLunaBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  const senderMaLunaBalanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let { balance: redeemerStartingMaLunaBalance} = await queryContract(terra, ma_token_address, senderMaLunaBalanceQueryMsg);

  const redeemAmount = 5_000_000;
  const executeMsg = {
    "send": {
      "contract": lpContractAddress,
      "amount": redeemAmount.toString(),
      "msg": toEncodedBinary({ "redeem": {"id": "uluna"} }),
    }
  };

  const redeemSendMsg = new MsgExecuteContract(wallet.key.accAddress, ma_token_address, executeMsg);
  let redeemTxResult = await performTransaction(terra, wallet, redeemSendMsg);

  console.log("Redeem Message Sent:");
  console.log(redeemSendMsg);

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


  console.log("### Testing Borrow...");
  const borrower = terra.wallets.test2;
  let {_coins: {uluna: {amount: borrowerStartingLunaBalance}}} = await terra.bank.balance(borrower.key.accAddress);

  const {_coins: {uluna: {amount: borrowContractStartingBalance}}}  = await terra.bank.balance(lpContractAddress);

  const borrowAmount = 4_000_000;
  const borrowMsg = {"borrow_native": {"denom": "uluna", "amount": borrowAmount.toString()}};
  const executeBorrowMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, borrowMsg);
  const borrowTxResult = await performTransaction(terra, borrower, executeBorrowMsg);

  console.log("Borrow Message Sent: ");
  console.log(executeBorrowMsg);

  let borrowTxInfo = await terra.tx.txInfo(borrowTxResult.txhash);
  const borrowTxFee = Number(borrowTxInfo.tx.fee.amount._coins.uluna.amount);

  let {_coins: {uluna: {amount: borrowerEndingLunaBalance}}} = await terra.bank.balance(borrower.key.accAddress);

  const borrowerLunaBalanceDiff = borrowerEndingLunaBalance - borrowerStartingLunaBalance;
  if (borrowerLunaBalanceDiff !== (borrowAmount - borrowTxFee)) {
    throw new Error(`[Borrow]: expected depositor's balance to increase by ${borrowAmount - borrowTxFee}, \
    got ${borrowerLunaBalanceDiff}`);
  }

  const {_coins: {uluna: {amount: borrowContractEndingBalance}}}  = await terra.bank.balance(lpContractAddress);
  const borrowContractDiff = borrowContractStartingBalance - borrowContractEndingBalance;

  if (borrowContractDiff !== borrowAmount) {
    throw new Error(`[Borrow]: expected luna balance to decrease by ${borrowAmount} for address \
    ${lpContractAddress}, got ${borrowContractDiff}`);
  }


  console.log("### Testing Repay...");
  const repayer = terra.wallets.test2;
  let {_coins: {uluna: {amount: repayerStartingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);

  const repayMsg = {"repay_native": {"denom": "uluna"}};
  let repayAmount = 2_000_000;
  let repayCoins = new Coin("uluna", repayAmount);
  const executeRepayMsg = new MsgExecuteContract(repayer.key.accAddress, lpContractAddress, repayMsg, [repayCoins]);
  const repayTxResult = await performTransaction(terra, repayer, executeRepayMsg);

  console.log("Repay Message Sent: ");
  console.log(executeRepayMsg);

  let repayTxInfo = await terra.tx.txInfo(repayTxResult.txhash);
  const repayTxFee = Number(repayTxInfo.tx.fee.amount._coins.uluna.amount);

  let {_coins: {uluna: {amount: repayerEndingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);
  const partialRepayDiff = repayerStartingLunaBalance - repayerEndingLunaBalance;

  if (partialRepayDiff !== (repayAmount + repayTxFee)) {
    throw new Error(`[Repay]: expected repayer's balance to decrease by ${partialRepayDiff + repayTxFee}, \
    got ${partialRepayDiff}`);
  }

  console.log(await terra.bank.balance(lpContractAddress));

  let overpayAmount = 3_000_000;
  let overpayCoins = new Coin("uluna", overpayAmount);
  const executeOverpayMsg = new MsgExecuteContract(repayer.key.accAddress, lpContractAddress, repayMsg, [overpayCoins]);
  const overpayTxResult = await performTransaction(terra, repayer, executeOverpayMsg);
  console.log(overpayTxResult.logs[0].events[3].attributes);

  let overpayTxInfo = await terra.tx.txInfo(overpayTxResult.txhash);
  const overpayTxFee = Number(overpayTxInfo.tx.fee.amount._coins.uluna.amount);
  console.log("overpay tx fee: " + overpayTxFee);

  let {_coins: {uluna: {amount: overpayEndingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);
  console.log("ending luna balance:  " + repayerEndingLunaBalance);
  console.log("overpay ending luna balance: " + overpayEndingLunaBalance);
  console.log("Diff in Luna Balance after overpaying: " + (overpayEndingLunaBalance - repayerEndingLunaBalance));

  const overpayRepayDiff = overpayEndingLunaBalance - repayerEndingLunaBalance;
  console.log("overpay repay diff: " + overpayRepayDiff);
  // if (overpayRepayDiff !== (overpayAmount + overpayTxFee - (borrowAmount - repayAmount))) {
  //   throw new Error(`[Repay]: expected repayer to be refunded ${overpayAmount - overpayTxFee - (borrowAmount - repayAmount)}, \
  // got ${overpayRepayDiff}`);
  // }

  console.log(await terra.bank.balance(repayer.key.accAddress));
  console.log(await terra.bank.balance(lpContractAddress));
}

main().catch(err => console.log(err));
