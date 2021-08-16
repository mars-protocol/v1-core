import { LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import { deployContract, executeContract, setTimeoutDuration, uploadContract } from "../helpers.js"

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const wallet = terra.wallets.test1

  // console.log(await terra.oracle.votes("uusd"))
  // console.log(await terra.oracle.activeDenoms())

  const addressProvider = await deployContract(terra, wallet, "../artifacts/address_provider.wasm", { owner: wallet.key.accAddress })

  const incentives = await deployContract(terra, wallet, "../artifacts/incentives.wasm",
    {
      owner: wallet.key.accAddress,
      address_provider_address: addressProvider
    }
  )

  const oracle = await deployContract(terra, wallet, "../artifacts/oracle.wasm", { owner: wallet.key.accAddress })

  const maTokenCodeId = await uploadContract(terra, wallet, "../artifacts/ma_token.wasm")

  const redBank = await deployContract(terra, wallet, "../artifacts/red_bank.wasm",
    {
      config: {
        owner: wallet.key.accAddress,
        address_provider_address: addressProvider,
        insurance_fund_fee_share: "0.1",
        treasury_fee_share: "0.2",
        ma_token_code_id: maTokenCodeId,
        close_factor: "0.5",
      }
    }
  )

  await executeContract(terra, wallet, addressProvider,
    {
      update_config: {
        config: {
          owner: wallet.key.accAddress,
          incentives_address: incentives,
          oracle_address: oracle,
          red_bank_address: redBank
        }
      }
    }
  )

  console.log("init assets")

  await executeContract(terra, wallet, redBank,
    {
      init_asset: {
        asset: {
          native: {
            denom: "uluna"
          }
        },
        asset_params: {
          initial_borrow_rate: "0.1",
          min_borrow_rate: "0.0",
          max_borrow_rate: "2.0",
          max_loan_to_value: "0.55",
          reserve_factor: "0.2",
          maintenance_margin: "0.65",
          liquidation_bonus: "0.1",
          kp_1: "0.02",
          optimal_utilization_rate: "0.7",
          kp_augmentation_threshold: "0.15",
          kp_2: "0.05"
        }
      }
    }
  )

  await executeContract(terra, wallet, redBank,
    {
      init_asset: {
        asset: {
          native: {
            denom: "uusd"
          }
        },
        asset_params: {
          initial_borrow_rate: "0.2",
          min_borrow_rate: "0.0",
          max_borrow_rate: "1.0",
          max_loan_to_value: "0.75",
          reserve_factor: "0.2",
          maintenance_margin: "0.85",
          liquidation_bonus: "0.1",
          kp_1: "0.04",
          optimal_utilization_rate: "0.9",
          kp_augmentation_threshold: "0.15",
          kp_2: "0.07"
        }
      }
    }
  )

  console.log("provide usd")

  const provider = terra.wallets.test2
  await executeContract(terra, provider, redBank,
    {
      deposit_native: {
        denom: "uusd"
      }
    },
    String(1_000_000_000000) + "uusd"
  )

  console.log("provide luna")

  const borrower = terra.wallets.test3
  await executeContract(terra, borrower, redBank,
    {
      deposit_native: {
        denom: "uluna"
      }
    },
    String(100_000_000000) + "uluna"
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
        amount: String(1_000_000000)
      }
    }
  )

  console.log("repay")

  const repay = generateRepayUstFunction(terra, borrower, redBank)

  await repay(1_000000)

  await executeContract(terra, borrower, redBank,
    {
      repay_native: {
        denom: "uusd"
      }
    },
    String(1_000000) + "uusd"
  )
}

main().catch(err => console.log(err))

function generateRepayUstFunction(terra: LCDClient, borrower: Wallet, redBank: string) {
  return async function (amount: number) {
    await executeContract(terra, borrower, redBank,
      {
        repay_native: {
          denom: "uusd"
        }
      },
      String(1_000000) + "uusd"
    )
  }
}
