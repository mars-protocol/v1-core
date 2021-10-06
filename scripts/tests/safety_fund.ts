import {
  LocalTerra,
  MsgSend,
  Wallet,
  LCDClient
} from "@terra-money/terra.js"
import {
  strict as assert,
  strictEqual
} from "assert"
import 'dotenv/config.js'
import { join } from "path"
import {
  deployContract,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { queryBalanceNative } from "./test_helpers.js"

// CONSTS

// required environment variables
const ASTROPORT_ARTIFACTS_PATH = process.env.ASTROPORT_ARTIFACTS_PATH!

const TOKEN_SUPPLY = 1_000_000_000_000000
const TOKEN_LP = 10_000_000_000000
const USD_LP = 1_000_000_000000
const SAFETY_FUND_TOKEN_BALANCE = 100_000_000000

// TYPES

interface AstroportNativeToken { native_token: { denom: string } }

interface AstroportToken { token: { contract_addr: string } }

type AstroportAsset = AstroportNativeToken | AstroportToken

interface Env {
  terra: LocalTerra,
  deployer: Wallet,
  tokenCodeID: number,
  pairCodeID: number,
  astroportFactory: string,
  safetyFund: string,
}

// HELPERS

async function instantiateUsdPair(
  terra: LCDClient,
  wallet: Wallet,
  astroportFactory: string,
  bid: AstroportAsset,
) {
  const result = await executeContract(terra, wallet, astroportFactory,
    {
      create_pair: {
        pair_type: { xyk: {} },
        asset_infos: [
          bid,
          { native_token: { denom: "uusd" } }
        ]
      }
    }
  )
  return result.logs[0].eventsByType.wasm.pair_contract_addr[0]
}

async function provideLiquidity(
  terra: LCDClient,
  wallet: Wallet,
  address: string,
  token: AstroportAsset,
  coins: string,
) {
  await executeContract(terra, wallet, address,
    {
      provide_liquidity: {
        assets: [
          {
            info: token,
            amount: String(TOKEN_LP)
          }, {
            info: { native_token: { denom: "uusd" } },
            amount: String(USD_LP)
          }
        ]
      }
    },
    coins,
  )
}

// TESTS

async function testSwapNativeTokenToUsd(env: Env, denom: string) {
  const { terra, safetyFund, deployer, astroportFactory } = env

  const NATIVE_TOKEN = { native_token: { denom } }

  // instantiate a native token/USD Astroport pair
  const pairAddress = await instantiateUsdPair(terra, deployer, astroportFactory, NATIVE_TOKEN)

  await provideLiquidity(terra, deployer, pairAddress, NATIVE_TOKEN, `${USD_LP}uusd,${TOKEN_LP}${denom}`)

  // transfer some native token to the safety fund
  await performTransaction(terra, deployer,
    new MsgSend(
      deployer.key.accAddress,
      safetyFund,
      {
        [denom]: SAFETY_FUND_TOKEN_BALANCE
      }
    )
  )

  // cache the USD balance before swapping
  const prevUsdBalance = await queryBalanceNative(terra, safetyFund, "uusd")

  // swap the native token balance in the safety fund to USD
  await executeContract(terra, deployer, safetyFund,
    {
      "swap_asset_to_uusd": {
        "offer_asset_info": NATIVE_TOKEN,
        "amount": String(SAFETY_FUND_TOKEN_BALANCE)
      }
    }
  )

  // check the safety fund balances
  const usdBalance = await queryBalanceNative(terra, safetyFund, "uusd")
  assert(usdBalance > prevUsdBalance)
  const tokenBalance = await queryBalanceNative(terra, safetyFund, denom)
  strictEqual(tokenBalance, 0)

  // check the Astroport pair balances
  const pool = await queryContract(terra, pairAddress, { pool: {} })
  strictEqual(parseInt(pool.assets[0].amount), TOKEN_LP + SAFETY_FUND_TOKEN_BALANCE)
  assert(parseInt(pool.assets[1].amount) < USD_LP)
}

async function testSwapTokenToUsd(env: Env, address: string) {
  const { terra, safetyFund, deployer, astroportFactory } = env

  const TOKEN = { token: { contract_addr: address } }

  // instantiate a token/USD Astroport pair
  const pairAddress = await instantiateUsdPair(terra, deployer, astroportFactory, TOKEN)
  // approve the pair contract to transfer the token
  await executeContract(terra, deployer, address,
    {
      increase_allowance: {
        spender: pairAddress,
        amount: String(TOKEN_LP),
      }
    }
  )
  await provideLiquidity(terra, deployer, pairAddress, TOKEN, `${USD_LP}uusd`)

  // transfer some tokens to the safety fund
  await executeContract(terra, deployer, address,
    {
      transfer: {
        amount: String(SAFETY_FUND_TOKEN_BALANCE),
        recipient: safetyFund
      }
    }
  )

  // cache the USD balance before swapping
  const prevUsdBalance = await queryBalanceNative(terra, safetyFund, "uusd")

  // swap the token balance in the safety fund to USD
  await executeContract(terra, deployer, safetyFund,
    {
      swap_asset_to_uusd: {
        offer_asset_info: TOKEN,
        amount: String(SAFETY_FUND_TOKEN_BALANCE)
      }
    }
  )

  // check the safety fund balances
  const usdBalance = await queryBalanceNative(terra, safetyFund, "uusd")
  assert(usdBalance > prevUsdBalance)
  const tokenBalance = await queryContract(terra, address, { balance: { address: safetyFund } })
  strictEqual(parseInt(tokenBalance.balance), 0)

  // check the Astroport pair balances
  const pool = await queryContract(terra, pairAddress, { pool: {} })
  strictEqual(parseInt(pool.assets[0].amount), TOKEN_LP + SAFETY_FUND_TOKEN_BALANCE)
  assert(parseInt(pool.assets[1].amount) < USD_LP)
}

// MAIN

(async () => {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

  console.log("upload contracts")

  const tokenCodeID = await uploadContract(terra, deployer, join(ASTROPORT_ARTIFACTS_PATH, "astroport_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(ASTROPORT_ARTIFACTS_PATH, "astroport_pair.wasm"))
  const astroportFactory = await deployContract(terra, deployer, join(ASTROPORT_ARTIFACTS_PATH, "astroport_factory.wasm"),
    {
      token_code_id: tokenCodeID,
      pair_configs: [
        {
          code_id: pairCodeID,
          pair_type: { xyk: {} },
          total_fee_bps: 0,
          maker_fee_bps: 0
        }
      ]
    }
  )

  const safetyFund = await deployContract(terra, deployer, "../artifacts/mars_safety_fund.wasm",
    {
      owner: deployer.key.accAddress,
      astroport_factory_address: astroportFactory,
      astroport_max_spread: "0.01",
    }
  )

  const tokenAddress = await instantiateContract(terra, deployer, tokenCodeID,
    {
      name: "Mars",
      symbol: "MARS",
      decimals: 6,
      initial_balances: [
        {
          address: deployer.key.accAddress,
          amount: String(TOKEN_SUPPLY)
        }
      ]
    }
  )

  const env = {
    terra,
    deployer,
    tokenCodeID,
    pairCodeID,
    astroportFactory,
    safetyFund,
  }

  console.log("testSwapNativeTokenToUsd")
  await testSwapNativeTokenToUsd(env, "uluna")

  console.log("testSwapTokenToUsd")
  await testSwapTokenToUsd(env, tokenAddress)

  console.log("OK")
})()
