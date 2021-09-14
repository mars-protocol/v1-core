import { LocalTerra } from "@terra-money/terra.js"
import { join } from "path"
import 'dotenv/config.js'
import {
  deployContract,
  executeContract,
  setTimeoutDuration,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"

// CONSTS

// required environment variables:
const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"

const UUSD_COLLATERAL = 1_000_000_000000
const MARS_COLLATERAL = 100_000_000_000000

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1
  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3

  console.log("upload contracts")

  const addressProvider = await deployContract(terra, deployer, "../artifacts/address_provider.wasm",
    { owner: deployer.key.accAddress }
  )

  const incentives = await deployContract(terra, deployer, "../artifacts/incentives.wasm",
    {
      owner: deployer.key.accAddress,
      address_provider_address: addressProvider
    }
  )

  const oracle = await deployContract(terra, deployer, "../artifacts/oracle.wasm",
    { owner: deployer.key.accAddress }
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
      initial_balances: [{ address: borrower.key.accAddress, amount: String(MARS_COLLATERAL) }],
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
            linear: {
              optimal_utilization_rate: "1",
              base: "0",
              slope_1: "1",
              slope_2: "0",
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
            linear: {
              optimal_utilization_rate: "1",
              base: "0",
              slope_1: "1",
              slope_2: "0",
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

  console.log("provide uusd")

  await executeContract(terra, provider, redBank,
    { deposit_native: { denom: "uusd" } },
    `${UUSD_COLLATERAL}uusd`
  )

  console.log("provide mars")

  await executeContract(terra, borrower, mars,
    {
      send: {
        contract: redBank,
        amount: String(MARS_COLLATERAL),
        msg: toEncodedBinary({ deposit_cw20: {} })
      }
    }
  )

  console.log("borrow uusd")

  await executeContract(terra, borrower, redBank,
    {
      borrow: {
        asset: { native: { denom: "uusd" } },
        amount: String(UUSD_COLLATERAL)
      }
    }
  )

  console.log("OK")
}

main().catch(err => console.log(err))
