import {Coin, Int, isTxError, LocalTerra, MsgExecuteContract, StdFee} from "@terra-money/terra.js";
import {deploy, performTransaction, queryContract, setup} from "./helpers.mjs";
import BigNumber from "bignumber.js";
// BigNumber.config({ DECIMAL_PLACES: 18 })

function assert(expression, message) {
  if (!expression) {
    throw new Error(message);
  }
}

function assertEqual(left, right, message = "Expected values to be equal") {
  assert(left === right, `${message} got \n\t-left: ${left}, \n\t-right: ${right}`);
}

function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}

function isValueInDelta(value, target, deviation) {
  return Math.abs(value - target) < deviation
}

// Round value to given number of decimals
function round(num, dec) {
  let multiplicator = Math.pow(10, dec);
  num = parseFloat((num * multiplicator).toFixed(11));
  let test = (Math.round(num) / multiplicator);
  return +(test.toFixed(dec));
}

async function getExpectedIndicesAndRates(
  reserve,
  blockTime,
  initialLiquidity,
  moreDebt,
  lessDebt,
  lessLiquidity
) {
  const SECONDS_PER_YEAR = new BigNumber(31536000, 10);
  blockTime = new BigNumber(blockTime, 10);
  initialLiquidity = new BigNumber(initialLiquidity, 10);
  moreDebt = new BigNumber(moreDebt, 10);
  lessDebt = new BigNumber(lessDebt, 10);
  lessLiquidity = new BigNumber(lessLiquidity, 10);

  let interestsLastUpdated = new BigNumber(reserve.interests_last_updated, 10);
  let liquidityRate = new BigNumber(reserve.liquidity_rate, 10);
  let liquidityIndex = new BigNumber(reserve.liquidity_index, 10);
  let borrowRate = new BigNumber(reserve.borrow_rate, 10);
  let borrowIndex = new BigNumber(reserve.borrow_index, 10);
  let debtTotalScaled = new BigNumber(reserve.debt_total_scaled, 10);
  let borrowSlope = new BigNumber(reserve.borrow_slope, 10);

  console.log(reserve);

  console.log("debtTotalScaled: " + debtTotalScaled);

  console.log("block time: " + blockTime);
  console.log("interests last updated: " + interestsLastUpdated);
  let secondsElapsed = blockTime.dividedBy(1000, 10).minus(interestsLastUpdated, 10); // time conversion
  console.log("seconds elapsed: " + secondsElapsed.toFixed());
  // market indices
  let expectedAccumulatedLiquidityInterest = liquidityRate.times(secondsElapsed, 10).dividedBy(SECONDS_PER_YEAR, 10).plus(1, 10);
  let expectedLiquidityIndex = liquidityIndex.times(expectedAccumulatedLiquidityInterest, 10);

  let expectedAccumulatedBorrowInterest = borrowRate.times(secondsElapsed, 10).dividedBy(SECONDS_PER_YEAR, 10).plus(1, 10);
  let expectedBorrowIndex = borrowIndex.times(expectedAccumulatedBorrowInterest, 10);

  // When borrowing, new computed index is used for scaled amount
  let moreDebtScaled = moreDebt.dividedBy(expectedBorrowIndex, 10);
  // When repaying, new computed index is used for scaled amount
  let lessDebtScaled = lessDebt.dividedBy(expectedBorrowIndex, 10);
  let newDebtTotal = new BigNumber(0, 10);
  // NOTE: Don't panic here so that the total repay of debt can be simulated
  // when less debt is greater than outstanding debt
  // TODO: Maybe split index and interest rate calculations and take the needed inputs for each
  if (debtTotalScaled.plus(moreDebtScaled, 10) > lessDebtScaled) {
    newDebtTotal = debtTotalScaled.plus(moreDebtScaled, 10).minus(lessDebtScaled, 10);
    console.log("new debt total: " + newDebtTotal);
  }
  let decDebtTotal = newDebtTotal.times(expectedBorrowIndex, 10);
  console.log("dec debt total: " + decDebtTotal);
  let decLiquidityTotal = initialLiquidity.minus(lessLiquidity, 10);
  let totalLiquidity = decLiquidityTotal.plus(decDebtTotal, 10);
  console.log("total liquidity: " + totalLiquidity);
  let expectedUtilizationRate = !totalLiquidity.isZero() ? decDebtTotal.dividedBy(totalLiquidity, 10) : new BigNumber(0, 10);
  console.log("expected utilization rate: " + expectedUtilizationRate.toFixed());

  // interest rates
  let expectedBorrowRate = expectedUtilizationRate.times(borrowSlope, 10);
  let expectedLiquidityRate = expectedBorrowRate.times(expectedUtilizationRate, 10);

  return {
    expectedLiquidityIndex: expectedLiquidityIndex.toFixed(),
    expectedBorrowIndex: expectedBorrowIndex.toFixed(),
    expectedLiquidityRate: expectedLiquidityRate.toFixed(),
    expectedBorrowRate: expectedBorrowRate.toFixed(),
  }
}

