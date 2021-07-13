import { LocalTerra } from "@terra-money/terra.js";
import { executeContract, instantiateContract, queryContract, uploadContract, toEncodedBinary } from "./helpers.mjs";
import { strictEqual } from "assert";
import { join } from "path";

const TERRASWAP_ARTIFACTS_PATH = "../terraswap/artifacts";
const TOKEN_INIT_AMOUNT = "10000"

const terra = new LocalTerra();
const wallet = terra.wallets.test1;

// create a token
let tokenCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))

let tokenAddress = await instantiateContract(terra, wallet, tokenCodeID,
  {
    "name": "Mars",
    "symbol": "MARS",
    "decimals": 2,
    "initial_balances": [
      {
        "address": wallet.key.accAddress,
        "amount": TOKEN_INIT_AMOUNT
      }
    ]
  }
)

// check balance is correct
let balance = await queryContract(terra, tokenAddress,
  {
    "balance": {
      "address": wallet.key.accAddress
    }
  }
)

strictEqual(balance.balance, TOKEN_INIT_AMOUNT)

// create a pair
let pairCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))

let factoryCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"))

// instantiate the factory contract to act as a directory of instantiated pairs
let factoryAddress = await instantiateContract(terra, wallet, factoryCodeID,
  {
    "pair_code_id": pairCodeID,
    "token_code_id": tokenCodeID
  }
)

// instantiate a pair
let pairAddress = await instantiateContract(terra, wallet, factoryCodeID,
  {
    "pair_code_id": pairCodeID,
    "token_code_id": tokenCodeID,
    "init_hook": {
      "msg": toEncodedBinary(
        {
          "create_pair": {
            "asset_infos": [
              {
                "token": {
                  "contract_addr": tokenAddress
                }
              }, {
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

// approve pair contract to spend the token
await executeContract(terra, wallet, tokenAddress,
  {
    "increase_allowance": {
      "spender": pairAddress, 
      "amount": "10"
    }
  }
)

// LP
await executeContract(terra, wallet, pairAddress,
  {
    "provide_liquidity": {
      "assets": [
        {
          "info": {
            "token": {
              "contract_addr": tokenAddress
            }
          }, 
          "amount": "10"
        }, {
          "info": {
            "native_token": {
              "denom": "uusd"
            }
          }, "amount": "10"
        }
      ]
    }
  },
  "10uusd"
)

// check the pool is correct
let pool = await queryContract(terra, pairAddress,
  {
    "pool": {}
  }
)

strictEqual(pool.assets[0].amount, "10")
strictEqual(pool.assets[1].amount, "10")

console.log("DONE")