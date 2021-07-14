/*
Integration test for the insurance fund contract swapping assets to UST via Terraswap.

Run the test from the root of this repo:
```
docker compose -f ../LocalTerra/docker-compose.yml up -d > /dev/null
node tests/insurance_fund.js
docker compose -f ../LocalTerra/docker-compose.yml down
```

Required directory structure:
```
$ tree -L 1 ..
.
├── LocalTerra
├── protocol
├── terraswap
```

This test works on columbus-4 with the following versions:
- LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
- terracli v0.5.0-rc0
- terraswap 72c60c05c43841499f760710a03f864c5ee4db3b

Adjust the `timeout_*` config items in `LocalTerra/config/config.toml` to make the test run faster.

TODO:
- Upgrade to columbus-5
*/

import { LocalTerra, MsgSend } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  toEncodedBinary,
  uploadContract
} from "../scripts/helpers.mjs"
import { strict as assert, strictEqual } from "assert"
import { join } from "path"

// consts and globals
const TERRASWAP_ARTIFACTS_PATH = "../terraswap/artifacts"
const TOKEN_SUPPLY = 1_000_000_000_000000
const ASSET_LP = 100_000_000_000000
const UUSD_LP = 1_000_000_000000
const INSURANCE_FUND_ASSET_BALANCE = 1_000_000_000000

const terra = new LocalTerra()
const wallet = terra.wallets.test1

// helpers
async function instantiateUusdPair(bid) {
  return await instantiateContract(terra, wallet, factoryCodeID,
    {
      "pair_code_id": pairCodeID,
      "token_code_id": tokenCodeID,
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
        "contract_addr": factoryAddress
      }
    }
  )
}

async function provideLiquidity(address, asset, coins) {
  await executeContract(terra, wallet, address,
    {
      "provide_liquidity": {
        "assets": [
          {
            "info": asset, 
            "amount": String(ASSET_LP)
          }, {
            "info": {
              "native_token": {
                "denom": "uusd"
              }
            },
            "amount": String(UUSD_LP)
          }
        ]
      }
    },
    coins,
  )
}

// upload Terraswap contracts
let tokenCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
let pairCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
let factoryCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"))

// instantiate the factory contract without `init_hook`, so that it can be a directory of pairs
let factoryAddress = await instantiateContract(terra, wallet, factoryCodeID,
  {
    "pair_code_id": pairCodeID,
    "token_code_id": tokenCodeID
  }
)

// instantiate a token contract
let tokenAddress = await instantiateContract(terra, wallet, tokenCodeID,
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

// instantiate the Mars insurance fund
let insuranceFundAddress = await deployContract(terra, wallet, join("artifacts", "insurance_fund.wasm"),
  {
    "owner": wallet.key.accAddress,
    "terraswap_factory_address": factoryAddress,
    "terraswap_max_spread": "0.01",
  }
)

// test with LUNA
const LUNA = {
  "native_token": {
    "denom": "uluna"
  }
}

// instantiate a LUNA/uUSD Terraswap pair
let lunaPairAddress = await instantiateUusdPair(LUNA)
await provideLiquidity(lunaPairAddress, LUNA, `${UUSD_LP}uusd,${ASSET_LP}uluna`)

// transfer some LUNA to the insurance fund
await performTransaction(terra, wallet,
  new MsgSend(
    wallet.key.accAddress,
    insuranceFundAddress,
    {
      uluna: INSURANCE_FUND_ASSET_BALANCE
    }
  )
)

// swap the LUNA balance in the insurance fund to uUSD
await executeContract(terra, wallet, insuranceFundAddress,
  {
    "swap_asset_to_uusd": {
      "offer_asset_info": LUNA,
      "amount": String(INSURANCE_FUND_ASSET_BALANCE)
    }
  }
)

// check the insurance fund balances
let balances = await terra.bank.balance(insuranceFundAddress)
strictEqual(balances.get("uluna"), undefined)
assert(balances.get("uusd").amount > 0)

// check the Terraswap pair balances
let pool = await queryContract(terra, lunaPairAddress,
  {
    "pool": {}
  }
)
strictEqual(pool.assets[0].amount, String(ASSET_LP + INSURANCE_FUND_ASSET_BALANCE))
assert(parseInt(pool.assets[1].amount) < UUSD_LP)


// test with a token
const TOKEN = {
  "token": {
    "contract_addr": tokenAddress
  }
}

// instantiate a token/uUSD Terraswap pair
let tokenPairAddress = await instantiateUusdPair(TOKEN)
// approve the pair contract to transfer the token
await executeContract(terra, wallet, tokenAddress,
  {
    "increase_allowance": {
      "spender": tokenPairAddress, 
      "amount": String(ASSET_LP),
    }
  }
)
await provideLiquidity(tokenPairAddress, TOKEN, `${UUSD_LP}uusd`)

// transfer some tokens to the insurance fund
await executeContract(terra, wallet, tokenAddress,
  {
    "transfer": {
      "amount": String(INSURANCE_FUND_ASSET_BALANCE),
      "recipient": insuranceFundAddress
    }
  }  
)

// swap the token balance in the insurance fund to uUSD
await executeContract(terra, wallet, insuranceFundAddress,
  {
    "swap_asset_to_uusd": {
      "offer_asset_info": TOKEN,
      "amount": String(INSURANCE_FUND_ASSET_BALANCE)
    }
  }
)

// check the insurance fund balances
let tokenBalance = await queryContract(terra, tokenAddress,
  {
    "balance": {
      "address": insuranceFundAddress
    }
  }
)
strictEqual(tokenBalance.balance, "0")
balances = await terra.bank.balance(insuranceFundAddress)
assert(balances.get("uusd").amount > 0)

// check the Terraswap pair balances
pool = await queryContract(terra, tokenPairAddress,
  {
    "pool": {}
  }
)
strictEqual(pool.assets[0].amount, String(ASSET_LP + INSURANCE_FUND_ASSET_BALANCE))
assert(parseInt(pool.assets[1].amount) < UUSD_LP)
