import {Coin, Int, isTxError, LocalTerra, MsgExecuteContract, StdFee} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract, setup} from "./helpers.mjs";
import BigNumber from "bignumber.js";
BigNumber.config({ DECIMAL_PLACES: 18 })

// CONSTANTS AND GLOBALS
const INITIAL_ASSETS = [
  {denom: "uluna", borrow_slope: "4", loan_to_value: "0.5"},
  {denom: "uusd", borrow_slope: "5", loan_to_value: "0.8"},
  {denom: "ukrw", borrow_slope: "2", loan_to_value: "0.6"},
];

function debug(string) {
  if (Number(process.env.DEBUG) === 1) {
    console.log(string);
  }
}

// ASSERTS
function assert(expression, message) {
  if (!expression) {
    throw new Error(message);
  }
}

function assertEqual(left, right, message = "Expected values to be equal") {
  assert(left === right, `${message} got \n\t-left:  ${left}, \n\t-right: ${right}`);
}

function assertEqualBN(left, right, message = "Expected values to be equal") {
  assert(left.eq(right), `${message} got \n\t-left:  ${left}, \n\t-right: ${right}`);
}

function assertEqualIndicesAndRates(expectedStateReserve, actualRates) {
  assertEqualBN(expectedStateReserve.liquidityIndex, actualRates.liquidityIndex);
  assertEqualBN(expectedStateReserve.borrowIndex, actualRates.borrowIndex);
  assertEqualBN(expectedStateReserve.borrowRate, actualRates.borrowRate);
  assertEqualBN(expectedStateReserve.liquidityRate, actualRates.liquidityRate);
}

// HELPERS
function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

function isValueInDelta(value, target, deviation) {
  return Math.abs(value - target) < deviation
}

function getTimestampInSecondsFromDateField(dateField) {
  return (new Date(dateField).valueOf()) / 1000;
}

// Expected State
function updateExpectedAssetIndices(expectedState, asset, blockTime) {
  let expectedStateReserve = expectedState.reserves[asset];
  const SECONDS_PER_YEAR = new BigNumber(31536000);

  let secondsElapsed = blockTime - expectedStateReserve.interestsLastUpdated;

  let expectedAccumulatedLiquidityInterest =
    expectedStateReserve.liquidityRate
      .times(secondsElapsed)
      .dividedBy(SECONDS_PER_YEAR)
      .plus(1);
  expectedStateReserve.liquidityIndex = expectedStateReserve.liquidityIndex.times(expectedAccumulatedLiquidityInterest);

  let expectedAccumulatedBorrowInterest =
    expectedStateReserve.borrowRate
      .times(secondsElapsed)
      .dividedBy(SECONDS_PER_YEAR)
      .plus(1);
  expectedStateReserve.borrowIndex = expectedStateReserve.borrowIndex.times(expectedAccumulatedBorrowInterest);

  expectedStateReserve.interestsLastUpdated = blockTime;
}

function updateExpectedAssetRates(expectedState, asset) {
  let expectedStateReserve = expectedState.reserves[asset];

  let assetDebtTotal = expectedStateReserve.debtTotalScaled.times(expectedStateReserve.borrowIndex);
  console.log("asset debt total: " + assetDebtTotal.toNumber());
  let assetLiquidityTotal = new BigNumber(expectedState.lpContractBalances[asset]);
  console.log("asset liquidity total: " + assetLiquidityTotal.toNumber());
  let assetLockedTotal = assetLiquidityTotal.plus(assetDebtTotal);
  console.log("asset locked total: " + assetLockedTotal);

  let expectedUtilizationRate =
    assetLockedTotal.isZero() ? new BigNumber(0) : assetDebtTotal.dividedBy(assetLockedTotal);

  console.log("expected utilization rate: " + expectedUtilizationRate.toNumber());

  expectedStateReserve.borrowRate = expectedUtilizationRate.times(expectedStateReserve.borrowSlope);
  console.log("borrow rate: " + expectedStateReserve.borrowRate.toNumber());
  expectedStateReserve.liquidityRate = expectedStateReserve.borrowRate.times(expectedUtilizationRate);
  console.log("liquidity rate: " + expectedStateReserve.liquidityRate);
}

// QUERIES
async function getAddressNativeBalances(terra, address) {
  let ret = {};
  let balanceQuery =
    await terra.bank.balance(address);

  INITIAL_ASSETS.map(asset => asset.denom).forEach((denom) => {
    ret[denom] = Number(balanceQuery._coins[denom].amount);
  });

  return ret;
}


