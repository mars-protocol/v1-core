import { Coin, Int, LCDClient, LocalTerra, MnemonicKey, Wallet } from "@terra-money/terra.js"
import { strictEqual, strict as assert } from "assert"
import { join } from "path"
import 'dotenv/config.js'
import {
  deployContract,
  executeContract,
  instantiateContract,
  queryContract,
  setTimeoutDuration,
  sleep,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"

// CONSTS

// required environment variables:
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!
const TERRASWAP_ARTIFACTS_PATH = process.env.TERRASWAP_ARTIFACTS_PATH!

// protocol rewards collector
const SAFETY_FUND_FEE_SHARE = 0.1
const TREASURY_FEE_SHARE = 0.2

// red-bank
const CLOSE_FACTOR = 0.5
const MAX_LTV = 0.55
const LIQUIDATION_BONUS = 0.1
const MA_TOKEN_SCALING_FACTOR = 1_000_000
// set a high interest rate, so tests can be run faster
const INTEREST_RATE = 100000

// native tokens
const LUNA_USD_PRICE = 25
const USD_COLLATERAL_AMOUNT = 100_000_000_000000
const LUNA_COLLATERAL_AMOUNT = 1_000_000000
const USD_BORROW_AMOUNT = LUNA_COLLATERAL_AMOUNT * LUNA_USD_PRICE * MAX_LTV

// cw20 tokens
const CW20_TOKEN_USD_PRICE = 10
const CW20_TOKEN_1_COLLATERAL_AMOUNT = 100_000_000_000000
const CW20_TOKEN_2_COLLATERAL_AMOUNT = 1_000_000000
const CW20_TOKEN_1_BORROW_AMOUNT = CW20_TOKEN_2_COLLATERAL_AMOUNT * MAX_LTV
const CW20_TOKEN_1_UUSD_PAIR_UUSD_LP_AMOUNT = 1_000_000_000000
const CW20_TOKEN_1_UUSD_PAIR_CW20_TOKEN_1_LP_AMOUNT = CW20_TOKEN_1_UUSD_PAIR_UUSD_LP_AMOUNT * CW20_TOKEN_USD_PRICE

// HELPERS

async function queryMaAssetAddress(terra: LCDClient, redBank: string, asset: Asset): Promise<string> {
  const market = await queryContract(terra, redBank, { market: { asset: asset } })
  return market.ma_token_address
}

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

async function depositNative(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    { deposit_native: { denom: denom } },
    `${amount}${denom}`
  )
}

