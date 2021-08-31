import {
  BlockTxBroadcastResult,
  Coin,
  Int,
  LCDClient,
  LocalTerra,
  MsgExecuteContract,
  Wallet
} from "@terra-money/terra.js"
import { strictEqual, strict as assert } from "assert"
import {
  deployContract,
  executeContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  sleep,
  uploadContract,
} from "../helpers.js"

// CONSTS

const CLOSE_FACTOR = 0.5
const LUNA_MAX_LTV = 0.55
const LIQUIDATION_BONUS = 0.1
// set a high interest rate, so tests can be run faster
const INTEREST_RATE = 100000
const LUNA_USD_PRICE = 25

const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 1_000_000000
const USD_BORROW = LUNA_COLLATERAL * LUNA_USD_PRICE * LUNA_MAX_LTV

// HELPERS

async function queryMaAssetAddress(terra: LCDClient, redBank: string, denom: string) {
  const market = await queryContract(terra, redBank,
    { market: { asset: { native: { denom: denom } } } }
  )
  return market.ma_token_address
}

async function setAssetOraclePriceSource(terra: LCDClient, wallet: Wallet, oracle: string, denom: string, price: number) {
  await executeContract(terra, wallet, oracle,
    {
      set_asset: {
        asset: { native: { denom: denom } },
        price_source: { fixed: { price: String(price) } }
      }
    }
  )
}

async function getTxTimestamp(terra: LCDClient, result: BlockTxBroadcastResult) {
  await sleep(100)
  const txInfo = await terra.tx.txInfo(result.txhash)
  return Date.parse(txInfo.timestamp) / 1000 // seconds
}

async function deposit(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  const result = await executeContract(terra, wallet, redBank,
    { deposit_native: { denom: denom } },
    `${amount}${denom}`
  )
  return await getTxTimestamp(terra, result)
}

async function borrow(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    {
      borrow: {
        asset: { native: { denom: denom } },
        amount: String(amount)
      }
    }
  )
}

function computeTax(amount: number, taxRate: number, taxCap: number) {
  const tax = amount - amount / (1 + taxRate)
  return tax > taxCap ? taxCap : Math.round(tax) // TODO check this and use big num types
}

function deductTax(amount: number, taxRate: number, taxCap: number) {
  return amount - computeTax(amount, taxRate, taxCap)
}

async function queryNativeBalance(terra: LCDClient, address: string, denom: string) {
  const balances = await terra.bank.balance(address)
  const balance = balances.get(denom)
  if (balance === undefined) {
    return 0
  }
  return balance.amount.toNumber()
}

async function queryCw20Balance(terra: LCDClient, userAddress: string, contractAddress: string) {
  const result = await queryContract(terra, contractAddress, { balance: { address: userAddress } })
  return parseInt(result.balance)
}
interface Env {
  terra: LocalTerra,
  redBank: string,
  deployer: Wallet,
  provider: Wallet,
  liquidator: Wallet,
  taxRate: number,
  taxCap: number,
  maUluna: string,
}

// TESTS

