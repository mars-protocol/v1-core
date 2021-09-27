/*
Integration test for the safety fund contract swapping assets to UST via Astroport.

Required directory structure:
```
$ tree -L 1 $(git rev-parse --show-toplevel)/..
.
├── LocalTerra
├── protocol
├── terraswap
```
*/
import { Int, LocalTerra, MsgSend, Numeric, Wallet, LCDClient } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { strict as assert, strictEqual } from "assert"
import { join } from "path"
import { queryBalanceNative } from "./test_helpers.js"

// CONSTS

const ZERO = new Int(0)
const MARS_ARTIFACTS_PATH = "../artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"
const TOKEN_SUPPLY = 1_000_000_000_000000
const TOKEN_LP = 10_000_000_000000
const USD_LP = 1_000_000_000000
const SAFETY_FUND_TOKEN_BALANCE = 100_000_000000

// TYPES

interface TerraSwapNativeToken { native_token: { denom: string } }

interface TerraSwapToken { token: { contract_addr: string } }

type TerraSwapAsset = TerraSwapNativeToken | TerraSwapToken

interface Env {
  terra: LocalTerra,
  deployer: Wallet,
  tokenCodeID: number,
  pairCodeID: number,
  factoryCodeID: number,
  terraswapFactory: string,
  safetyFund: string,
}

// HELPERS

async function instantiateUsdPair(
  terra: LCDClient,
  wallet: Wallet,
  terraswapFactory: string,
  bid: TerraSwapAsset,
) {
  const result = await executeContract(terra, wallet, terraswapFactory,
    {
      create_pair: {
        asset_infos: [
          bid,
          { "native_token": { "denom": "uusd" } }
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
  token: TerraSwapAsset,
  coins: string,
) {
  await executeContract(terra, wallet, address,
    {
      "provide_liquidity": {
        "assets": [
          {
            "info": token,
            "amount": String(TOKEN_LP)
          }, {
            "info": { "native_token": { "denom": "uusd" } },
            "amount": String(USD_LP)
          }
        ]
      }
    },
    coins,
  )
}

// TESTS

async function testSwapNativeTokenToUsd(env: Env, denom: string) {
  const { terra, safetyFund, deployer, terraswapFactory } = env

  const NATIVE_TOKEN = { "native_token": { "denom": denom } }

  // instantiate a native token/USD Astroport pair
  const pairAddress = await instantiateUsdPair(terra, deployer, terraswapFactory, NATIVE_TOKEN)

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
  const { terra, safetyFund, deployer, terraswapFactory } = env

  const TOKEN = { "token": { "contract_addr": address } }

  // instantiate a token/USD Astroport pair
  const pairAddress = await instantiateUsdPair(terra, deployer, terraswapFactory, TOKEN)
  // approve the pair contract to transfer the token
  await executeContract(terra, deployer, address,
    {
      "increase_allowance": {
        "spender": pairAddress,
        "amount": String(TOKEN_LP),
      }
    }
  )
  await provideLiquidity(terra, deployer, pairAddress, TOKEN, `${USD_LP}uusd`)

  // transfer some tokens to the safety fund
  await executeContract(terra, deployer, address,
    {
      "transfer": {
        "amount": String(SAFETY_FUND_TOKEN_BALANCE),
        "recipient": safetyFund
      }
    }
  )

  // cache the USD balance before swapping
  const prevUsdBalance = await queryBalanceNative(terra, safetyFund, "uusd")

  // swap the token balance in the safety fund to USD
  await executeContract(terra, deployer, safetyFund,
    {
      "swap_asset_to_uusd": {
        "offer_asset_info": TOKEN,
        "amount": String(SAFETY_FUND_TOKEN_BALANCE)
      }
    }
  )

  // check the safety fund balances
  const usdBalance = await queryBalanceNative(terra, safetyFund, "uusd")
  assert(usdBalance > prevUsdBalance)
  const tokenBalance = await queryContract(terra, address, { "balance": { "address": safetyFund } })
  strictEqual(parseInt(tokenBalance.balance), 0)

  // check the Astroport pair balances
  const pool = await queryContract(terra, pairAddress, { "pool": {} })
  strictEqual(parseInt(pool.assets[0].amount), TOKEN_LP + SAFETY_FUND_TOKEN_BALANCE)
  assert(parseInt(pool.assets[1].amount) < USD_LP)
}

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

  console.log("deploying Astroport contracts")
  const tokenCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const factoryCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"))
  // instantiate the factory contract without `init_hook`, so that it can be a directory of pairs
  const terraswapFactory = await instantiateContract(terra, deployer, factoryCodeID,
    {
      "pair_code_id": pairCodeID,
      "token_code_id": tokenCodeID
    }
  )

  console.log("deploying Mars safety fund")
  const safetyFund = await deployContract(terra, deployer, join(MARS_ARTIFACTS_PATH, "safety_fund.wasm"),
    {
      "owner": deployer.key.accAddress,
      "astroport_factory_address": terraswapFactory,
      "astroport_max_spread": "0.01",
    }
  )

  console.log("deploying a token contract")
  const tokenAddress = await instantiateContract(terra, deployer, tokenCodeID,
    {
      "name": "Mars",
      "symbol": "MARS",
      "decimals": 6,
      "initial_balances": [
        {
          "address": deployer.key.accAddress,
          "amount": String(TOKEN_SUPPLY)
        }
      ]
    }
  )

  const env = {
    terra,
    deployer,
    tokenCodeID,
    pairCodeID,
    factoryCodeID,
    terraswapFactory,
    safetyFund,
  }

  console.log("testSwapNativeTokenToUsd")
  await testSwapNativeTokenToUsd(env, "uluna")

  console.log("testSwapTokenToUsd")
  await testSwapTokenToUsd(env, tokenAddress)

  console.log("OK")
}

main().catch(err => console.log(err));