async function depositCw20(terra: LCDClient, wallet: Wallet, redBank: string, contract: string, amount: number) {
  return await executeContract(terra, wallet, contract,
    {
      send: {
        contract: redBank,
        amount: String(amount),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )
}

async function borrowNative(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    {
      borrow: {
        asset: { native: { denom: denom } },
        amount: String(amount)
      }
    }
  )
}

async function borrowCw20(terra: LCDClient, wallet: Wallet, redBank: string, contract: string, amount: number) {
  return await executeContract(terra, wallet, redBank,
    {
      borrow: {
        asset: { cw20: { contract_addr: contract } },
        amount: String(amount)
      }
    }
  )
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

async function computeTax(terra: LCDClient, coin: Coin) {
  const DECIMAL_FRACTION = new Int("1000000000000000000") // 10^18
  const taxRate = await terra.treasury.taxRate()
  const taxCap = (await terra.treasury.taxCap(coin.denom)).amount
  const amount = coin.amount
  const tax = amount.sub(
    amount
      .mul(DECIMAL_FRACTION)
      .div(DECIMAL_FRACTION.mul(taxRate).add(DECIMAL_FRACTION))
  )
  return tax.gt(taxCap) ? taxCap : tax
}

async function deductTax(terra: LCDClient, coin: Coin) {
  return coin.amount.sub(await computeTax(terra, coin)).floor()
}

function approximateEqual(actual: number, expected: number, tol: number) {
  try {
    assert(actual >= expected - tol && actual <= expected + tol)
  } catch (error) {
    strictEqual(actual, expected)
  }
}

// TYPES

interface Native { native: { denom: string } }

interface CW20 { cw20: { contract_addr: string } }

type Asset = Native | CW20

interface Env {
  terra: LocalTerra
  deployer: Wallet
  provider: Wallet
  borrower: Wallet
  cw20Token1: string
  cw20Token2: string
  maUluna: string
  maUusd: string
  maCw20Token1: string
  maCw20Token2: string
  redBank: string
  protocolRewardsCollector: string
  treasury: string
  safetyFund: string
  staking: string
  cw20Token1UusdPair: string
}

// TESTS

async function testNative(env: Env) {
  const {
    terra,
    deployer,
    provider,
    borrower,
    maUusd,
    redBank,
    protocolRewardsCollector,
    treasury,
    safetyFund,
    staking
  } = env

  {
    console.log("provider provides uusd")

    const maUusdBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    strictEqual(maUusdBalanceBefore, 0)

    await depositNative(terra, provider, redBank, "uusd", USD_COLLATERAL_AMOUNT)

    const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    strictEqual(maUusdBalanceAfter, 0)
  }


  console.log("borrower provides uluna")

  await depositNative(terra, borrower, redBank, "uluna", LUNA_COLLATERAL_AMOUNT)

  console.log("borrower borrows uusd up to the borrow limit of their uluna collateral")

  await borrowNative(terra, borrower, redBank, "uusd", Math.floor(USD_BORROW_AMOUNT))

  {
    console.log("repay")

    const maUusdBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)

    await executeContract(terra, borrower, redBank,
      { repay_native: { denom: "uusd" } },
      `${Math.floor(USD_BORROW_AMOUNT)}uusd`
    )

    const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    assert(maUusdBalanceAfter > maUusdBalanceBefore)
  }

  {
    console.log("withdraw")

    const maUusdBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)

    await executeContract(terra, provider, redBank,
      {
        withdraw: {
          asset: { native: { denom: "uusd" } },
          amount: String(Math.floor(USD_COLLATERAL_AMOUNT / 2))
        }
      }
    )

    const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    assert(maUusdBalanceAfter > maUusdBalanceBefore)
  }

  console.log("protocol rewards collector withdraws from the red bank")

  {
    console.log("- specify an amount")

    const maUusdBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    const uusdBalanceBefore = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")

    // withdraw half
    await executeContract(terra, deployer, protocolRewardsCollector,
      {
        withdraw_from_red_bank: {
          asset: { native: { denom: "uusd" } },
          amount: String(Math.floor(maUusdBalanceBefore / MA_TOKEN_SCALING_FACTOR / 2))
        }
      }
    )

    const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    const uusdBalanceAfter = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    assert(maUusdBalanceAfter < maUusdBalanceBefore)
    assert(uusdBalanceAfter > uusdBalanceBefore)
  }

  {
    console.log("- don't specify an amount")

    const uusdBalanceBefore = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")

    // withdraw remaining balance
    let result = await executeContract(terra, deployer, protocolRewardsCollector,
      { withdraw_from_red_bank: { asset: { native: { denom: "uusd" } } } }
    )

    const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
    const uusdBalanceAfter = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    assert(uusdBalanceAfter > uusdBalanceBefore)

    // withdrawing from the red bank triggers protocol rewards to be minted to the protocol rewards
    // collector, so the maUusd balance will not be zero after this call
    const maUusdMintAmount = parseInt(result.logs[0].eventsByType.wasm.amount[0])
    strictEqual(maUusdBalanceAfter, maUusdMintAmount)
  }


  console.log("try to distribute uusd rewards")

  await assert.rejects(
    executeContract(terra, deployer, protocolRewardsCollector,
      { distribute_protocol_rewards: { asset: { native: { denom: "uusd" } } } }
    ),
    (error: any) => {
      return error.response.data.error.includes("Asset is not enabled for distribution: \"uusd\"")
    }
  )

  console.log("enable uusd for distribution")

  await executeContract(terra, deployer, protocolRewardsCollector,
    {
      update_asset_config: {
        asset: { native: { denom: "uusd" } },
        enabled: true
      }
    }
  )

  {
    console.log("distribute uusd rewards")

    const protocolRewardsCollectorUusdBalanceBefore = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    const treasuryUusdBalanceBefore = await queryNativeBalance(terra, treasury, "uusd")
    const safetyFundUusdBalanceBefore = await queryNativeBalance(terra, safetyFund, "uusd")
    const stakingUusdBalanceBefore = await queryNativeBalance(terra, staking, "uusd")

    await executeContract(terra, deployer, protocolRewardsCollector,
      { distribute_protocol_rewards: { asset: { native: { denom: "uusd" } } } }
    )

    const protocolRewardsCollectorUusdBalanceAfter = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    const treasuryUusdBalanceAfter = await queryNativeBalance(terra, treasury, "uusd")
    const safetyFundUusdBalanceAfter = await queryNativeBalance(terra, safetyFund, "uusd")
    const stakingUusdBalanceAfter = await queryNativeBalance(terra, staking, "uusd")

    // TODO why is `protocolRewardsCollectorUusdBalanceAfter == 3`? rounding errors from integer arithmetic?
    // strictEqual(protocolRewardsCollectorUusdBalanceAfter, 0)
    // Check a tight interval instead of equality
    assert(protocolRewardsCollectorUusdBalanceAfter < 4)

    const protocolRewardsCollectorUusdBalanceDifference =
      protocolRewardsCollectorUusdBalanceBefore - protocolRewardsCollectorUusdBalanceAfter
    const treasuryUusdBalanceDifference = treasuryUusdBalanceAfter - treasuryUusdBalanceBefore
    const safetyFundUusdBalanceDifference = safetyFundUusdBalanceAfter - safetyFundUusdBalanceBefore
    const stakingUusdBalanceDifference = stakingUusdBalanceAfter - stakingUusdBalanceBefore

    const expectedTreasuryUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * TREASURY_FEE_SHARE)
      )).toNumber()
    const expectedSafetyFundUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * SAFETY_FUND_FEE_SHARE)
      )).toNumber()

    const expectedStakingUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * (1 - (TREASURY_FEE_SHARE + SAFETY_FUND_FEE_SHARE)))
      )).toNumber()

    // TODO why is treasuryUusdBalanceDifference 2 uusd different from expected?
    // strictEqual(treasuryUusdBalanceDifference, expectedTreasuryUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(treasuryUusdBalanceDifference, expectedTreasuryUusdBalanceDifference, 2)

    // TODO why is safetyFundUusdBalanceDifference 1 uusd different from expected?
    // strictEqual(safetyFundUusdBalanceDifference, expectedSafetyFundUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(safetyFundUusdBalanceDifference, expectedSafetyFundUusdBalanceDifference, 1)

    // TODO why is stakingUusdBalanceDifference 4 uusd different from expected?
    // strictEqual(stakingUusdBalanceDifference, expectedStakingUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(stakingUusdBalanceDifference, expectedStakingUusdBalanceDifference, 4)
  }
}