function getIndicesAndRatesFromTxResult(txResult) {
  let fromContractEvent = txResult.logs[0].eventsByType.from_contract;

  let liquidityRate = new BigNumber(fromContractEvent.liquidity_rate[0]);
  let borrowRate = new BigNumber(fromContractEvent.borrow_rate[0]);
  let liquidityIndex = new BigNumber(fromContractEvent.liquidity_index[0]);
  let borrowIndex = new BigNumber(fromContractEvent.borrow_index[0]);
  return {liquidityRate, borrowRate, liquidityIndex, borrowIndex}
}

// ACTIONS
async function depositAssets(terra, wallet, lpContractAddress, deposits) {
  for (let deposit of deposits) {
    let depositMsg = {"deposit_native": {"denom": deposit.denom}};
    let depositAmount = deposit.amount;
    let coins = new Coin(deposit.denom, depositAmount);
    let executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);

    await performTransaction(terra, wallet, executeDepositMsg);
  }
}

// TESTS
async function testDeposit(env, expectedState, depositUser, depositAsset, depositAmount) {
  console.log(`### Testing Deposit | ${depositUser} -> ${depositAmount} ${depositAsset}`);

  let depositAddress = env.terra.wallets[depositUser].key.accAddress;

  // Execute Deposit
  let depositMsg = {"deposit_native": {"denom": depositAsset}};
  let coins = new Coin(depositAsset, depositAmount);
  let executeDepositMsg =
    new MsgExecuteContract(depositAddress, env.lpContractAddress, depositMsg, [coins]);
  let depositTxResult = await performTransaction(env.terra, env.ownerWallet, executeDepositMsg);
  debug(executeDepositMsg);
  debug(depositTxResult);

  let txInfo = await env.terra.tx.txInfo(depositTxResult.txhash);

  // lpContract balance should go up by deposit amount
  expectedState.lpContractBalances[depositAsset] += depositAmount;
  let lpContractBalance = await env.terra.bank.balance(env.lpContractAddress);
  debug(lpContractBalance);
  assertEqual(expectedState.lpContractBalances[depositAsset], Number(lpContractBalance._coins[depositAsset].amount));

  // maContract balance should go up by deposit amount
  expectedState.maContractBalances[depositAsset] += depositAmount;
  let balanceQueryMsg = {"balance": {"address": depositAddress}};
  const balanceQueryResult =
    await queryContract(
      env.terra,
      expectedState.reserves[depositAsset].maTokenAddress,
      balanceQueryMsg);
  debug(balanceQueryMsg);
  debug(balanceQueryResult);
  assertEqual(expectedState.maContractBalances[depositAsset], Number(balanceQueryResult.balance));

  // Update and check indices and rates
  let blockTime = getTimestampInSecondsFromDateField(txInfo.timestamp);
  updateExpectedAssetIndices(expectedState, depositAsset, blockTime);
  updateExpectedAssetRates(expectedState, depositAsset);

  let actualIndicesAndRates = getIndicesAndRatesFromTxResult(depositTxResult);
  assertEqualIndicesAndRates(expectedState.reserves[depositAsset], actualIndicesAndRates);

  // Depositor balance should go down by deposit amount + txfee
  const depositTxFee = Number(txInfo.tx.fee.amount._coins[depositAsset].amount);
  expectedState.userBalances[depositUser][depositAsset] -= (depositAmount + depositTxFee);
  let actualEndingBalances = await getAddressNativeBalances(env.terra, depositAddress);
  assertEqual(
    expectedState.userBalances[depositUser][depositAsset],
    actualEndingBalances[depositAsset]
  );
}

