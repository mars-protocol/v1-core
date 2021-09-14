import { Coin, LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { strictEqual } from "assert"

// CONSTS

const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 100_000_000_000000
const USD_BORROW = 100_000_000_000000

// HELPERS

async function getDebt(terra: LCDClient, borrower: Wallet, redBank: string) {
  const debts = await queryContract(terra, redBank,
    { user_debt: { user_address: borrower.key.accAddress } }
  )

  const debt = debts.debts.filter((coin: any) => coin.denom == "uusd")[0].amount_scaled

  return parseInt(debt)
}

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1
  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3

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
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )

  await executeContract(terra, deployer, oracle,
    {
      set_asset: {
        asset: { native: { denom: "uluna" } },
        price_source: { fixed: { price: "25" } }
      }
    }
  )

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
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )

  await executeContract(terra, deployer, oracle,
    {
      set_asset: {
        asset: { native: { denom: "uusd" } },
        price_source: { fixed: { price: "1" } }
      }
    }
  )

  // TESTS

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
