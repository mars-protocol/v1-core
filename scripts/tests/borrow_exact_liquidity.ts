import { LocalTerra } from "@terra-money/terra.js"
import { join } from "path"
import {
  deployContract,
  executeContract,
  setTimeoutDuration,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"

// CONSTS
const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"

const BORROW_CW20_UUSD_COLLATERAL = 100_000_000_000000
const BORROW_CW20_MARS_COLLATERAL = 1_000_000_000000

const BORROW_NATIVE_UUSD_COLLATERAL = 1_000_000_000000
const BORROW_NATIVE_MARS_COLLATERAL = 100_000_000_000000

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

  const mars = await deployContract(terra, deployer, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"),
    {
      name: "Mars",
      symbol: "MARS",
      decimals: 6,
      initial_balances: [
        {
          address: provider.key.accAddress,
          amount: String(BORROW_CW20_MARS_COLLATERAL)
        }, {
          address: borrower.key.accAddress,
          amount: String(BORROW_NATIVE_MARS_COLLATERAL)
        }
      ],
      mint: { minter: deployer.key.accAddress },
    }
  )

  await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          incentives_address: incentives,
          mars_token_address: mars,
          oracle_address: oracle,
          red_bank_address: redBank,
          protocol_admin_address: deployer.key.accAddress,
        }
      }
    }
  )



  console.log("init assets")

  // mars
  await executeContract(terra, deployer, redBank,
    {
      init_asset: {
        asset: { cw20: { contract_addr: mars } },
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
        asset: { cw20: { contract_addr: mars } },
        price_source: { fixed: { price: "2" } }
      }
    }
  )

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
        asset: { native: { denom: "uusd" } },
        price_source: { fixed: { price: "1" } }
      }
    }
  )

  // TESTS

  console.log("borrow cw20")

  console.log("provide mars")

  await executeContract(terra, provider, mars,
    {
      send: {
        contract: redBank,
        amount: String(BORROW_CW20_MARS_COLLATERAL),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )

  console.log("provide uusd")

  await executeContract(terra, borrower, redBank,
    { deposit_native: { denom: "uusd" } },
    `${BORROW_CW20_UUSD_COLLATERAL}uusd`
  )

  console.log("borrow mars")

  await executeContract(terra, borrower, redBank,
    {
      borrow: {
        asset: { cw20: { contract_addr: mars } },
        amount: String(BORROW_CW20_MARS_COLLATERAL)
      }
    }
  )


  console.log("borrow native token")

  console.log("provide uusd")

  await executeContract(terra, provider, redBank,
    { deposit_native: { denom: "uusd" } },
    `${BORROW_NATIVE_UUSD_COLLATERAL}uusd`
  )

  console.log("provide mars")

  await executeContract(terra, borrower, mars,
    {
      send: {
        contract: redBank,
        amount: String(BORROW_NATIVE_MARS_COLLATERAL),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )

  console.log("borrow uusd")

  await executeContract(terra, borrower, redBank,
    {
      borrow: {
        asset: { native: { denom: "uusd" } },
        amount: String(BORROW_NATIVE_UUSD_COLLATERAL)
      }
    }
  )

  console.log("OK")
}

main().catch(err => console.log(err))
