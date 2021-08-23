/*
LocalTerra oracle needs ~1500 ms timeouts to work. Set these with:

```
sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/1500ms/' config/config.toml
```
*/
import { isTxError, LCDClient, LocalTerra, MsgExecuteContract, Wallet } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { strict as assert } from "assert"

// consts

const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 100_000_000_000000
const USD_BORROW = 100_000_000_000000

// helpers

async function checkCollateral(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, enabled: boolean) {
  const collateral = await queryContract(terra, redBank,
    {
      collateral: {
        address: wallet.key.accAddress
      }
    }
  )

  for (const c of collateral.collateral) {
    if (c.denom == denom && c.enabled == enabled) {
      return true
    }
  }
  return false
}

// tests

async function testHealthFactorChecks(terra: LocalTerra, redBank: string, maLuna: string) {
  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3
  const recipient = terra.wallets.test4

  console.log("provider provides USD")

  await executeContract(terra, provider, redBank,
    {
      deposit_native: {
        denom: "uusd"
      }
    },
    `${USD_COLLATERAL}uusd`
  )

  console.log("provider provides Luna")

  await executeContract(terra, borrower, redBank,
    {
      deposit_native: {
        denom: "uluna"
      }
    },
    `${LUNA_COLLATERAL}uluna`
  )

  console.log("borrower borrows USD")

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

  console.log("transferring the entire maToken balance should fail")

  {
    const executeMsg = new MsgExecuteContract(recipient.key.accAddress, maLuna, {
      transfer: {
        amount: String(LUNA_COLLATERAL),
        recipient: recipient.key.accAddress
      }
    })
    const result = await performTransaction(terra, recipient, executeMsg)
    assert(isTxError(result))
  }

  console.log("transferring a small amount of the maToken balance should work")

  assert(await checkCollateral(terra, recipient, redBank, "uluna", false))

  await executeContract(terra, borrower, maLuna,
    {
      transfer: {
        amount: String(Math.floor(LUNA_COLLATERAL / 100)),
        recipient: recipient.key.accAddress
      }
    }
  )

  assert(await checkCollateral(terra, recipient, redBank, "uluna", true))
}

async function testCollateralStatusChanges(terra: LocalTerra, redBank: string, maLuna: string) {
  const provider = terra.wallets.test5
  const recipient = terra.wallets.test6

  console.log("provider provides Luna")

  await executeContract(terra, provider, redBank,
    {
      deposit_native: {
        denom: "uluna"
      }
    },
    `${LUNA_COLLATERAL}uluna`
  )

  assert(await checkCollateral(terra, provider, redBank, "uluna", true))
  assert(await checkCollateral(terra, recipient, redBank, "uluna", false))

  console.log("transferring maTokens to recipient should enable that asset as collateral")

  await executeContract(terra, provider, maLuna,
    {
      transfer: {
        amount: String(LUNA_COLLATERAL),
        recipient: recipient.key.accAddress
      }
    }
  )

  assert(await checkCollateral(terra, provider, redBank, "uluna", false))
  assert(await checkCollateral(terra, recipient, redBank, "uluna", true))
}

// main

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

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

  // maLuna token address
  const maLunaMarket = await queryContract(terra, redBank,
    {
      market: {
        asset: {
          native: {
            denom: "uluna"
          }
        }
      }
    }
  )

  const maLuna = maLunaMarket.ma_token_address

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

  // tests

  console.log("testHealthFactorChecks")
  await testHealthFactorChecks(terra, redBank, maLuna)

  console.log("testCollateralStatusChanges")
  await testCollateralStatusChanges(terra, redBank, maLuna)

  console.log("OK")
}

main().catch(err => console.log(err))