async function testRedeem(env, expectedState, redeemUser, redeemAsset, redeemAmount) {
  console.log(`### Testing Redeem | ${redeemUser} -> ${redeemAmount} ${redeemAsset}`);

  let redeemAddress = env.terra.wallets[redeemUser].key.accAddress;

  const executeMsg = {
    "send": {
      "contract": env.lpContractAddress,
      "amount": redeemAmount.toString(),
      "msg": toEncodedBinary({ "redeem": {"id": redeemAsset} }),
    }
  };

  const redeemSendMsg = new MsgExecuteContract(redeemAddress, expectedState.reserves[redeemAsset].maTokenAddress, executeMsg);
  let redeemTxResult = await performTransaction(env.terra, env.terra.wallets[redeemUser], redeemSendMsg);
  debug(redeemSendMsg);
  debug(redeemTxResult);

  let redeemTxInfo = await env.terra.tx.txInfo(redeemTxResult.txhash);

  // lpContract balance should go down by redeem amount
  expectedState.lpContractBalances[redeemAsset] -= redeemAmount;
  let lpContractBalance = await env.terra.bank.balance(env.lpContractAddress);
  debug(lpContractBalance);
  assertEqual(expectedState.lpContractBalances[redeemAsset], Number(lpContractBalance._coins[redeemAsset].amount));

  // maContract balance should go down by redeem amount
  expectedState.maContractBalances[redeemAsset] -= redeemAmount;
  let balanceQueryMsg = {"balance": {"address": redeemAddress}};
  const balanceQueryResult =
    await queryContract(
      env.terra,
      expectedState.reserves[redeemAsset].maTokenAddress,
      balanceQueryMsg);
  debug(balanceQueryMsg);
  debug(balanceQueryResult);
  assertEqual(expectedState.maContractBalances[redeemAsset], Number(balanceQueryResult.balance));

  // Update and check indices and rates
  let blockTime = getTimestampInSecondsFromDateField(redeemTxInfo.timestamp);
  updateExpectedAssetIndices(expectedState, redeemAsset, blockTime);
  updateExpectedAssetRates(expectedState, redeemAsset);

  let actualIndicesAndRates = getIndicesAndRatesFromTxResult(redeemTxResult);
  assertEqualIndicesAndRates(expectedState.reserves[redeemAsset], actualIndicesAndRates);

  // Redeemer balance should go up by redeem amount - txfee
  const redeemTxFee = Number(redeemTxInfo.tx.fee.amount._coins[redeemAsset].amount);
  expectedState.userBalances[redeemUser][redeemAsset] += (redeemAmount - redeemTxFee);
  let actualEndingBalances = await getAddressNativeBalances(env.terra, redeemAddress);
  assertEqual(
    expectedState.userBalances[redeemUser][redeemAsset],
    actualEndingBalances[redeemAsset]
  );
}

async function testBorrow(env, expectedState, borrowUser, borrowAsset, borrowAmount) {
  console.log(`### Testing Borrow | ${borrowUser} -> ${borrowAmount} ${borrowAsset}`);

  let borrowAddress = env.terra.wallets[borrowUser].key.accAddress;

  let borrowMsg = {"borrow_native": {"denom": borrowAsset, "amount": borrowAmount.toString()}};
  let executeBorrowMsg = new MsgExecuteContract(borrowAddress, env.lpContractAddress, borrowMsg);
  const borrowTxResult = await performTransaction(env.terra, env.terra.wallets[borrowUser], executeBorrowMsg);

  debug(executeBorrowMsg);
  debug(borrowTxResult);

  let borrowTxInfo = await env.terra.tx.txInfo(borrowTxResult.txhash);

  // LP Contract balance should go down by borrow amount
  expectedState.lpContractBalances[borrowAsset] -= borrowAmount;
  const contractBalance = await env.terra.bank.balance(env.lpContractAddress);
  assertEqual(expectedState.lpContractBalances[borrowAsset], Number(contractBalance._coins[borrowAsset].amount));

  // maContract balance should remain unchanged
  let balanceQueryMsg = {"balance": {"address": borrowAddress}};
  const balanceQueryResult =
    await queryContract(
      env.terra,
      expectedState.reserves[borrowAsset].maTokenAddress,
      balanceQueryMsg);
  debug(balanceQueryMsg);
  debug(balanceQueryResult);
  assertEqual(expectedState.maContractBalances[borrowAsset], Number(balanceQueryResult.balance));

  // Update debt total, indices, and rates and test
  let borrowAmountScaled = new BigNumber(borrowAmount).dividedBy(expectedState.reserves[borrowAsset].borrowIndex);
  expectedState.reserves[borrowAsset].debtTotalScaled = expectedState.reserves[borrowAsset].debtTotalScaled.plus(borrowAmountScaled);
  let blockTime = getTimestampInSecondsFromDateField(borrowTxInfo.timestamp);
  updateExpectedAssetIndices(expectedState, borrowAsset, blockTime);
  updateExpectedAssetRates(expectedState, borrowAsset);

  let actualIndicesAndRates = getIndicesAndRatesFromTxResult(borrowTxResult);
  assertEqualIndicesAndRates(expectedState.reserves[borrowAsset], actualIndicesAndRates);

  // Borrower balance should go up by borrow amount - txfee
  const borrowTxFee = Number(borrowTxInfo.tx.fee.amount._coins[borrowAsset].amount);
  expectedState.userBalances[borrowUser][borrowAsset] += (borrowAmount - borrowTxFee);
  let actualEndingBalances = await getAddressNativeBalances(env.terra, borrowAddress);
  assertEqual(
    expectedState.userBalances[borrowUser][borrowAsset],
    actualEndingBalances[borrowAsset]
  );
}

