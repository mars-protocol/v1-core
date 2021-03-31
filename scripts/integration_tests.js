import {Coin, Int, isTxError, LocalTerra, MsgExecuteContract, StdFee} from "@terra-money/terra.js";
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
  const initialAssets = [
    {denom: "uluna", borrow_slope: "0.1", loan_to_value: "0.5"},
    {denom: "uusd", borrow_slope: "0.5", loan_to_value: "0.8"},
    {denom: "umnt", borrow_slope: "0.3", loan_to_value: "0.7"},
    {denom: "ukrw", borrow_slope: "0.2", loan_to_value: "0.6"},
    {denom: "usdr", borrow_slope: "0.6", loan_to_value: "0.5"},
  ];
  await setup(terra, wallet, lpContractAddress, {initialAssets});

  await testReserveQuery(terra, lpContractAddress, "uusd")
  await testReserveQuery(terra, lpContractAddress, "uluna");
  await testReserveQuery(terra, lpContractAddress, "umnt")
  await testReserveQuery(terra, lpContractAddress, "ukrw");
  await testReserveQuery(terra, lpContractAddress, "usdr");

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

  let depositMsg = {"deposit_native": {"denom": "uluna"}};
  let depositAmount = 10_000_000;
  let coins = new Coin("uluna", depositAmount);
  let executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);
  let depositTxResult = await performTransaction(terra, wallet, executeDepositMsg);

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
  let borrowAmount = 4_000_000;
  let borrowMsg = {"borrow_native": {"denom": "uluna", "amount": borrowAmount.toString()}};
  let executeBorrowMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, borrowMsg);

  let tx = await borrower.createAndSignTx({
    msgs: [executeBorrowMsg],
    fee: new StdFee(30000000, [
      new Coin('uluna', 4000000),
    ]),
  });

  const failedBorrowResult = await terra.tx.broadcast(tx);
  if (!isTxError(failedBorrowResult) || !failedBorrowResult.raw_log.includes("address has no collateral deposited")) {
    throw new Error("Borrower has no collateral deposited. Should not be able to borrow.");
  }

  depositAmount = 80_000_000;
  coins = new Coin("uusd", depositAmount);
  depositMsg = {"deposit_native": {"denom": "uusd"}}
  executeDepositMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, depositMsg, [coins]);
  await performTransaction(terra, borrower, executeDepositMsg);

  // borrow again, still with insufficient collateral deposited
  tx = await borrower.createAndSignTx({
    msgs: [executeBorrowMsg],
    fee: new StdFee(30000000, [
      new Coin('uluna', 4000000),
    ]),
  });

  const secondFailedBorrowResult = await terra.tx.broadcast(tx);
  console.log(secondFailedBorrowResult);
  if (!isTxError(secondFailedBorrowResult) || !secondFailedBorrowResult.raw_log.includes("borrow amount exceeds maximum allowed given current collateral value")) {
    throw new Error("Borrower has insufficient collateral and should not be able to borrow.");
  }

  let {_coins: {uluna: {amount: borrowerStartingLunaBalance}}} = await terra.bank.balance(borrower.key.accAddress);
  const {_coins: {uluna: {amount: borrowContractStartingBalance}}}  = await terra.bank.balance(lpContractAddress);

  // send smaller borrow that should succeed
  let { amount: uusd_to_luna_rate } = await terra.oracle.exchangeRate("uusd");
  let borrowerCollateral = depositAmount / uusd_to_luna_rate;
  let loan_to_value = 0.8;
  borrowAmount = new Int(borrowerCollateral * loan_to_value) - 10_000;
  borrowMsg = {"borrow_native": {"denom": "uluna", "amount": borrowAmount.toString()}};
  executeBorrowMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, borrowMsg);
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

  const {debts: debtBeforeRepay} = await queryContract(terra, lpContractAddress, {"debt": {"address": repayer.key.accAddress}});
  for (let debt of debtBeforeRepay) {
    if (debt.denom === "uluna" && Number(debt.amount) !== borrowAmount) {
      throw new Error(`[Debt]: expected repayer's uluna debt to be ${borrowAmount} before payment, got ${debt.amount}`);
    }
  }

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

  const {debts: debtBeforeFullRepay} = await queryContract(terra, lpContractAddress, {"debt": {"address": repayer.key.accAddress}});
  for (let debt of debtBeforeFullRepay) {
    if (debt.denom === "uluna" && (Math.abs(Number(debt.amount) - (borrowAmount - repayAmount)) > 10)) {
      throw new Error(`[Debt]: expected repayer's uluna debt to be ${borrowAmount - repayAmount} after ${repayAmount} payment, got ${debt.amount}`);
    }
  }

  let overpayAmount = 3_000_000;
  let overpayCoins = new Coin("uluna", overpayAmount);
  const executeOverpayMsg = new MsgExecuteContract(repayer.key.accAddress, lpContractAddress, repayMsg, [overpayCoins]);
  const overpayTxResult = await performTransaction(terra, repayer, executeOverpayMsg);

  let overpayTxInfo = await terra.tx.txInfo(overpayTxResult.txhash);
  const overpayTxFee = Number(overpayTxInfo.tx.fee.amount._coins.uluna.amount);

  let {_coins: {uluna: {amount: overpayEndingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);
  const overpayRepayDiff = repayerEndingLunaBalance - overpayEndingLunaBalance;

  if (Math.abs(overpayRepayDiff - ((borrowAmount - repayAmount) + overpayTxFee)) > 10) {
    throw new Error(`[Repay]: expected repayer's balance to decrease by ${(borrowAmount - repayAmount) + overpayTxFee}, \
  got ${overpayRepayDiff}`);
  }

  const {debts: debtAfterRepay} = await queryContract(terra, lpContractAddress, {"debt": {"address": repayer.key.accAddress}});
  for (let debt of debtAfterRepay) {
    if (debt.denom === "uluna" && debt.amount !== "0") {
      throw new Error(`[Debt]: expected repayer's uluna debt to be 0 after full repayment, got ${debt.amount}`);
    }
  }
}

main().catch(err => console.log(err));
