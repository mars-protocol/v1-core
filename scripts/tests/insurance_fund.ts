/*
Integration test for the insurance fund contract swapping assets to UST via Terraswap.

Required directory structure:
```
$ tree -L 1 $(git rev-parse --show-toplevel)/..
.
├── LocalTerra
├── protocol
├── terraswap
```

This test works on columbus-4 with the following versions:
- LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
- terracli v0.5.0-rc0
- terraswap 72c60c05c43841499f760710a03f864c5ee4db3b

TODO:
- Upgrade to columbus-5
*/

import { Int, LocalTerra, MsgSend, Numeric, Wallet } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"
import { strict as assert, strictEqual } from "assert"
import { join } from "path"

// types

interface NativeToken {
  native_token: {
    denom: string
  }
}

interface CW20 {
  token: {
    contract_addr: string
  }
}

type Token = NativeToken | CW20

interface Env {
  terra: LocalTerra,
  wallet: Wallet,
  tokenCodeID: number,
  pairCodeID: number,
  factoryCodeID: number,
  factoryAddress: string,
  insuranceFundAddress: string,
}

// consts and globals

const ZERO = new Int(0)
const MARS_ARTIFACTS_PATH = "../artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"
const TOKEN_SUPPLY = 1_000_000_000_000000
const TOKEN_LP = 10_000_000_000000
const USD_LP = 1_000_000_000000
const INSURANCE_FUND_TOKEN_BALANCE = 100_000_000000

// helpers

async function instantiateUsdPair(env: Env, bid: Token): Promise<string> {
  return await instantiateContract(env.terra, env.wallet, env.factoryCodeID,
    {
      "pair_code_id": env.pairCodeID,
      "token_code_id": env.tokenCodeID,
      "init_hook": {
        "msg": toEncodedBinary(
          {
            "create_pair": {
              "asset_infos": [
                bid, {
                  "native_token": {
                    "denom": "uusd"
                  }
                }
              ]
            }
          }
        ),
        "contract_addr": env.factoryAddress
      }
    }
  )
}

async function provideLiquidity(env: Env, address: string, token: Token, coins: string) {
  await executeContract(env.terra, env.wallet, address,
    {
      "provide_liquidity": {
        "assets": [
          {
            "info": token,
            "amount": String(TOKEN_LP)
          }, {
            "info": {
              "native_token": {
                "denom": "uusd"
              }
            },
            "amount": String(USD_LP)
          }
        ]
      }
    },
    coins,
  )
}

async function getBalance(env: Env, address: string, denom: string): Promise<Numeric.Output> {
  const balances = await env.terra.bank.balance(address)
  const balance = balances.get(denom)
  if (balance === undefined) {
    return ZERO
  }
  return balance.amount
}

// tests

async function testSwapNativeTokenToUsd(env: Env, denom: string) {
  const NATIVE_TOKEN = {
    "native_token": {
      "denom": denom
    }
  }

  // instantiate a native token/USD Terraswap pair
  const pairAddress = await instantiateUsdPair(env, NATIVE_TOKEN)
  await provideLiquidity(env, pairAddress, NATIVE_TOKEN, `${USD_LP}uusd,${TOKEN_LP}${denom}`)

  // transfer some native token to the insurance fund
  await performTransaction(env.terra, env.wallet,
    new MsgSend(
      env.wallet.key.accAddress,
      env.insuranceFundAddress,
      {
        [denom]: INSURANCE_FUND_TOKEN_BALANCE
      }
    )
  )

  // cache the USD balance before swapping
  const prevUsdBalance = await getBalance(env, env.insuranceFundAddress, "uusd")

  // swap the native token balance in the insurance fund to USD
  await executeContract(env.terra, env.wallet, env.insuranceFundAddress,
    {
      "swap_asset_to_uusd": {
        "offer_asset_info": NATIVE_TOKEN,
        "amount": String(INSURANCE_FUND_TOKEN_BALANCE)
      }
    }
  )

  // check the insurance fund balances
  const usdBalance = await getBalance(env, env.insuranceFundAddress, "uusd")
  assert(usdBalance.gt(prevUsdBalance))
  const tokenBalance = await getBalance(env, env.insuranceFundAddress, denom)
  strictEqual(tokenBalance, ZERO)

  // check the Terraswap pair balances
  const pool = await queryContract(env.terra, pairAddress,
    {
      "pool": {}
    }
  )
  strictEqual(pool.assets[0].amount, String(TOKEN_LP + INSURANCE_FUND_TOKEN_BALANCE))
  assert(parseInt(pool.assets[1].amount) < USD_LP)
}