async function testRepay(env, expectedState, repayUser, repayAsset, repayAmount) {
  console.log(`### Testing Repay | ${repayUser} -> ${repayAmount} ${repayAsset}`);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let assetReserve = await queryContract(env.terra, env.lpContractAddress, reserveQueryMsg);
  console.log(assetReserve);

  let repayAddress = env.terra.wallets[repayUser].key.accAddress;

  const repayMsg = {"repay_native": {"denom": repayAsset}};
  let repayCoins = new Coin(repayAsset, repayAmount);
  const executeRepayMsg = new MsgExecuteContract(repayAddress, env.lpContractAddress, repayMsg, [repayCoins]);
  const repayTxResult = await performTransaction(env.terra, env.terra.wallets[repayUser], executeRepayMsg);

  debug(executeRepayMsg);
  debug(repayTxResult);

  let repayTxInfo = await env.terra.tx.txInfo(repayTxResult.txhash);

  // check lpContract balance increases by repay amount
  expectedState.lpContractBalances[repayAsset] += repayAmount;
  const contractBalance = await env.terra.bank.balance(env.lpContractAddress);
  assertEqual(expectedState.lpContractBalances[repayAsset], Number(contractBalance._coins[repayAsset].amount));

  // check maContract balance remains unchanged
  let balanceQueryMsg = {"balance": {"address": repayAddress}};
  const balanceQueryResult =
    await queryContract(
      env.terra,
      expectedState.reserves[repayAsset].maTokenAddress,
      balanceQueryMsg);
  debug(balanceQueryMsg);
  debug(balanceQueryResult);
  assertEqual(expectedState.maContractBalances[repayAsset], Number(balanceQueryResult.balance));

  // Update debt total and check indices and rates
  let blockTime = getTimestampInSecondsFromDateField(repayTxInfo.timestamp);
  updateExpectedAssetIndices(expectedState, repayAsset, blockTime);
  let repayAmountScaled = new BigNumber(repayAmount).dividedBy(expectedState.reserves[repayAsset].borrowIndex);
  expectedState.reserves[repayAsset].debtTotalScaled = expectedState.reserves[repayAsset].debtTotalScaled.minus(repayAmountScaled);
  console.log("my debt total scaled: " + expectedState.reserves[repayAsset].debtTotalScaled.toNumber());
  expectedState.reserves[repayAsset].debtTotalScaled = new BigNumber(1);
  updateExpectedAssetRates(expectedState, repayAsset);

  reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  assetReserve = await queryContract(env.terra, env.lpContractAddress, reserveQueryMsg);
  console.log(assetReserve);

  let actualIndicesAndRates = getIndicesAndRatesFromTxResult(repayTxResult);
  // assertEqualIndicesAndRates(expectedState.reserves[repayAsset], actualIndicesAndRates);

  // Repayer balance should go down by repay amount + txfee
  const repayTxFee = Number(repayTxInfo.tx.fee.amount._coins.uluna.amount);
  expectedState.userBalances[repayUser][repayAsset] -= (repayAmount + repayTxFee);
  let actualEndingBalances = await getAddressNativeBalances(env.terra, repayAddress);
  assertEqual(
    expectedState.userBalances[repayUser][repayAsset],
    actualEndingBalances[repayAsset]
  );
}

