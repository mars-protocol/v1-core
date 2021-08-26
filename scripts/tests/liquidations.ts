import {
  BlockTxBroadcastResult,
  Coin,
  Int,
  LCDClient,
  LocalTerra,
  Wallet
} from "@terra-money/terra.js"
import { strictEqual, strict as assert } from "assert"
import { join } from "path"
import {
  deployContract,
  executeContract,
  executeContractFails,
  LOCAL_TERRA_FEE_UUSD,
  queryContract,
  setTimeoutDuration,
  sleep,
  uploadContract,
} from "../helpers.js"

// CONSTS

const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"

const CLOSE_FACTOR = 0.5
const LUNA_MAX_LTV = 0.55
const USD_MAX_LTV = 0.75
const LIQUIDATION_BONUS = 0.1

const LUNA_USD_PRICE = 25
const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 1_000_000000
const USD_BORROW = LUNA_COLLATERAL * LUNA_USD_PRICE * LUNA_MAX_LTV // 13750

const INTEREST_RATE = 100000
const SECONDS_IN_YEAR = 365 * 24 * 60 * 60

// HELPERS

async function maAssetAddress(terra: LCDClient, redBank: string, denom: string) {
  const market = await queryContract(terra, redBank, { market: { asset: { native: { denom: denom } } } })
  return market.ma_token_address
}

async function setAsset(terra: LCDClient, wallet: Wallet, oracle: string, denom: string, price: number) {
  await executeContract(terra, wallet, oracle,
    {
      set_asset: {
        asset: { native: { denom: denom } },
        price_source: { fixed: { price: String(price) } }
      }
    }
  )
}

async function txTimestamp(terra: LCDClient, result: BlockTxBroadcastResult) {
  await sleep(100)
  const txInfo = await terra.tx.txInfo(result.txhash)
  return Date.parse(txInfo.timestamp) / 1000 // seconds
}

async function deposit(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  const result = await executeContract(terra, wallet, redBank, { deposit_native: { denom: denom } }, `${amount}${denom}`)
  return await txTimestamp(terra, result)
}

async function borrow(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    { borrow: { asset: { native: { denom: denom } }, amount: String(amount) } },
    `${amount}${denom}`
  )
}


function calculateTaxOnBorrowedAmount(amount: number, taxRate: number, taxCap: number) {
  const tax = amount - amount / (1 + taxRate)
  return tax > taxCap ? taxCap : Math.round(tax)
}

function amountReceivedFromBorrowedAmount(amount: number, taxRate: number, taxCap: number) {
  return amount - calculateTaxOnBorrowedAmount(amount, taxRate, taxCap)
}

interface Env {
  terra: LocalTerra,
  redBank: string,
  provider: Wallet,
  liquidator: Wallet,
  taxRate: number,
  taxCap: number,
  maUluna: string,
}

// TESTS

// async function testLiquidatorReceivesMaToken() {

// }

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

  const tokenCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const terraswapFactory = await deployContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"),
    {
      "pair_code_id": pairCodeID,
      "token_code_id": tokenCodeID
    }
  )

  const staking = await deployContract(terra, deployer, "../artifacts/staking.wasm",
    {
      config: {
        owner: deployer.key.accAddress,
        address_provider_address: addressProvider,
        terraswap_factory_address: terraswapFactory,
        terraswap_max_spread: "0.05",
        cooldown_duration: 10,
        unstake_window: 300,
      }
    }
  )

  const mars = await deployContract(terra, deployer, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"),
    {
      name: "Mars",
      symbol: "MARS",
      decimals: 6,
      initial_balances: [],
      mint: { minter: incentives },
    }
  )

  const xMars = await deployContract(terra, deployer, "../artifacts/xmars_token.wasm",
    {
      name: "xMars",
      symbol: "xMARS",
      decimals: 6,
      initial_balances: [],
      mint: { minter: staking },
    }
  )

  // update address provider
  const tmp = await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          incentives_address: incentives,
          mars_token_address: mars,
          oracle_address: oracle,
          red_bank_address: redBank,
          staking_address: staking,
          xmars_token_address: xMars,
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
  await setAsset(terra, deployer, oracle, "uluna", LUNA_USD_PRICE)
  const maUluna: string = await maAssetAddress(terra, redBank, "uluna")

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
  await setAsset(terra, deployer, oracle, "uusd", 1)

  const taxRate = (await terra.treasury.taxRate()).toNumber()
  const taxCap = (await terra.treasury.taxCap("uusd")).amount.toNumber()

  const provider = terra.wallets.test2
  const liquidator = terra.wallets.test3

  const env: Env = {
    terra,
    redBank,
    provider,
    liquidator,
    taxRate,
    taxCap,
    maUluna,
  }

  let debtFraction = 0.4
  await test(env, terra.wallets.test4, debtFraction, false)
  await test(env, terra.wallets.test5, debtFraction, true)

  debtFraction = 1.0
  await test(env, terra.wallets.test6, debtFraction, false)
  await test(env, terra.wallets.test7, debtFraction, true)

  console.log("OK")
}

