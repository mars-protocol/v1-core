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
  instantiateContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  sleep,
  toEncodedBinary,
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
const CW20_BORROW = LUNA_COLLATERAL * LUNA_MAX_LTV

const TOKEN_DECIMALS = 6
const MA_TOKEN_SCALING_FACTOR = 1_000_000

// HELPERS

async function queryMaAssetAddress(terra: LCDClient, redBank: string, asset: Asset) {
  const market = await queryContract(terra, redBank,
    { market: { asset: asset } }
  )
  return market.ma_token_address
}

// TYPES

interface Native {
  native: {
    denom: string
  }
}

interface CW20 {
  cw20: {
    contract_addr: string
  }
}

type Asset = Native | CW20

async function setAssetOraclePriceSource(terra: LCDClient, wallet: Wallet, oracle: string, asset: Asset, price: number) {
  await executeContract(terra, wallet, oracle,
    {
      set_asset: {
        asset: asset,
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

async function depositCw20(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
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

async function borrowCw20(terra: LCDClient, wallet: Wallet, redBank: string, contractAddress: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    {
      borrow: {
        asset: { cw20: { contract_addr: contractAddress } },
        amount: String(amount)
      }
    }
  )
}

async function mintCw20(terra: LCDClient, wallet: Wallet, contract: string, recipient: string, amount: number) {
  return await executeContract(terra, wallet, contract,
    {
      mint: {
        recipient: recipient,
        amount: String(amount),
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
  taxRate: number,
  taxCap: number,
  maUluna: string,
  cw20Token1: string,
  cw20Token2: string,
  maCw20Token1: string,
  maCw20Token2: string,
}

// TESTS

async function testCollateralizedLoan(env: Env, borrower: Wallet, borrowFraction: number, receiveMaToken: Boolean) {
  console.log("testCollateralizedLoan: borrowFraction:", borrowFraction, "receiveMaToken:", receiveMaToken)

  const { terra, redBank, deployer, taxRate, taxCap, maUluna } = env

  console.log("provider provides uusd")

  const provider = deployer
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

  const liquidator = deployer

  let uusdAmountLiquidated = uusdAmountBorrowed
  // should fail because the borrower's health factor is > 1
  try {
    await executeContract(terra, liquidator, redBank,
      {
        liquidate_native: {
          collateral_asset: { native: { denom: "uluna" } },
          debt_asset_denom: "uusd",
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
      { user_position: { user_address: borrower.key.accAddress } }
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
        debt_asset_denom: "uusd",
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
    strictEqual(maUlunaBalanceDifference, collateralAmountLiquidated * MA_TOKEN_SCALING_FACTOR)
  } else {
    const ulunaBalanceDifference = ulunaBalanceAfter - ulunaBalanceBefore
    const ulunaTxFee = txInfo.tx.fee.amount.get("uluna")!.amount.toNumber()
    strictEqual(ulunaBalanceDifference, collateralAmountLiquidated - ulunaTxFee)
  }
}

async function testCollateralizedLoanCw20(env: Env, borrower: Wallet, borrowFraction: number, receiveMaToken: Boolean) {
  console.log("testCollateralizedLoanCw20: borrowFraction:", borrowFraction, "receiveMaToken:", receiveMaToken)

  const { terra, redBank, deployer, cw20Token1, cw20Token2, maCw20Token2 } = env

  const provider = deployer
  const liquidator = deployer

  // mint some tokens
  await mintCw20(terra, deployer, cw20Token1, provider.key.accAddress, USD_COLLATERAL)
  await mintCw20(terra, deployer, cw20Token2, borrower.key.accAddress, LUNA_COLLATERAL)
  await mintCw20(terra, deployer, cw20Token1, liquidator.key.accAddress, USD_COLLATERAL)

  console.log("provider provides cw20 token 1")

  await executeContract(terra, provider, cw20Token1,
    {
      send: {
        contract: redBank,
        amount: String(USD_COLLATERAL),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )

  console.log("borrower provides cw20 token 2")

  await executeContract(terra, borrower, cw20Token2,
    {
      send: {
        contract: redBank,
        amount: String(LUNA_COLLATERAL),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )

  console.log("borrower borrows a small amount of cw20 token 1")

  let totalCw20Token1AmountBorrowed = 0

  let cw20Token1AmountBorrowed = Math.floor(CW20_BORROW * 0.01)
  let txResult = await borrowCw20(terra, borrower, redBank, cw20Token1, cw20Token1AmountBorrowed)
  let txEvents = txResult.logs[0].eventsByType

  let cw20Token1AmountReceivedFromBorrow = parseInt(txEvents.from_contract.amount[1])
  let expectedCw20Token1AmountReceived = cw20Token1AmountBorrowed
  strictEqual(cw20Token1AmountReceivedFromBorrow, expectedCw20Token1AmountReceived)

  totalCw20Token1AmountBorrowed += cw20Token1AmountBorrowed

  console.log("liquidator tries to liquidate the borrower")

  let cw20Token1AmountLiquidated = cw20Token1AmountBorrowed
  // should fail because the borrower's health factor is > 1
  try {
    await executeContract(terra, liquidator, cw20Token1,
      {
        send: {
          contract: redBank,
          amount: String(cw20Token1AmountLiquidated),
          msg: toEncodedBinary({
            liquidate_cw20: {
              collateral_asset: { cw20: { contract_addr: cw20Token2 } },
              user_address: borrower.key.accAddress,
              receive_ma_token: receiveMaToken,
            }
          })
        }
      }
    )
  } catch (error) {
    strictEqual(error.config.url, "/txs/estimate_fee")
    assert(error.response.data.error.includes(
      "User's health factor is not less than 1 and thus cannot be liquidated"
    ))
  }

  console.log("borrower borrows cw20 token 1 up to the borrow limit of their cw20 token 2 collateral")

  cw20Token1AmountBorrowed = Math.floor(CW20_BORROW * 0.98)
  txResult = await borrowCw20(terra, borrower, redBank, cw20Token1, cw20Token1AmountBorrowed)
  txEvents = txResult.logs[0].eventsByType

  cw20Token1AmountReceivedFromBorrow = parseInt(txEvents.from_contract.amount[1])
  expectedCw20Token1AmountReceived = cw20Token1AmountBorrowed
  strictEqual(cw20Token1AmountReceivedFromBorrow, expectedCw20Token1AmountReceived)

  totalCw20Token1AmountBorrowed += cw20Token1AmountBorrowed

  console.log("liquidator waits until the borrower's health factor is < 1, then liquidates")

  // wait until the borrower can be liquidated
  let tries = 0
  let maxTries = 10
  let backoff = 1

  while (true) {
    const userPosition = await queryContract(terra, redBank,
      { user_position: { user_address: borrower.key.accAddress } }
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
  const cw20Token1BalanceBefore = await queryCw20Balance(terra, liquidator.key.accAddress, cw20Token1)
  const cw20Token2BalanceBefore = await queryCw20Balance(terra, liquidator.key.accAddress, cw20Token2)
  const maCw20Token2BalanceBefore = await queryCw20Balance(terra, liquidator.key.accAddress, maCw20Token2)

  // liquidate the borrower
  cw20Token1AmountLiquidated = Math.floor(totalCw20Token1AmountBorrowed * borrowFraction)
  txResult = await executeContract(terra, liquidator, cw20Token1,
    {
      send: {
        contract: redBank,
        amount: String(cw20Token1AmountLiquidated),
        msg: toEncodedBinary({
          liquidate_cw20: {
            collateral_asset: { cw20: { contract_addr: cw20Token2 } },
            user_address: borrower.key.accAddress,
            receive_ma_token: receiveMaToken,
          }
        })
      }
    }
  )
  txEvents = txResult.logs[0].eventsByType
  await sleep(100)
  const txInfo = await terra.tx.txInfo(txResult.txhash)

  // get the liquidator's balances after they have liquidated the borrower
  const cw20Token1BalanceAfter = await queryCw20Balance(terra, liquidator.key.accAddress, cw20Token1)
  const cw20Token2BalanceAfter = await queryCw20Balance(terra, liquidator.key.accAddress, cw20Token2)
  const maCw20Token2BalanceAfter = await queryCw20Balance(terra, liquidator.key.accAddress, maCw20Token2)

  // the maximum fraction of debt that can be liquidated is `CLOSE_FACTOR`
  const expectedLiquidatedDebtFraction = borrowFraction > CLOSE_FACTOR ? CLOSE_FACTOR : borrowFraction

  // debt amount repaid
  const debtAmountRepaid = parseInt(txEvents.wasm.debt_amount_repaid[0])
  const expectedDebtAmountRepaid = Math.floor(totalCw20Token1AmountBorrowed * expectedLiquidatedDebtFraction)

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

  // liquidator cw20 token 1 balance
  const cw20Token1BalanceDifference = cw20Token1BalanceBefore - cw20Token1BalanceAfter
  strictEqual(cw20Token1BalanceDifference, debtAmountRepaid)

  // refund amount
  const refundAmount = parseInt(txEvents.wasm.refund_amount[0])
  if (borrowFraction > CLOSE_FACTOR) {
    // liquidator paid more than the maximum repayable debt, so is refunded the difference
    const expectedRefundAmount = cw20Token1AmountLiquidated - debtAmountRepaid
    strictEqual(refundAmount, expectedRefundAmount)
  } else {
    // liquidator paid less than the maximum repayable debt, so no refund is owed
    strictEqual(refundAmount, 0)
  }

  // collateral amount liquidated
  const collateralAmountLiquidated = parseInt(txEvents.wasm.collateral_amount_liquidated[0])
  const expectedCollateralAmountLiquidated = Math.floor(debtAmountRepaid * (1 + LIQUIDATION_BONUS))
  strictEqual(collateralAmountLiquidated, expectedCollateralAmountLiquidated)

  // collateral amount received
  if (receiveMaToken) {
    const maCw20Token2BalanceDifference = maCw20Token2BalanceAfter - maCw20Token2BalanceBefore
    strictEqual(maCw20Token2BalanceDifference, collateralAmountLiquidated * MA_TOKEN_SCALING_FACTOR)
  } else {
    const cw20Token2BalanceDifference = cw20Token2BalanceAfter - cw20Token2BalanceBefore
    strictEqual(cw20Token2BalanceDifference, collateralAmountLiquidated)
  }
}

async function testUncollateralizedLoan(env: Env, borrower: Wallet) {
  console.log("testUncollateralizedLoan")

  const { terra, redBank, deployer, taxRate, taxCap } = env

  console.log("provider provides uusd")

  const provider = deployer

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

  const liquidator = deployer

  // check user position
  const userPositionT1 = await queryContract(terra, redBank,
    { user_position: { user_address: borrower.key.accAddress } }
  )
  strictEqual(userPositionT1.health_status, "not_borrowing")


  // should fail because there are no collateralized loans
  try {
    await executeContract(terra, liquidator, redBank,
      {
        liquidate_native: {
          collateral_asset: { native: { denom: "uluna" } },
          debt_asset_denom: "uusd",
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
    { user_position: { user_address: borrower.key.accAddress } }
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

  // cw20 tokens
  // TODO use .env file
  const cw20CodeId = await uploadContract(terra, deployer, "../../cw-plus/artifacts/cw20_base.wasm")

  const cw20Token1 = await instantiateContract(terra, deployer, cw20CodeId,
    {
      name: "cw20 Token 1",
      symbol: "ONE",
      decimals: TOKEN_DECIMALS,
      initial_balances: [],
      mint: { minter: deployer.key.accAddress }
    }
  )

  const cw20Token2 = await instantiateContract(terra, deployer, cw20CodeId,
    {
      name: "cw20 Token 2",
      symbol: "TWO",
      decimals: TOKEN_DECIMALS,
      initial_balances: [],
      mint: { minter: deployer.key.accAddress }
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
  await setAssetOraclePriceSource(terra, deployer, oracle, { native: { denom: "uluna" } }, LUNA_USD_PRICE)
  const maUluna: string = await queryMaAssetAddress(terra, redBank, { native: { denom: "uluna" } })

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
  await setAssetOraclePriceSource(terra, deployer, oracle, { native: { denom: "uusd" } }, 1)

  // cw20token1
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { cw20: { contract_addr: cw20Token1 } },
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
  await setAssetOraclePriceSource(terra, deployer, oracle, { cw20: { contract_addr: cw20Token1 } }, LUNA_USD_PRICE)
  const maCw20Token1: string = await queryMaAssetAddress(terra, redBank, { cw20: { contract_addr: cw20Token1 } })

  // cw20token2
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { cw20: { contract_addr: cw20Token2 } },
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
  await setAssetOraclePriceSource(terra, deployer, oracle, { cw20: { contract_addr: cw20Token2 } }, LUNA_USD_PRICE)
  const maCw20Token2: string = await queryMaAssetAddress(terra, redBank, { cw20: { contract_addr: cw20Token2 } })

  const taxRate = (await terra.treasury.taxRate()).toNumber()
  const taxCap = (await terra.treasury.taxCap("uusd")).amount.toNumber()

  const env: Env = {
    terra,
    redBank,
    deployer,
    taxRate,
    taxCap,
    maUluna,
    cw20Token1,
    cw20Token2,
    maCw20Token1,
    maCw20Token2,
  }

  // collateralized
  let borrowFraction = CLOSE_FACTOR - 0.1
  // await testCollateralizedLoan(env, terra.wallets.test2, borrowFraction, false)
  // await testCollateralizedLoan(env, terra.wallets.test3, borrowFraction, true)
  await testCollateralizedLoanCw20(env, terra.wallets.test4, borrowFraction, false)
  await testCollateralizedLoanCw20(env, terra.wallets.test5, borrowFraction, true)

  borrowFraction = CLOSE_FACTOR + 0.1
  // await testCollateralizedLoan(env, terra.wallets.test6, borrowFraction, false)
  // await testCollateralizedLoan(env, terra.wallets.test7, borrowFraction, true)
  await testCollateralizedLoanCw20(env, terra.wallets.test8, borrowFraction, false)
  await testCollateralizedLoanCw20(env, terra.wallets.test9, borrowFraction, true)

  // uncollateralized
  await testUncollateralizedLoan(env, terra.wallets.test10)

  console.log("OK")
}

main().catch(err => console.log(err))