// async function testBorrow(env, expectedState, borrowUser, borrowAsset, depositAsset, depositAmount) {
//   console.log(`### Testing Borrow | ${borrowUser}`);
//
//   // try borrowing deposit amount, should fail as the user has no collateral
//   const borrowAddress = env.terra.wallets[borrowUser].key.accAddress;
//   let borrowMsg = {"borrow_native": {"denom": borrowAsset, "amount": depositAmount.toString()}};
//   let executeBorrowMsg = new MsgExecuteContract(borrowAddress, env.lpContractAddress, borrowMsg);
//
//   let tx = await env.terra.wallets[borrowUser].createAndSignTx({
//     msgs: [executeBorrowMsg],
//     fee: new StdFee(30000000, [
//       new Coin(borrowAsset, 4000000),
//     ]),
//   });
//
//   const failedBorrowResult = await env.terra.tx.broadcast(tx);
//   console.log('First Failed Borrow Message Sent:')
//   console.log(failedBorrowResult);
//   if (!isTxError(failedBorrowResult) || !failedBorrowResult.raw_log.includes("address has no collateral deposited")) {
//     throw new Error("Borrower has no collateral deposited. Should not be able to borrow.");
//   }
//
//   let coins = new Coin(depositAsset, depositAmount);
//   let depositMsg = {"deposit_native": {"denom": depositAsset}};
//   let executeDepositMsg = new MsgExecuteContract(borrowAddress, env.lpContractAddress, depositMsg, [coins]);
//   let depositTxResult = await performTransaction(env.terra, env.terra.wallets[borrowUser], executeDepositMsg);
//
//   let depositTxInfo = await env.terra.tx.txInfo(depositTxResult.txhash);
//
//   // Contract balance should go up by deposit amount
//   expectedState.contractBalances[depositAsset] += depositAmount;
//   let balanceQueryMsg = {"balance": {"address": borrowAddress}};
//   const balanceQueryResult =
//     await queryContract(
//       env.terra,
//       expectedState.reserves[depositAsset].maTokenAddress,
//       balanceQueryMsg);
//   debug(balanceQueryMsg);
//   debug(balanceQueryResult);
//   assertEqual(expectedState.contractBalances[depositAsset], Number(balanceQueryResult.balance));
//
//   // Update and check indices and rates
//   let blockTime = getTimestampInSecondsFromDateField(depositTxInfo.timestamp);
//   updateExpectedAssetIndices(expectedState, depositAsset, blockTime);
//   updateExpectedAssetRates(expectedState, depositAsset);
//
//   let actualIndicesAndRates = getIndicesAndRatesFromTxResult(depositTxResult);
//   assertEqualIndicesAndRates(expectedState.reserves[depositAsset], actualIndicesAndRates);
//
//   // Depositor balance should go down by deposit amount + txfee
//   const depositTxFee = Number(depositTxInfo.tx.fee.amount._coins.uluna.amount);
//   expectedState.userBalances[borrowUser][depositAsset] -= (depositAmount + depositTxFee);
//   let actualEndingBalances = await getAddressNativeBalances(env.terra, borrowAddress);
//   assertEqual(
//     expectedState.userBalances[borrowUser][depositAsset],
//     actualEndingBalances[depositAsset]
//   );
//
//   // borrow again, still with insufficient collateral deposited
//   tx = await env.terra.wallets[borrowUser].createAndSignTx({
//     msgs: [executeBorrowMsg],
//     fee: new StdFee(30000000, [
//       new Coin('uluna', 4000000),
//     ]),
//   });
//
//   const secondFailedBorrowResult = await env.terra.tx.broadcast(tx);
//   console.log('Second Failed Borrow Message Sent:')
//   console.log(secondFailedBorrowResult);
//   if (!isTxError(secondFailedBorrowResult) || !secondFailedBorrowResult.raw_log.includes("borrow amount exceeds maximum allowed given current collateral value")) {
//     throw new Error("Borrower has insufficient collateral and should not be able to borrow.");
//   }
// }