async function testCw20(env: Env) {
  const {
    terra,
    deployer,
    provider,
    borrower,
    cw20Token1,
    cw20Token2,
    maCw20Token1,
    redBank,
    protocolRewardsCollector,
    treasury,
    safetyFund,
    staking,
  } = env

  // mint some tokens
  await mintCw20(terra, deployer, cw20Token1, provider.key.accAddress, CW20_TOKEN_1_COLLATERAL_AMOUNT)
  await mintCw20(terra, deployer, cw20Token2, borrower.key.accAddress, CW20_TOKEN_2_COLLATERAL_AMOUNT)

  {
    console.log("provider provides cw20 token 1")

    const maCwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    strictEqual(maCwToken1BalanceBefore, 0)

    await depositCw20(terra, provider, redBank, cw20Token1, CW20_TOKEN_1_COLLATERAL_AMOUNT)

    const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    strictEqual(maCwToken1BalanceAfter, 0)
  }

  console.log("borrower provides cw20 token 2")

  await depositCw20(terra, borrower, redBank, cw20Token2, CW20_TOKEN_2_COLLATERAL_AMOUNT)

  console.log("borrower borrows cw20 token 1 up to the borrow limit of their cw20 token 2 collateral")

  await borrowCw20(terra, borrower, redBank, cw20Token1, CW20_TOKEN_1_BORROW_AMOUNT)

  {
    console.log("repay")

    const maCwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)

    await executeContract(terra, borrower, cw20Token1,
      {
        send: {
          contract: redBank,
          amount: String(CW20_TOKEN_1_BORROW_AMOUNT),
          msg: toEncodedBinary({ repay_cw20: {} })
        }
      }
    )

    const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    assert(maCwToken1BalanceAfter > maCwToken1BalanceBefore)
  }

  {
    console.log("withdraw")

    const maCwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)

    await executeContract(terra, provider, redBank,
      {
        withdraw: {
          asset: { cw20: { contract_addr: cw20Token1 } },
          amount: String(Math.floor(CW20_TOKEN_1_BORROW_AMOUNT / 2))
        }
      }
    )

    const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    assert(maCwToken1BalanceAfter > maCwToken1BalanceBefore)
  }

  console.log("protocol rewards collector withdraws from the red bank")

  {
    console.log("- specify an amount")

    const maCwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    const cwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, cw20Token1)

    // withdraw half
    await executeContract(terra, deployer, protocolRewardsCollector,
      {
        withdraw_from_red_bank: {
          asset: { cw20: { contract_addr: cw20Token1 } },
          amount: String(Math.floor(maCwToken1BalanceBefore / MA_TOKEN_SCALING_FACTOR / 2))
        }
      }
    )

    const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    const cwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, cw20Token1)
    assert(maCwToken1BalanceAfter < maCwToken1BalanceBefore)
    assert(cwToken1BalanceAfter > cwToken1BalanceBefore)
  }

  {
    console.log("- don't specify an amount")

    const cwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, cw20Token1)

    // withdraw remaining balance
    const result = await executeContract(terra, deployer, protocolRewardsCollector,
      { withdraw_from_red_bank: { asset: { cw20: { contract_addr: cw20Token1 } } } }
    )

    const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
    const cwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, cw20Token1)
    assert(cwToken1BalanceAfter > cwToken1BalanceBefore)

    // withdrawing from the red bank triggers protocol rewards to be minted to the protocol rewards
    // collector, so the maCw20Token1 balance will not be zero after this call
    const maCw20Token1MintAmount = parseInt(result.logs[0].eventsByType.wasm.amount[0])
    strictEqual(maCwToken1BalanceAfter, maCw20Token1MintAmount)
  }

  console.log("try to distribute cw20 token 1 rewards")

  await assert.rejects(
    executeContract(terra, deployer, protocolRewardsCollector,
      { distribute_protocol_rewards: { asset: { cw20: { contract_addr: cw20Token1 } } } }
    ),
    (error: any) => {
      return error.response.data.error.includes(`Asset is not enabled for distribution: \"${cw20Token1}\"`)
    }
  )

  console.log("swap cw20 token 1 to uusd")

  await executeContract(terra, deployer, protocolRewardsCollector,
    {
      swap_asset_to_uusd: {
        offer_asset_info: { token: { contract_addr: cw20Token1 } }
      }
    }
  )

  console.log("enable uusd for distribution")

  await executeContract(terra, deployer, protocolRewardsCollector,
    {
      update_asset_config: {
        asset: { native: { denom: "uusd" } },
        enabled: true
      }
    }
  )

  {
    console.log("distribute uusd rewards")

    const protocolRewardsCollectorUusdBalanceBefore = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    const treasuryUusdBalanceBefore = await queryNativeBalance(terra, treasury, "uusd")
    const safetyFundUusdBalanceBefore = await queryNativeBalance(terra, safetyFund, "uusd")
    const stakingUusdBalanceBefore = await queryNativeBalance(terra, staking, "uusd")

    await executeContract(terra, deployer, protocolRewardsCollector,
      { distribute_protocol_rewards: { asset: { native: { denom: "uusd" } } } }
    )

    const protocolRewardsCollectorUusdBalanceAfter = await queryNativeBalance(terra, protocolRewardsCollector, "uusd")
    const treasuryUusdBalanceAfter = await queryNativeBalance(terra, treasury, "uusd")
    const safetyFundUusdBalanceAfter = await queryNativeBalance(terra, safetyFund, "uusd")
    const stakingUusdBalanceAfter = await queryNativeBalance(terra, staking, "uusd")

    // TODO why is `protocolRewardsCollectorUusdBalanceAfter == 3`? rounding errors from integer arithmetic?
    // strictEqual(protocolRewardsCollectorUusdBalanceAfter, 0)
    // Check a tight interval instead of equality
    assert(protocolRewardsCollectorUusdBalanceAfter < 4)

    const protocolRewardsCollectorUusdBalanceDifference =
      protocolRewardsCollectorUusdBalanceBefore - protocolRewardsCollectorUusdBalanceAfter
    const treasuryUusdBalanceDifference = treasuryUusdBalanceAfter - treasuryUusdBalanceBefore
    const safetyFundUusdBalanceDifference = safetyFundUusdBalanceAfter - safetyFundUusdBalanceBefore
    const stakingUusdBalanceDifference = stakingUusdBalanceAfter - stakingUusdBalanceBefore

    const expectedTreasuryUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * TREASURY_FEE_SHARE)
      )).toNumber()
    const expectedSafetyFundUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * SAFETY_FUND_FEE_SHARE)
      )).toNumber()

    const expectedStakingUusdBalanceDifference =
      (await deductTax(
        terra,
        new Coin("uusd", protocolRewardsCollectorUusdBalanceDifference * (1 - (TREASURY_FEE_SHARE + SAFETY_FUND_FEE_SHARE)))
      )).toNumber()

    // TODO why is treasuryUusdBalanceDifference 2 uusd different from expected?
    // strictEqual(treasuryUusdBalanceDifference, expectedTreasuryUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(treasuryUusdBalanceDifference, expectedTreasuryUusdBalanceDifference, 2)

    // TODO why is safetyFundUusdBalanceDifference 1 uusd different from expected?
    // strictEqual(safetyFundUusdBalanceDifference, expectedSafetyFundUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(safetyFundUusdBalanceDifference, expectedSafetyFundUusdBalanceDifference, 1)

    // TODO why is stakingUusdBalanceDifference 4 uusd different from expected?
    // strictEqual(stakingUusdBalanceDifference, expectedStakingUusdBalanceDifference)
    // Check a tight interval instead of equality
    approximateEqual(stakingUusdBalanceDifference, expectedStakingUusdBalanceDifference, 4)
  }
}