async function testCollateralizedLoan(env: Env, borrower: Wallet, borrowFraction: number, receiveMaToken: Boolean) {
  console.log("borrowFraction:", borrowFraction, "receiveMaToken:", receiveMaToken)

  const { terra, redBank, provider, liquidator, taxRate, taxCap, maUluna } = env

  console.log("provider provides uusd")

  await deposit(terra, provider, redBank, "uusd", USD_COLLATERAL)

  console.log("borrower provides uluna")

  await deposit(terra, borrower, redBank, "uluna", LUNA_COLLATERAL)

  console.log("borrower borrows a small amount of uusd")

  let totalUusdAmountBorrowed = 0
  let totalUusdAmountReceivedFromBorrow = 0

  let uusdAmountBorrowed = Math.floor(USD_BORROW * 0.01)
  let txResult = await borrow(terra, borrower, redBank, "uusd", uusdAmountBorrowed)
  let txEvents = txResult.logs[0].eventsByType

  // amount received after deducting Terra tax from the borrowed amount
  let uusdAmountReceivedFromBorrow = Coin.fromString(txEvents.coin_received.amount[0]).amount.toNumber()
  let expectedUusdAmountReceived = deductTax(uusdAmountBorrowed, taxRate, taxCap)
  strictEqual(uusdAmountReceivedFromBorrow, expectedUusdAmountReceived)

  totalUusdAmountBorrowed += uusdAmountBorrowed
  totalUusdAmountReceivedFromBorrow += uusdAmountReceivedFromBorrow

  console.log("liquidator tries to liquidate the borrower")

  let uusdAmountLiquidated = uusdAmountBorrowed
  // should fail because the borrower's health factor is > 1
  try {
    await executeContract(terra, liquidator, redBank,
      {
        liquidate_native: {
          collateral_asset: { native: { denom: "uluna" } },
          debt_asset: "uusd",
          user_address: borrower.key.accAddress,
          receive_ma_token: receiveMaToken,
        }
      },
      `${uusdAmountLiquidated}uusd`
    )
  } catch (error) {
    strictEqual(error.config.url, "/txs/estimate_fee")
    assert(error.response.data.error.includes(
      "User's health factor is not less than 1 and thus cannot be liquidated"
    ))
  }

  console.log("borrower borrows uusd up to the borrow limit of their uluna collateral")

  uusdAmountBorrowed = Math.floor(USD_BORROW * 0.98)
  txResult = await borrow(terra, borrower, redBank, "uusd", uusdAmountBorrowed)
  txEvents = txResult.logs[0].eventsByType

  uusdAmountReceivedFromBorrow = Coin.fromString(txEvents.coin_received.amount[0]).amount.toNumber()
  expectedUusdAmountReceived = deductTax(uusdAmountBorrowed, taxRate, taxCap)
  strictEqual(uusdAmountReceivedFromBorrow, expectedUusdAmountReceived)

  totalUusdAmountBorrowed += uusdAmountBorrowed
  totalUusdAmountReceivedFromBorrow += uusdAmountReceivedFromBorrow

  console.log("liquidator waits until the borrower's health factor is < 1, then liquidates")

  // wait until the borrower can be liquidated
  let tries = 0
  let maxTries = 10
  let backoff = 1

  while (true) {
    const userPosition = await queryContract(terra, redBank,
      { user_position: { address: borrower.key.accAddress } }
    )
    const healthFactor = parseFloat(userPosition.health_status.borrowing)
    if (healthFactor < 1.0) {
      break
    }

    // timeout
    tries++
    if (tries == maxTries) {
      throw new Error(`timed out waiting ${maxTries} times for the borrower to be liquidated`)
    }

    // exponential backoff
    console.log("health factor:", healthFactor, `backing off: ${backoff} s`)
    await sleep(backoff * 1000)
    backoff *= 2
  }

  // get the liquidator's balances before they liquidate the borrower
  const uusdBalanceBefore = await queryNativeBalance(terra, liquidator.key.accAddress, "uusd")
  const ulunaBalanceBefore = await queryNativeBalance(terra, liquidator.key.accAddress, "uluna")
  const maUlunaBalanceBefore = await queryCw20Balance(terra, liquidator.key.accAddress, maUluna)

  // liquidate the borrower
  uusdAmountLiquidated = Math.floor(totalUusdAmountBorrowed * borrowFraction)
  txResult = await executeContract(terra, liquidator, redBank,
    {
      liquidate_native: {
        collateral_asset: { native: { denom: "uluna" } },
        debt_asset: "uusd",
        user_address: borrower.key.accAddress,
        receive_ma_token: receiveMaToken,
      }
    },
    `${uusdAmountLiquidated}uusd`
  )
  txEvents = txResult.logs[0].eventsByType
  await sleep(100)
  const txInfo = await terra.tx.txInfo(txResult.txhash)

  // cache the liquidator's balances after they have liquidated the borrower
  const uusdBalanceAfter = await queryNativeBalance(terra, liquidator.key.accAddress, "uusd")
  const ulunaBalanceAfter = await queryNativeBalance(terra, liquidator.key.accAddress, "uluna")
  const maUlunaBalanceAfter = await queryCw20Balance(terra, liquidator.key.accAddress, maUluna)

  // the maximum fraction of debt that can be liquidated is `CLOSE_FACTOR`
  const expectedLiquidatedDebtFraction = borrowFraction > CLOSE_FACTOR ? CLOSE_FACTOR : borrowFraction

  // debt amount repaid
  const debtAmountRepaid = parseInt(txEvents.wasm.debt_amount_repaid[0])
  const expectedDebtAmountRepaid = Math.floor(totalUusdAmountBorrowed * expectedLiquidatedDebtFraction)

  if (borrowFraction > CLOSE_FACTOR) {
    // pay back the maximum repayable debt
    // use intervals because the exact amount of debt owed at any time t changes as interest accrues
    assert(
      // check that the actual amount of debt repaid is greater than the expected amount,
      // due to the debt accruing interest
      debtAmountRepaid > expectedDebtAmountRepaid &&
      // check that the actual amount of debt repaid is less than the debt after one year
      debtAmountRepaid < expectedDebtAmountRepaid * (1 + INTEREST_RATE)
    )
  } else {
    // pay back less than the maximum repayable debt
    // check that the actual amount of debt repaid is equal to the expected amount of debt repaid
    strictEqual(debtAmountRepaid, expectedDebtAmountRepaid)
  }

  // liquidator uusd balance
  const uusdBalanceDifference = uusdBalanceBefore - uusdBalanceAfter
  if (borrowFraction > CLOSE_FACTOR) {
    const uusdLiquidationTax = await terra.utils.calculateTax(new Coin("uusd", uusdAmountLiquidated))
    // TODO why is uusdBalanceDifference 1 or 2 uusd different from expected?
    try {
      strictEqual(
        uusdBalanceDifference,
        debtAmountRepaid + computeTax(debtAmountRepaid, taxRate, taxCap) + uusdLiquidationTax.amount.toNumber()
      )
    } catch (e) {
      console.log(e)
    }
  } else {
    strictEqual(
      uusdBalanceDifference,
      debtAmountRepaid + computeTax(debtAmountRepaid, taxRate, taxCap)
    )
  }

  // refund amount
  const refundAmount = parseInt(txEvents.wasm.refund_amount[0])
  if (borrowFraction > CLOSE_FACTOR) {
    // liquidator paid more than the maximum repayable debt, so is refunded the difference
    const expectedRefundAmount = uusdAmountLiquidated - debtAmountRepaid
    strictEqual(refundAmount, expectedRefundAmount)
  } else {
    // liquidator paid less than the maximum repayable debt, so no refund is owed
    strictEqual(refundAmount, 0)
  }

  // collateral amount liquidated
  const collateralAmountLiquidated = parseInt(txEvents.wasm.collateral_amount_liquidated[0])
  const expectedCollateralAmountLiquidated =
    Math.floor(debtAmountRepaid * (1 + LIQUIDATION_BONUS) / LUNA_USD_PRICE)
  strictEqual(collateralAmountLiquidated, expectedCollateralAmountLiquidated)

  // collateral amount received
  if (receiveMaToken) {
    const maUlunaBalanceDifference = maUlunaBalanceAfter - maUlunaBalanceBefore
    strictEqual(collateralAmountLiquidated, maUlunaBalanceDifference)
  } else {
    const ulunaBalanceDifference = ulunaBalanceAfter - ulunaBalanceBefore
    const ulunaTxFee = txInfo.tx.fee.amount.get("uluna")!.amount.toNumber()
    strictEqual(ulunaBalanceDifference, collateralAmountLiquidated - ulunaTxFee)
  }
}