function getRealRates(txResult) {
  let {from_contract} = txResult.logs[0].eventsByType;
  let liquidityRate = from_contract.liquidity_rate[0];
  let borrowRate = from_contract.borrow_rate[0];
  let liquidityIndex = from_contract.liquidity_index[0];
  let borrowIndex = from_contract.borrow_index[0];
  return {liquidityRate, borrowRate, liquidityIndex, borrowIndex}
}

function assertEqualInterestRates(realRates, expectedRates) {
  assertEqual(expectedRates.expectedLiquidityIndex, realRates.liquidityIndex, `Expected liquidity index to be ${expectedRates.expectedLiquidityIndex}, got ${realRates.liquidityIndex}`)
  assertEqual(expectedRates.expectedBorrowIndex, realRates.borrowIndex, `Expected borrow index to be ${expectedRates.expectedBorrowIndex}, got ${realRates.borrowIndex}`);
  assertEqual(expectedRates.expectedLiquidityRate, realRates.liquidityRate, `Expected liquidity rate to be ${expectedRates.expectedLiquidityRate}, got ${realRates.liquidityRate}`);
  assertEqual(expectedRates.expectedBorrowRate, realRates.borrowRate, `Expected borrow rate to be ${expectedRates.expectedBorrowRate}, got ${realRates.borrowRate}`);
}

async function testReserveQuery(terra, address, denom) {
  console.log("### Testing Reserve...")
  let reserveQueryMsg = {"reserve": {"denom": denom}};
  let reserveResult = await queryContract(terra, address, reserveQueryMsg);

  assert(
    reserveResult.hasOwnProperty("ma_token_address"),
    `[Reserve]: Reserve Query for symbol ${denom} failed. Result has no property ma_token_address.`
  )
}

async function depositAssets(terra, wallet, lpContractAddress, deposits) {
  for (let deposit of deposits) {
    let depositMsg = {"deposit_native": {"denom": deposit.denom}};
    let depositAmount = deposit.amount;
    let coins = new Coin(deposit.denom, depositAmount);
    let executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);

    await performTransaction(terra, wallet, executeDepositMsg);
  }
}

