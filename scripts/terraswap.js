import { LocalTerra } from "@terra-money/terra.js";
import { deployContract, executeContract, instantiateContract, queryContract, uploadContract, toEncodedBinary } from "./helpers.mjs";
import { strict as assert, strictEqual } from "assert";
import { join } from "path";

const TERRASWAP_ARTIFACTS_PATH = "../terraswap/artifacts"
const TOKEN_SUPPLY = 1_000_000_000_000000
const TOKEN_LP = 100_000_000_000000
const UUSD_LP = 1_000_000_000000
const TOKEN_INSURANCE_FUND = 1_000_000_000000

const terra = new LocalTerra();
const wallet = terra.wallets.test1;

// deploy a token contract
let tokenCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))

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

// check the token balance is correct for the wallet
let balance = await queryContract(terra, tokenAddress,
  {
    "balance": {
      "address": wallet.key.accAddress
    }
  }
)

strictEqual(balance.balance, String(TOKEN_SUPPLY))

// deploy a Terraswap pair contract for the token
let pairCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))

let factoryCodeID = await uploadContract(terra, wallet, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"))

// first, instantiate the factory contract without `init_hook`, so that it can be a directory of pairs
let factoryAddress = await instantiateContract(terra, wallet, factoryCodeID,
  {
    "pair_code_id": pairCodeID,
    "token_code_id": tokenCodeID
  }
)

// then, instantiate a pair using the factory contract
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

// approve the pair contract to transfer the token
await executeContract(terra, wallet, tokenAddress,
  {
    "increase_allowance": {
      "spender": pairAddress, 
      "amount": String(TOKEN_LP),
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
          "amount": String(TOKEN_LP)
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
  String(UUSD_LP)+"uusd"
)

// check the balances in the pool are correct
let pool = await queryContract(terra, pairAddress,
  {
    "pool": {}
  }
)

strictEqual(pool.assets[0].amount, String(TOKEN_LP))
strictEqual(pool.assets[1].amount, String(UUSD_LP))

// deploy the Mars insurance fund
let insuranceFundAddress = await deployContract(terra, wallet, join("artifacts", "insurance_fund.wasm"),
  {
    "owner": wallet.key.accAddress,
    "terraswap_factory_address": factoryAddress,
    "terraswap_max_spread": "0.05",
  }
)

// transfer some tokens to the insurance fund
await executeContract(terra, wallet, tokenAddress,
  {
    "transfer": {
      "amount": String(TOKEN_INSURANCE_FUND),
      "recipient": insuranceFundAddress
    }
  }  
)

// check the token balance of the insurance fund is correct
balance = await queryContract(terra, tokenAddress,
  {
    "balance": {
      "address": insuranceFundAddress
    }
  }
)

strictEqual(balance.balance, String(TOKEN_INSURANCE_FUND))

// swap some of the token balance for UST
let res = await executeContract(terra, wallet, insuranceFundAddress,
  {
    "swap_asset_to_uusd": {
      "offer_asset_info": {
        "token": {
          "contract_addr": tokenAddress
        }
      },
      "amount": String(TOKEN_INSURANCE_FUND)
    }
  }
)

// check the token balance of the insurance fund is correct
balance = await queryContract(terra, tokenAddress,
  {
    "balance": {
      "address": insuranceFundAddress
    }
  }
)

strictEqual(balance.balance, "0")

// check the UST balance of the insurance fund is correct
let insuranceFundBalances = await terra.bank.balance(insuranceFundAddress)

assert(insuranceFundBalances.get("uusd").amount > 0)

// check the balances in the pool are correct
pool = await queryContract(terra, pairAddress,
  {
    "pool": {}
  }
)

strictEqual(pool.assets[0].amount, String(TOKEN_LP + TOKEN_INSURANCE_FUND))
assert(parseInt(pool.assets[1].amount) < UUSD_LP)

console.log("DONE")