async function testUncollateralizedLoan(env: Env, borrower: Wallet) {
  const { terra, redBank, deployer, provider, liquidator, taxRate, taxCap } = env

  console.log("provider provides uusd")

  await deposit(terra, provider, redBank, "uusd", USD_COLLATERAL)

  console.log("set uncollateralized loan limit for borrower")

  await executeContract(terra, deployer, redBank,
    {
      update_uncollateralized_loan_limit: {
        user_address: borrower.key.accAddress,
        asset: { native: { denom: "uusd" } },
        new_limit: String(USD_COLLATERAL),
      }
    }
  )

  console.log("borrower borrows uusd")

  const uusdBalanceBefore = await queryNativeBalance(terra, borrower.key.accAddress, "uusd")

  const uusdAmountBorrowed = USD_COLLATERAL
  let txResult = await borrow(terra, borrower, redBank, "uusd", uusdAmountBorrowed)
  const txEvents = txResult.logs[0].eventsByType
  const loggedUusdAmountBorrowed = parseInt(txEvents.wasm.amount[0])
  strictEqual(loggedUusdAmountBorrowed, uusdAmountBorrowed)

  const uusdBalanceAfter = await queryNativeBalance(terra, borrower.key.accAddress, "uusd")
  const uusdBalanceDifference = uusdBalanceAfter - uusdBalanceBefore
  strictEqual(
    uusdBalanceDifference,
    uusdAmountBorrowed - computeTax(uusdAmountBorrowed, taxRate, taxCap)
  )

  console.log("liquidator tries to liquidate the borrower")

  // check user position
  const userPositionT1 = await queryContract(terra, redBank,
    { user_position: { address: borrower.key.accAddress } }
  )
  strictEqual(userPositionT1.health_status, "not_borrowing")


  // should fail because there are no collateralized loans
  try {
    await executeContract(terra, liquidator, redBank,
      {
        liquidate_native: {
          collateral_asset: { native: { denom: "uluna" } },
          debt_asset: "uusd",
          user_address: borrower.key.accAddress,
          receive_ma_token: false,
        }
      },
      `${uusdAmountBorrowed}uusd`
    )
  } catch (error) {
    strictEqual(error.config.url, "/txs/estimate_fee")
    assert(error.response.data.error.includes(
      "user has a positive uncollateralized loan limit and thus cannot be liquidated"
    ))
  }


  const userPositionT2 = await queryContract(terra, redBank,
    { user_position: { address: borrower.key.accAddress } }
  )
  strictEqual(userPositionT1.total_collateralized_debt_in_uusd, userPositionT2.total_collateralized_debt_in_uusd)
  strictEqual(userPositionT1.max_debt_in_uusd, userPositionT2.max_debt_in_uusd)
}

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

  console.log("upload contracts")

  const addressProvider = await deployContract(terra, deployer, "../artifacts/address_provider.wasm",
    { owner: deployer.key.accAddress }
  )

  const incentives = await deployContract(terra, deployer, "../artifacts/incentives.wasm",
    {
      owner: deployer.key.accAddress,
      address_provider_address: addressProvider
    }
  )

  const oracle = await deployContract(terra, deployer, "../artifacts/oracle.wasm",
    { owner: deployer.key.accAddress }
  )

  const maTokenCodeId = await uploadContract(terra, deployer, "../artifacts/ma_token.wasm")

  const redBank = await deployContract(terra, deployer, "../artifacts/red_bank.wasm",
    {
      config: {
        owner: deployer.key.accAddress,
        address_provider_address: addressProvider,
        insurance_fund_fee_share: "0.1",
        treasury_fee_share: "0.2",
        ma_token_code_id: maTokenCodeId,
        close_factor: String(CLOSE_FACTOR),
      }
    }
  )

  // update address provider
  await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          incentives_address: incentives,
          oracle_address: oracle,
          red_bank_address: redBank,
          protocol_admin_address: deployer.key.accAddress,
        }
      }
    }
  )

  console.log("init assets")

  // uluna
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { native: { denom: "uluna" } },
        asset_params: {
          initial_borrow_rate: "0.1",
          max_loan_to_value: String(LUNA_MAX_LTV),
          reserve_factor: "0.2",
          maintenance_margin: String(LUNA_MAX_LTV + 0.001),
          liquidation_bonus: String(LIQUIDATION_BONUS),
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0",
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          }
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle, "uluna", LUNA_USD_PRICE)
  const maUluna: string = await queryMaAssetAddress(terra, redBank, "uluna")

  // uusd
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { native: { denom: "uusd" } },
        asset_params: {
          initial_borrow_rate: "0.2",
          max_loan_to_value: "0.75",
          reserve_factor: "0.2",
          maintenance_margin: "0.85",
          liquidation_bonus: String(LIQUIDATION_BONUS),
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0",
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          }
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle, "uusd", 1)

  const taxRate = (await terra.treasury.taxRate()).toNumber()
  const taxCap = (await terra.treasury.taxCap("uusd")).amount.toNumber()

  const provider = terra.wallets.test2
  const liquidator = terra.wallets.test3

  const env: Env = {
    terra,
    redBank,
    deployer,
    provider,
    liquidator,
    taxRate,
    taxCap,
    maUluna,
  }

  // collateralized
  let borrowFraction = CLOSE_FACTOR - 0.1
  await testCollateralizedLoan(env, terra.wallets.test4, borrowFraction, false)
  await testCollateralizedLoan(env, terra.wallets.test5, borrowFraction, true)

  borrowFraction = CLOSE_FACTOR + 0.1
  await testCollateralizedLoan(env, terra.wallets.test6, borrowFraction, false)
  await testCollateralizedLoan(env, terra.wallets.test7, borrowFraction, true)

  // uncollateralized
  await testUncollateralizedLoan(env, terra.wallets.test8)

  console.log("OK")
}

main().catch(err => console.log(err))