async function testInitialDeposit(inputs) {
  let {terra, wallet, initialLiquidity, lpContractAddress} = inputs;
  let {_coins: {uluna: {amount: depositorStartingBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let lunaReserve = await queryContract(terra, lpContractAddress, reserveQueryMsg);
  console.log(lunaReserve);
  let balanceQueryMsg = {"balance": {"address": wallet.key.accAddress}};
  const { balance: depositContractStartingBalance } = await queryContract(terra, lunaReserve.ma_token_address, balanceQueryMsg);

  let depositMsg = {"deposit_native": {"denom": "uluna"}};
  let depositAmount = 10_000_000;
  let coins = new Coin("uluna", depositAmount);
  let executeDepositMsg = new MsgExecuteContract(wallet.key.accAddress, lpContractAddress, depositMsg, [coins]);
  let depositTxResult = await performTransaction(terra, wallet, executeDepositMsg);

  console.log("Deposit Message Sent: ");
  console.log(executeDepositMsg);

  let {timestamp} = await terra.tx.txInfo(depositTxResult.txhash);
  let blockTime = new Date(timestamp).valueOf()

  let realRates = getRealRates(depositTxResult);
  let expectedRates = await getExpectedIndicesAndRates(lunaReserve, blockTime, initialLiquidity, 0, 0, 0);

  console.log(realRates);
  console.log(expectedRates);
  assertEqualInterestRates(realRates, expectedRates);

  initialLiquidity += depositAmount;

  const { balance: depositContractEndingBalance } = await queryContract(terra, lunaReserve.ma_token_address, balanceQueryMsg);
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

  return { initialLiquidity, depositAmount }

}

async function testRedeem(inputs) {
  let {terra, wallet, lpContractAddress, initialLiquidity} = inputs;
  let {_coins: {uluna: {amount: redeemerStartingLunaBalance}}} = await terra.bank.balance(wallet.key.accAddress);

  let reserveQueryMsg = {"reserve": {"denom": "uluna"}};
  let lunaReserve = await queryContract(terra, lpContractAddress, reserveQueryMsg);

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

  console.log("Redeem Message Sent:");
  console.log(redeemSendMsg);

  let redeemTxInfo = await terra.tx.txInfo(redeemTxResult.txhash);
  const redeemTxFee = Number(redeemTxInfo.tx.fee.amount._coins.uluna.amount);

  let blockTime = new Date(redeemTxInfo.timestamp).valueOf()

  let realRates = getRealRates(redeemTxResult);
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

  let realRates = getRealRates(borrowTxResult);
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
  let {_coins: {uluna: {amount: repayerStartingLunaBalance}}} = await terra.bank.balance(repayer.key.accAddress);
  const {debts: debtBeforeRepay} = await queryContract(terra, lpContractAddress, {"debt": {"address": repayer.key.accAddress}});
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

  let realRates = getRealRates(repayTxResult);
  let expectedRates = await getExpectedIndicesAndRates(lunaReserve, blockTime, initialLiquidity, 0, repayAmount, 0);

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

  realRates = getRealRates(overpayTxResult);
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

async function main() {
  const terra = new LocalTerra();
  let wallet = terra.wallets.test1;

  const lpContractAddress = await deploy(terra, wallet);
  const initialAssets = [
    {denom: "uluna", borrow_slope: "4", loan_to_value: "0.5"},
    {denom: "uusd", borrow_slope: "5", loan_to_value: "0.8"},
    {denom: "umnt", borrow_slope: "5", loan_to_value: "0.7"},
    {denom: "ukrw", borrow_slope: "2", loan_to_value: "0.6"},
    {denom: "usdr", borrow_slope: "6", loan_to_value: "0.5"},
  ];
  await setup(terra, wallet, lpContractAddress, {initialAssets});

  await testReserveQuery(terra, lpContractAddress, "uusd")
  await testReserveQuery(terra, lpContractAddress, "uluna");

  console.log("### Testing Config...")
  let configQueryMsg = {"config": {}};
  let configResult = await queryContract(terra, lpContractAddress, configQueryMsg);

  console.log("Config Query Sent:");
  console.log(configQueryMsg);

  assert(
    configResult.hasOwnProperty("ma_token_code_id"),
    "[Config]: Config query failed. Result has no property ma_token_code_id."
  )

  console.log("### Testing Deposit...");
  let depositInputs = {
    terra,
    wallet,
    lpContractAddress,
    initialLiquidity: 0,
  }
  let depositOutput = await testInitialDeposit(depositInputs);


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
}

main().catch(err => console.log(err));
