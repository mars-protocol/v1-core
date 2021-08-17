/*
LocalTerra oracle needs ~1500 ms timeouts to work. Set these with:

```
sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/1500ms/' config/config.toml
```
*/
import { Coin, LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { strictEqual } from "assert"

const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 100_000_000_000000
// TODO increase `USD_BORROW` once the oracle exchange rate bug is fixed
const USD_BORROW = 2_000_000_000000

async function getDebt(terra: LCDClient, borrower: Wallet, redBank: string) {
  const debts = await queryContract(terra, redBank,
    {
      debt: {
        address: borrower.key.accAddress
      }
    }
  )

  const debt = debts.debts.filter((coin: Coin) => coin.denom == "uusd")[0].amount

  return parseInt(debt)
}

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1
  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3

  // Check Terra uusd oracle is available, if not, try again in a few seconds
  const activeDenoms = await terra.oracle.activeDenoms()
  if (!activeDenoms.includes("uusd")) {
    throw new Error("Terra uusd oracle unavailable")
  }

  console.log("upload contracts")

  const addressProvider = await deployContract(terra, deployer, "../artifacts/address_provider.wasm",
    {
      owner: deployer.key.accAddress
    }
  )

  const incentives = await deployContract(terra, deployer, "../artifacts/incentives.wasm",
    {
      owner: deployer.key.accAddress,
      address_provider_address: addressProvider
    }
  )

  const oracle = await deployContract(terra, deployer, "../artifacts/oracle.wasm",
    {
      owner: deployer.key.accAddress
    }
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
        close_factor: "0.5",
      }
    }
  )

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
        asset: {
          native: {
            denom: "uluna"
          }
        },
        asset_params: {
          initial_borrow_rate: "0.1",
          max_loan_to_value: "0.55",
          reserve_factor: "0.2",
          maintenance_margin: "0.65",
          liquidation_bonus: "0.1",
          interest_rate_strategy: {
            dynamic: {
              min_borrow_rate: "0.0",
              max_borrow_rate: "2.0",
              kp_1: "0.02",
              optimal_utilization_rate: "0.7",
              kp_augmentation_threshold: "0.15",
              kp_2: "0.05"
            }
          }
        }
      }
    }
  )

  await executeContract(terra, deployer, oracle,
    {
      set_asset: {
        asset: {
          native: {
            denom: "uluna"
          }
        },
        price_source: {
          native: {
            denom: "uluna"
          }
        }
      }
    }
  )

  // Check oracle prices
  console.log("oracle contract Luna price", await queryContract(terra, oracle,
    {
      asset_price: {
        asset: {
          native: {
            denom: "uluna"
          }
        }
      }
    }
  ))
  console.log("terra oracle Luna price", await terra.oracle.exchangeRate("uusd"))

  // uusd
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: {
          native: {
            denom: "uusd"
          }
        },
        asset_params: {
          initial_borrow_rate: "0.2",
          max_loan_to_value: "0.75",
          reserve_factor: "0.2",
          maintenance_margin: "0.85",
          liquidation_bonus: "0.1",
          interest_rate_strategy: {
            dynamic: {
              min_borrow_rate: "0.0",
              max_borrow_rate: "1.0",
              kp_1: "0.04",
              optimal_utilization_rate: "0.9",
              kp_augmentation_threshold: "0.15",
              kp_2: "0.07"
            }
          }
        }
      }
    }
  )

  await executeContract(terra, deployer, oracle,
    {
      set_asset: {
        asset: {
          native: {
            denom: "uusd"
          }
        },
        price_source: {
          native: {
            denom: "uusd"
          }
        }
      }
    }
  )

  console.log("provide usd")

  await executeContract(terra, provider, redBank,
    {
      deposit_native: {
        denom: "uusd"
      }
    },
    `${USD_COLLATERAL}uusd`
  )

  console.log("provide luna")

  await executeContract(terra, borrower, redBank,
    {
      deposit_native: {
        denom: "uluna"
      }
    },
    `${LUNA_COLLATERAL}uluna`
  )

  console.log("borrow")

  await executeContract(terra, borrower, redBank,
    {
      borrow: {
        asset: {
          native: {
            denom: "uusd"
          }
        },
        amount: String(USD_BORROW)
      }
    }
  )

  console.log("repay")

  // Repay exponentially increasing amounts
  let repay = 1_000000
  let debt = await getDebt(terra, borrower, redBank)

  while (debt > 0) {
    await executeContract(terra, borrower, redBank,
      {
        repay_native: {
          denom: "uusd"
        }
      },
      `${repay}uusd`
    )

    debt = await getDebt(terra, borrower, redBank)

    console.log("repay:", repay, "debt:", debt)

    repay *= 10
  }

  // Remaining debt is zero
  strictEqual(debt, 0)

  console.log("OK")
}

main().catch(err => console.log(err))