/*
async function testRedeem(inputs) {
  let {terra, wallet, lpContractAddress, initialLiquidity} = inputs;
  let {_coins: {uluna: {amount: redeemerStartingLunaBalance}}} =
    await terra.bank.balance(wallet.key.accAddress);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let lunaReserve =
    await queryContract(terra, lpContractAddress, reserveQueryMsg);

  const senderMaLunaBalanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  let { balance: redeemerStartingMaLunaBalance} = await queryContract(terra, lunaReserve.ma_token_address, senderMaLunaBalanceQueryMsg);

  const redeemAmount = 5_000_000;
  const executeMsg = {
    "send": {
      "contract": lpContractAddress,
      "amount": redeemAmount.toString(),
      "msg": toEncodedBinary({ "redeem": {"id": "uluna"} }),
    }
  };

  const redeemSendMsg = new MsgExecuteContract(wallet.key.accAddress, lunaReserve.ma_token_address, executeMsg);
  let redeemTxResult = await performTransaction(terra, wallet, redeemSendMsg);

  console.log(redeemSendMsg);

  let redeemTxInfo = await terra.tx.txInfo(redeemTxResult.txhash);
  const redeemTxFee = Number(redeemTxInfo.tx.fee.amount._coins.uluna.amount);

  let blockTime = new Date(redeemTxInfo.timestamp).valueOf()

  let realRates = getRealIndicesAndRates(redeemTxResult);
  let expectedRates = await getExpectedIndicesAndRates(lunaReserve, blockTime, initialLiquidity, 0, 0, redeemAmount);

  console.log(realRates);
  console.log(expectedRates);
  assertEqualInterestRates(realRates, expectedRates);

  initialLiquidity -= redeemAmount;

  let { balance: redeemerEndingMaLunaBalance} = await queryContract(terra, lunaReserve.ma_token_address, senderMaLunaBalanceQueryMsg);
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

  return { initialLiquidity }
}

async function testBorrow(inputs) {
  let {terra, lpContractAddress, borrower, initialLiquidity} = inputs;
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
  console.log('First Failed Borrow Message Sent:')
  console.log(failedBorrowResult);
  if (!isTxError(failedBorrowResult) || !failedBorrowResult.raw_log.includes("address has no collateral deposited")) {
    throw new Error("Borrower has no collateral deposited. Should not be able to borrow.");
  }

  let depositAmount = 8_000_000;
  let coins = new Coin("uusd", depositAmount);
  let depositMsg = {"deposit_native": {"denom": "uusd"}}
  let executeDepositMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, depositMsg, [coins]);
  await performTransaction(terra, borrower, executeDepositMsg);

  // borrow again, still with insufficient collateral deposited
  tx = await borrower.createAndSignTx({
    msgs: [executeBorrowMsg],
    fee: new StdFee(30000000, [
      new Coin('uluna', 4000000),
    ]),
  });

  const secondFailedBorrowResult = await terra.tx.broadcast(tx);
  console.log('Second Failed Borrow Message Sent:')
  console.log(secondFailedBorrowResult);
  if (!isTxError(secondFailedBorrowResult) || !secondFailedBorrowResult.raw_log.includes("borrow amount exceeds maximum allowed given current collateral value")) {
    throw new Error("Borrower has insufficient collateral and should not be able to borrow.");
  }

  let {_coins: {uluna: {amount: borrowerStartingLunaBalance}}} = await terra.bank.balance(borrower.key.accAddress);
  const {_coins: {uluna: {amount: borrowContractStartingBalance}}}  = await terra.bank.balance(lpContractAddress);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let lunaReserve = await queryContract(terra, lpContractAddress, reserveQueryMsg);

  // send smaller borrow that should succeed
  let { amount: uusd_to_luna_rate } = await terra.oracle.exchangeRate("uusd");
  let borrowerCollateral = depositAmount / uusd_to_luna_rate;
  borrowAmount = new Int(borrowerCollateral * Number(lunaReserve.loan_to_value) - 10_000);
  console.log("actual utilization rate: " + (borrowAmount / 5_000_000));
  console.log(initialLiquidity + " initial liquidity");
  borrowMsg = {"borrow_native": {"denom": "uluna", "amount": borrowAmount.toString()}};
  executeBorrowMsg = new MsgExecuteContract(borrower.key.accAddress, lpContractAddress, borrowMsg);
  const borrowTxResult = await performTransaction(terra, borrower, executeBorrowMsg);

  console.log("Borrow Message Sent: ");
  console.log(executeBorrowMsg);

  let borrowTxInfo = await terra.tx.txInfo(borrowTxResult.txhash);
  const borrowTxFee = Number(borrowTxInfo.tx.fee.amount._coins.uluna.amount);

  console.log(lunaReserve);
  let blockTime = new Date(borrowTxInfo.timestamp).valueOf()

  let realRates = getRealIndicesAndRates(borrowTxResult);
  let expectedRates = await getExpectedIndicesAndRates(lunaReserve, blockTime, initialLiquidity, borrowAmount, 0, borrowAmount);

  console.log(realRates);
  console.log(expectedRates);
  assertEqualInterestRates(realRates, expectedRates);

  initialLiquidity -= borrowAmount;
  let {_coins: {uluna: {amount: borrowerEndingLunaBalance}}} = await terra.bank.balance(borrower.key.accAddress);

  const borrowerLunaBalanceDiff = borrowerEndingLunaBalance - borrowerStartingLunaBalance;
  if (borrowerLunaBalanceDiff !== (borrowAmount - borrowTxFee)) {
    throw new Error(`[Borrow]: expected depositor's balance to increase by ${borrowAmount - borrowTxFee}, \
    got ${borrowerLunaBalanceDiff}`);
  }

  const {_coins: {uluna: {amount: borrowContractEndingBalance}}}  = await terra.bank.balance(lpContractAddress);
  const borrowContractDiff = borrowContractStartingBalance - borrowContractEndingBalance;

  if (borrowContractDiff !== Number(borrowAmount)) {
    throw new Error(`[Borrow]: expected luna balance to decrease by ${borrowAmount} for address \
    ${lpContractAddress}, got ${borrowContractDiff}`);
  }

  return { initialLiquidity, borrowAmount }
}

async function testRepay(inputs) {
  let {terra, lpContractAddress, repayer, initialLiquidity, borrowAmount} = inputs;
  let {_coins: {uluna: {amount: repayerStartingLunaBalance}}} =
    await terra.bank.balance(repayer.key.accAddress);

  const {debts: debtBeforeRepay} =
    await queryContract(terra, lpContractAddress, {"debt": {"address": repayer.key.accAddress}});

  console.log(debtBeforeRepay);
  for (let debt of debtBeforeRepay) {
    if (debt.denom === "uluna" && Number(debt.amount) !== Number(borrowAmount)) {
      throw new Error(`[Debt]: expected repayer's uluna debt to be ${borrowAmount} before payment, got ${debt.amount}`);
    }
  }

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let lunaReserve = await queryContract(terra, lpContractAddress, reserveQueryMsg);

  const repayMsg = {"repay_native": {"denom": "uluna"}};
  let repayAmount = 200_000;
  let repayCoins = new Coin("uluna", repayAmount);
  const executeRepayMsg = new MsgExecuteContract(repayer.key.accAddress, lpContractAddress, repayMsg, [repayCoins]);
  const repayTxResult = await performTransaction(terra, repayer, executeRepayMsg);

  console.log("Repay Message Sent: ");
  console.log(executeRepayMsg);

  let repayTxInfo = await terra.tx.txInfo(repayTxResult.txhash);
  const repayTxFee = Number(repayTxInfo.tx.fee.amount._coins.uluna.amount);

  let blockTime = new Date(repayTxInfo.timestamp).valueOf()

  let realRates = getRealIndicesAndRates(repayTxResult);
  let expectedRates =
    await getExpectedIndicesAndRates(
      lunaReserve,
      blockTime,
      initialLiquidity,
      0,
      repayAmount,
      0
    );

  console.log("actual utilization rate: " + ((borrowAmount - repayAmount) / 5_000_000));
  console.log(realRates);
  console.log(expectedRates);
  assertEqualInterestRates(realRates, expectedRates);

  initialLiquidity += repayAmount;

  let {_coins: {uluna: {amount: repayerEndingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);
  const partialRepayDiff = repayerStartingLunaBalance - repayerEndingLunaBalance;
  console.log("Ending Luna Balance: " + repayerEndingLunaBalance);

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

  lunaReserve = await queryContract(terra, lpContractAddress, reserveQueryMsg);

  let overpayAmount = 100_000;
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

  blockTime = new Date(overpayTxInfo.timestamp).valueOf()

  realRates = getRealIndicesAndRates(overpayTxResult);
  expectedRates = await getExpectedIndicesAndRates(lunaReserve, blockTime, initialLiquidity, 0, debtAfterRepay, 0);

  console.log(realRates);
  console.log(expectedRates);
  assertEqualInterestRates(realRates, expectedRates);
}

async function testCollateralCheck(inputs) {
  let {terra, wallet, lpContractAddress} = inputs;
  let deposits = [
    {denom: "uluna", amount: 10_000_000},
    {denom: "uusd", amount: 5_000_000},
    {denom: "umnt", amount: 15_000_000},
    {denom: "ukrw", amount: 50_000_000},
    {denom: "usdr", amount: 25_000_000}
  ];

  await depositAssets(terra, wallet, lpContractAddress, deposits);

  let reserve_ltv = {"uluna": 0.5, "uusd": 0.8, "umnt": 0.7, "ukrw": 0.6, "usdr": 0.5};
  let {_coins: exchangeRates} = await terra.oracle.exchangeRates();

  let max_borrow_allowed_in_uluna = 10_000_000 * reserve_ltv["uluna"];
  for (let deposit of deposits) {
    if (exchangeRates.hasOwnProperty(deposit.denom)) {
      max_borrow_allowed_in_uluna += reserve_ltv[deposit.denom] * deposit.amount / exchangeRates[deposit.denom].amount;
    }
  }

  let max_borrow_allowed_in_uusd = new Int(max_borrow_allowed_in_uluna / exchangeRates['uusd'].amount);

  let excessiveBorrowAmount = max_borrow_allowed_in_uusd + 100;
  let validBorrowAmount = max_borrow_allowed_in_uusd - 100;

  let borrowMsg = {"borrow_native": {"denom": "uusd", "amount": excessiveBorrowAmount.toString()}};
  let executeBorrowMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, borrowMsg);
  let tx = await wallet.createAndSignTx({
    msgs: [executeBorrowMsg],
    fee: new StdFee(30000000, [
      new Coin('uluna', 4000000),
    ]),
  });

  const insufficientCollateralResult = await terra.tx.broadcast(tx);
  if (!isTxError(insufficientCollateralResult) || !insufficientCollateralResult.raw_log.includes("borrow amount exceeds maximum allowed given current collateral value")) {
    throw new Error("[Collateral]: Borrower has insufficient collateral and should not be able to borrow.");
  }

  borrowMsg = {"borrow_native": {"denom": "uusd", "amount": validBorrowAmount.toString()}};
  executeBorrowMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, borrowMsg);
  await performTransaction(terra, wallet, executeBorrowMsg);

  console.log("Borrow Message Sent: ");
  console.log(executeBorrowMsg);
}
*/