async function testSwapTokenToUsd(env: Env, address: string) {
  const TOKEN = {
    "token": {
      "contract_addr": address
    }
  }

  // instantiate a token/USD Terraswap pair
  const pairAddress = await instantiateUsdPair(env, TOKEN)
  // approve the pair contract to transfer the token
  await executeContract(env.terra, env.wallet, address,
    {
      "increase_allowance": {
        "spender": pairAddress,
        "amount": String(TOKEN_LP),
      }
    }
  )
  await provideLiquidity(env, pairAddress, TOKEN, `${USD_LP}uusd`)

  // transfer some tokens to the insurance fund
  await executeContract(env.terra, env.wallet, address,
    {
      "transfer": {
        "amount": String(INSURANCE_FUND_TOKEN_BALANCE),
        "recipient": env.insuranceFundAddress
      }
    }
  )

  // cache the USD balance before swapping
  const prevUsdBalance = await getBalance(env, env.insuranceFundAddress, "uusd")

  // swap the token balance in the insurance fund to USD
  await executeContract(env.terra, env.wallet, env.insuranceFundAddress,
    {
      "swap_asset_to_uusd": {
        "offer_asset_info": TOKEN,
        "amount": String(INSURANCE_FUND_TOKEN_BALANCE)
      }
    }
  )

  // check the insurance fund balances
  const usdBalance = await getBalance(env, env.insuranceFundAddress, "uusd")
  assert(usdBalance.gt(prevUsdBalance))
  const tokenBalance = await queryContract(env.terra, address,
    {
      "balance": {
        "address": env.insuranceFundAddress
      }
    }
  )
  strictEqual(tokenBalance.balance, "0")

  // check the Terraswap pair balances
  const pool = await queryContract(env.terra, pairAddress,
    {
      "pool": {}
    }
  )
  strictEqual(pool.assets[0].amount, String(TOKEN_LP + INSURANCE_FUND_TOKEN_BALANCE))
  assert(parseInt(pool.assets[1].amount) < USD_LP)
}

// main

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const wallet = terra.wallets.test1

  console.log("deploying Terraswap contracts")
  const tokenCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const factoryCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"))
  // instantiate the factory contract without `init_hook`, so that it can be a directory of pairs
  const factoryAddress = await instantiateContract(terra, wallet, factoryCodeID,
    {
      "pair_code_id": pairCodeID,
      "token_code_id": tokenCodeID
    }
  )

  console.log("deploying Mars insurance fund")
  const insuranceFundAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, "insurance_fund.wasm"),
    {
      "owner": wallet.key.accAddress,
      "terraswap_factory_address": factoryAddress,
      "terraswap_max_spread": "0.01",
    }
  )

  console.log("deploying a token contract")
  const tokenAddress = await instantiateContract(terra, wallet, tokenCodeID,
    {
      "name": "Mars",
      "symbol": "MARS",
      "decimals": 6,
      "initial_balances": [
        {
          "address": wallet.key.accAddress,
          "amount": String(TOKEN_SUPPLY)
        }
      ]
    }
  )

  const env = {
    terra,
    wallet,
    tokenCodeID,
    pairCodeID,
    factoryCodeID,
    factoryAddress,
    insuranceFundAddress,
  }

  console.log("testSwapNativeTokenToUsd")
  await testSwapNativeTokenToUsd(env, "uluna")

  console.log("testSwapTokenToUsd")
  await testSwapTokenToUsd(env, tokenAddress)

  console.log("OK")
}

main().catch(err => console.log(err));