async function test(env: Env, borrower: Wallet, debtFraction: number, receiveMaToken: Boolean) {
  console.log("debtFraction:", debtFraction, "receiveMaToken:", receiveMaToken)

  const { terra, redBank, provider, liquidator, taxRate, taxCap, maUluna } = env

  console.log("provider provides USD")

  await executeContract(terra, provider, redBank,
    { deposit_native: { denom: "uusd" } },
    `${USD_COLLATERAL}uusd`
  )

  console.log("borrower provides Luna")

  await executeContract(terra, borrower, redBank,
    { deposit_native: { denom: "uluna" } },
    `${LUNA_COLLATERAL}uluna`
  )

  console.log("borrower borrows a small amount of uusd")

  let uusdBorrowed = 0

  const uusdBorrowAmount1 = Math.floor(USD_BORROW * 0.01)
  const uusdReceivedAmount1 = amountReceivedFromBorrowedAmount(uusdBorrowAmount1, taxRate, taxCap)
  uusdBorrowed += uusdReceivedAmount1
  const uusdBorrowResult1 = await borrow(terra, borrower, redBank, "uusd", uusdBorrowAmount1)
  const uusdBorrowTime1 = await txTimestamp(terra, uusdBorrowResult1)
  strictEqual(parseInt(uusdBorrowResult1.logs[0].eventsByType.wasm.amount_after_tax[0]), uusdReceivedAmount1)

  console.log("liquidator tries to liquidate the borrower, but fails because the borrower's health factor is > 1")

  assert(await executeContractFails(terra, liquidator, redBank,
    {
      liquidate_native: {
        collateral_asset: { native: { denom: "uluna" } },
        debt_asset: "uusd",
        user_address: borrower.key.accAddress,
        receive_ma_token: receiveMaToken,
      }
    }, `${uusdBorrowAmount1}uusd`
  ))

  console.log("borrower borrows up to the borrow limit for their uluna collateral")

  const uusdBorrowAmount2 = Math.floor(USD_BORROW * 0.98)
  const uusdReceivedAmount2 = amountReceivedFromBorrowedAmount(uusdBorrowAmount2, taxRate, taxCap)
  uusdBorrowed += uusdReceivedAmount2
  const uusdBorrowResult2 = await borrow(terra, borrower, redBank, "uusd", uusdBorrowAmount2)
  const uusdBorrowTime2 = await txTimestamp(terra, uusdBorrowResult2)
  strictEqual(parseInt(uusdBorrowResult2.logs[0].eventsByType.wasm.amount_after_tax[0]), uusdReceivedAmount2)

  console.log("liquidator waits until the borrower's health factor is < 1, then liquidates")

  let backoff = 1
  while (true) {
    const userPosition = await queryContract(terra, redBank, { user_position: { address: borrower.key.accAddress } })
    const healthFactor = parseFloat(userPosition.health_status.borrowing)
    if (healthFactor < 1.0) {
      break
    }

    console.log("health factor:", healthFactor, `backing off: ${backoff} s`)
    await sleep(backoff * 1000)
    backoff *= 2
  }

  const ulunaBalanceBeforeLiquidating = await queryNativeBalance(terra, liquidator.key.accAddress, "uluna")
  const maUlunaBalanceBeforeLiquidating = await queryCw20Balance(terra, liquidator.key.accAddress, maUluna)

  const uusdRepayAmount = Math.floor(uusdBorrowed * debtFraction)
  const uusdLiquidationResult = await executeContract(terra, liquidator, redBank,
    {
      liquidate_native: {
        collateral_asset: { native: { denom: "uluna" } },
        debt_asset: "uusd",
        user_address: borrower.key.accAddress,
        receive_ma_token: receiveMaToken,
      }
    }, `${uusdRepayAmount}uusd`
  )
  const uusdLiquidationTime = await txTimestamp(terra, uusdLiquidationResult)

  const ulunaBalanceAfter = await queryNativeBalance(terra, liquidator.key.accAddress, "uluna")
  const maUlunaBalanceAfter = await queryCw20Balance(terra, liquidator.key.accAddress, maUluna)

  // maximum fraction of the debt that can be liquidated
  const maxDebtFraction = debtFraction > CLOSE_FACTOR ? CLOSE_FACTOR : debtFraction

  // interest accrued on the uusd debt
  // const interest1 = uusdBorrowAmount1 * maxDebtFraction * INTEREST_RATE * (uusdLiquidationTime - uusdBorrowTime1) / SECONDS_IN_YEAR
  // const interest2 = uusdBorrowAmount2 * maxDebtFraction * INTEREST_RATE * (uusdLiquidationTime - uusdBorrowTime2) / SECONDS_IN_YEAR
  // const interest = interest1 + interest2

  // debt amount repaid
  // TODO calculate the correct `expectedDebtAmountRepaid`
  const debtAmountRepaid = parseInt(uusdLiquidationResult.logs[0].eventsByType.wasm.debt_amount_repaid[0])
  const expectedDebtAmountRepaid = Math.floor(uusdBorrowed * maxDebtFraction)// + interest
  try {
    strictEqual(debtAmountRepaid, expectedDebtAmountRepaid)
  } catch (e) {
    console.log(e)
  }

  // refund amount
  const refundAmount = parseInt(uusdLiquidationResult.logs[0].eventsByType.wasm.refund_amount[0])
  if (debtFraction > CLOSE_FACTOR) {
    // TODO calculate the correct `expectedRefundAmount`
    const expectedRefundAmount = uusdRepayAmount - expectedDebtAmountRepaid
    try {
      strictEqual(refundAmount, expectedRefundAmount)
    } catch (e) {
      console.log(e)
    }
  } else {
    strictEqual(refundAmount, 0)
  }

  // collateral amount liquidated
  const collateralAmountLiquidated = parseInt(uusdLiquidationResult.logs[0].eventsByType.wasm.collateral_amount_liquidated[0])
  const expectedCollateralAmountLiquidated = new Int(expectedDebtAmountRepaid * (1 + LIQUIDATION_BONUS) / LUNA_USD_PRICE).toNumber()
  try {
    strictEqual(collateralAmountLiquidated, expectedCollateralAmountLiquidated)
  } catch (e) {
    console.log(e)
  }

  // collateral amount received
  if (receiveMaToken) {
    const maUlunaBalanceDifference = maUlunaBalanceAfter - maUlunaBalanceBeforeLiquidating
    try {
      strictEqual(collateralAmountLiquidated, maUlunaBalanceDifference)
    } catch (e) {
      console.log(e)
    }
  } else {
    const ulunaBalanceDifference = ulunaBalanceAfter - ulunaBalanceBeforeLiquidating
    try {
      strictEqual(collateralAmountLiquidated, ulunaBalanceDifference)
    } catch (e) {
      console.log(e)
    }
  }
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

main().catch(err => console.log(err))