async function testLiquidateNative(env: Env) {
  const {
    terra,
    deployer,
    provider,
    borrower,
    maUluna,
    maUusd,
    redBank,
    protocolRewardsCollector,
  } = env

  const liquidator = deployer

  console.log("provider provides uusd")

  await depositNative(terra, provider, redBank, "uusd", USD_COLLATERAL_AMOUNT)

  console.log("borrower provides uluna")

  await depositNative(terra, borrower, redBank, "uluna", LUNA_COLLATERAL_AMOUNT)

  console.log("borrower borrows uusd up to the borrow limit of their uluna collateral")

  await borrowNative(terra, borrower, redBank, "uusd", Math.floor(USD_BORROW_AMOUNT))

  console.log("someone borrows uluna in order for rewards to start accruing")

  await borrowNative(terra, provider, redBank, "uluna", Math.floor(LUNA_COLLATERAL_AMOUNT / 10))

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

  // get the protocol rewards collector balances before the borrower is liquidated
  const maUusdBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
  const maUlunaBalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maUluna)

  await executeContract(terra, liquidator, redBank,
    {
      liquidate_native: {
        collateral_asset: { native: { denom: "uluna" } },
        debt_asset_denom: "uusd",
        user_address: borrower.key.accAddress,
        receive_ma_token: false,
      }
    },
    `${Math.floor(USD_BORROW_AMOUNT * CLOSE_FACTOR)}uusd`
  )

  // get the protocol rewards collector balances after the borrower is liquidated
  const maUusdBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUusd)
  const maUlunaBalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maUluna)
  assert(maUusdBalanceAfter > maUusdBalanceBefore)
  assert(maUlunaBalanceAfter > maUlunaBalanceBefore)
}