// MAIN
async function main() {
  let terra = new LocalTerra();
  let ownerWallet = terra.wallets.test1;
  const lpContractAddress = await deploy(terra, ownerWallet);

  let env = {
    terra,
    ownerWallet,
    lpContractAddress,
  };

  await setup(env.terra, env.ownerWallet, lpContractAddress, {initialAssets: INITIAL_ASSETS});

  let test1Balances = await getAddressNativeBalances(env.terra, env.terra.wallets.test1.key.accAddress);
  let test2Balances = await getAddressNativeBalances(env.terra, env.terra.wallets.test2.key.accAddress);

  let expectedStateReserves = {};
  for (const denom of INITIAL_ASSETS.map(asset => asset.denom)) {
    let reserveQueryMsg = {"reserve": {"denom": denom}};
    let assetReserve = await queryContract(env.terra, env.lpContractAddress, reserveQueryMsg);

    expectedStateReserves[denom] = {
      liquidityRate: new BigNumber(0),
      borrowRate: new BigNumber(0),
      liquidityIndex: new BigNumber(1),
      borrowIndex: new BigNumber(1),
      debtTotalScaled: new BigNumber(0),
      borrowSlope: new BigNumber(assetReserve.borrow_slope),
      interestsLastUpdated: assetReserve.interests_last_updated,
      maTokenAddress: assetReserve.ma_token_address,
    };
  }

  let expectedState = {
    maContractBalances: {
      uluna: 0,
      uusd: 0,
      ukrw: 0,
    },
    lpContractBalances: {
      uluna: 0,
      uusd: 0,
      ukrw: 0,
    },
    userBalances: {
      test1: test1Balances,
      test2: test2Balances,
    },
    reserves: expectedStateReserves,
  }
  await testDeposit(env, expectedState, "test1", "uluna", 10_000_000);
  await testRedeem(env, expectedState, "test1", "uluna", 5_000_000);
  await testBorrow(env, expectedState, "test1", "uluna", 2_000_000);
  await testRepay(env, expectedState, "test1", "uluna", 2_000_000);
  /*
  console.log("### Testing Redeem...");
  let { initialLiquidity: initialLiquidityAfterDeposit } = depositOutput;
  let redeemInputs = {
    terra,
    wallet,
    lpContractAddress,
    initialLiquidity: initialLiquidityAfterDeposit,
  }
  let redeemOutputs = await testRedeem(redeemInputs);

  console.log("### Testing Borrow...");
  let { initialLiquidity: initialLiquidityAfterRedeem } = redeemOutputs;
  let borrowInputs = {
    terra,
    lpContractAddress,
    borrower: terra.wallets.test2,
    initialLiquidity: initialLiquidityAfterRedeem,
  }
  let borrowOutput = await testBorrow(borrowInputs);

  console.log("### Testing Repay...");
  let {borrowAmount, initialLiquidity: initialLiquidityAfterBorrow} = borrowOutput;
  let repayInputs = {
    terra,
    lpContractAddress,
    borrowAmount,
    repayer: terra.wallets.test2,
    initialLiquidity: initialLiquidityAfterBorrow,
  }
  await testRepay(repayInputs);

  console.log("### Testing Collateral Check...");
  let collateralCheckInputs = {
    terra,
    wallet: terra.wallets.test3,
    lpContractAddress,
  }
  await testCollateralCheck(collateralCheckInputs);
  */
  console.log("OK");
}

main().catch(err => console.log(err));