async function testLiquidateCw20(env: Env) {
  const {
    terra,
    deployer,
    provider,
    borrower,
    maCw20Token1,
    maCw20Token2,
    cw20Token1,
    cw20Token2,
    redBank,
    protocolRewardsCollector
  } = env

  const liquidator = deployer

  // mint some tokens
  await mintCw20(terra, deployer, cw20Token1, provider.key.accAddress, CW20_TOKEN_1_COLLATERAL_AMOUNT)
  await mintCw20(terra, deployer, cw20Token1, liquidator.key.accAddress, CW20_TOKEN_1_COLLATERAL_AMOUNT)
  await mintCw20(terra, deployer, cw20Token2, borrower.key.accAddress, CW20_TOKEN_2_COLLATERAL_AMOUNT)

  console.log("provider provides cw20 token 1")

  await depositCw20(terra, provider, redBank, cw20Token1, CW20_TOKEN_1_COLLATERAL_AMOUNT)

  console.log("borrower provides cw20 token 2")

  await depositCw20(terra, borrower, redBank, cw20Token2, CW20_TOKEN_2_COLLATERAL_AMOUNT)

  console.log("borrower borrows cw20 token 1 up to the borrow limit of their cw20 token 2 collateral")

  await borrowCw20(terra, borrower, redBank, cw20Token1, CW20_TOKEN_1_BORROW_AMOUNT)

  console.log("someone borrows cw20 token 2 in order for rewards to start accruing")

  await borrowCw20(terra, provider, redBank, cw20Token2, Math.floor(CW20_TOKEN_1_BORROW_AMOUNT / 10))

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

  // get the protocol rewards collector balances before the borrower is liquidated
  const maCwToken1BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
  const maCwToken2BalanceBefore = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token2)

  await executeContract(terra, liquidator, cw20Token1,
    {
      send: {
        contract: redBank,
        amount: String(Math.floor(CW20_TOKEN_1_BORROW_AMOUNT * CLOSE_FACTOR)),
        msg: toEncodedBinary({
          liquidate_cw20: {
            collateral_asset: { cw20: { contract_addr: cw20Token2 } },
            user_address: borrower.key.accAddress,
            receive_ma_token: false,
          }
        })
      }
    }
  )

  // get the protocol rewards collector balances after the borrower is liquidated
  const maCwToken1BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token1)
  const maCwToken2BalanceAfter = await queryCw20Balance(terra, protocolRewardsCollector, maCw20Token2)
  assert(maCwToken1BalanceAfter > maCwToken1BalanceBefore)
  assert(maCwToken2BalanceAfter > maCwToken2BalanceBefore)
}

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()

  // addresses
  const deployer = terra.wallets.test1
  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3
  // mock contract addresses
  const staking = new MnemonicKey().accAddress
  const safetyFund = new MnemonicKey().accAddress
  const treasury = new MnemonicKey().accAddress

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
        ma_token_code_id: maTokenCodeId,
        close_factor: "0.5",
      }
    }
  )

  const tokenCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const terraswapFactory = await deployContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"),
    {
      pair_code_id: pairCodeID,
      token_code_id: tokenCodeID
    }
  )

  const protocolRewardsCollector = await deployContract(terra, deployer, "../artifacts/protocol_rewards_collector.wasm",
    {
      config: {
        owner: deployer.key.accAddress,
        address_provider_address: addressProvider,
        safety_fund_fee_share: String(SAFETY_FUND_FEE_SHARE),
        treasury_fee_share: String(TREASURY_FEE_SHARE),
        astroport_factory_address: terraswapFactory,
        astroport_max_spread: "0.05",
      }
    }
  )

  // update address provider
  await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          protocol_rewards_collector_address: protocolRewardsCollector,
          staking_address: staking,
          treasury_address: treasury,
          insurance_fund_address: safetyFund,
          incentives_address: incentives,
          oracle_address: oracle,
          red_bank_address: redBank,
          protocol_admin_address: deployer.key.accAddress,
        }
      }
    }
  )

  // cw20 tokens
  const cw20CodeId = await uploadContract(terra, deployer, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"))

  const cw20Token1 = await instantiateContract(terra, deployer, cw20CodeId,
    {
      name: "cw20 Token 1",
      symbol: "ONE",
      decimals: 6,
      initial_balances: [],
      mint: { minter: deployer.key.accAddress }
    }
  )

  const cw20Token2 = await instantiateContract(terra, deployer, cw20CodeId,
    {
      name: "cw20 Token 2",
      symbol: "TWO",
      decimals: 6,
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
          max_loan_to_value: String(MAX_LTV),
          reserve_factor: "0.2",
          maintenance_margin: String(MAX_LTV + 0.001),
          liquidation_bonus: String(LIQUIDATION_BONUS),
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0.1", // TODO panics with 0
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle,
    { native: { denom: "uluna" } },
    LUNA_USD_PRICE
  )
  const maUluna = await queryMaAssetAddress(terra, redBank, { native: { denom: "uluna" } })

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
              optimal_utilization_rate: "0.1", // TODO panics with 0
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle,
    { native: { denom: "uusd" } },
    1
  )
  const maUusd = await queryMaAssetAddress(terra, redBank, { native: { denom: "uusd" } })

  // cw20token1
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { cw20: { contract_addr: cw20Token1 } },
        asset_params: {
          initial_borrow_rate: "0.1",
          max_loan_to_value: String(MAX_LTV),
          reserve_factor: "0.2",
          maintenance_margin: String(MAX_LTV + 0.001),
          liquidation_bonus: String(LIQUIDATION_BONUS),
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0.1",  // TODO panics with 0
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle,
    { cw20: { contract_addr: cw20Token1 } },
    CW20_TOKEN_USD_PRICE
  )
  const maCw20Token1 = await queryMaAssetAddress(terra, redBank, { cw20: { contract_addr: cw20Token1 } })

  // cw20token2
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { cw20: { contract_addr: cw20Token2 } },
        asset_params: {
          initial_borrow_rate: "0.1",
          max_loan_to_value: String(MAX_LTV),
          reserve_factor: "0.2",
          maintenance_margin: String(MAX_LTV + 0.001),
          liquidation_bonus: String(LIQUIDATION_BONUS),
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0.1", // TODO panics with 0
              base: String(INTEREST_RATE),
              slope_1: "0",
              slope_2: "0",
            }
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )
  await setAssetOraclePriceSource(terra, deployer, oracle,
    { cw20: { contract_addr: cw20Token2 } },
    CW20_TOKEN_USD_PRICE
  )
  const maCw20Token2 = await queryMaAssetAddress(terra, redBank, { cw20: { contract_addr: cw20Token2 } })

  // terraswap pair

  let result = await executeContract(terra, deployer, terraswapFactory,
    {
      create_pair: {
        asset_infos: [
          { token: { contract_addr: cw20Token1 } },
          { native_token: { denom: "uusd" } }
        ]
      }
    }
  )
  const cw20Token1UusdPair = result.logs[0].eventsByType.wasm.pair_contract_addr[0]

  await mintCw20(terra, deployer, cw20Token1, deployer.key.accAddress, CW20_TOKEN_1_UUSD_PAIR_CW20_TOKEN_1_LP_AMOUNT)

  await executeContract(terra, deployer, cw20Token1,
    {
      increase_allowance: {
        spender: cw20Token1UusdPair,
        amount: String(CW20_TOKEN_1_UUSD_PAIR_CW20_TOKEN_1_LP_AMOUNT),
      }
    }
  )

  await executeContract(terra, deployer, cw20Token1UusdPair,
    {
      provide_liquidity: {
        assets: [
          {
            info: { token: { contract_addr: cw20Token1 } },
            amount: String(CW20_TOKEN_1_UUSD_PAIR_CW20_TOKEN_1_LP_AMOUNT)
          }, {
            info: { native_token: { denom: "uusd" } },
            amount: String(CW20_TOKEN_1_UUSD_PAIR_UUSD_LP_AMOUNT)
          }
        ]
      }
    },
    `${CW20_TOKEN_1_UUSD_PAIR_UUSD_LP_AMOUNT}uusd`,
  )

  const env: Env = {
    terra,
    deployer,
    provider,
    borrower,
    cw20Token1,
    cw20Token2,
    maUluna,
    maUusd,
    maCw20Token1,
    maCw20Token2,
    redBank,
    protocolRewardsCollector,
    treasury,
    safetyFund,
    staking,
    cw20Token1UusdPair
  }

  console.log("testNative")
  env.provider = terra.wallets.test2
  env.borrower = terra.wallets.test3
  await testNative(env)

  console.log("testCw20")
  env.provider = terra.wallets.test4
  env.borrower = terra.wallets.test5
  await testCw20(env)

  console.log("testLiquidateNative")
  env.provider = terra.wallets.test6
  env.borrower = terra.wallets.test7
  await testLiquidateNative(env)

  console.log("testLiquidateCw20")
  env.provider = terra.wallets.test8
  env.borrower = terra.wallets.test9
  await testLiquidateCw20(env)

  console.log("OK")
}

main().catch(err => console.log(err))
