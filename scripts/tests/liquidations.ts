import {
  BlockTxBroadcastResult,
  Int,
  isTxError,
  LCDClient,
  LocalTerra,
  MsgExecuteContract,
  Wallet
} from "@terra-money/terra.js"
import { strictEqual, strict as assert } from "assert"
import { join } from "path"
import {
  deployContract,
  executeContract,
  executeContractFails,
  mayExecuteContract,
  queryContract,
  setTimeoutDuration,
  sleep,
  uploadContract,
} from "../helpers.js"

// CONSTS

const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"

const LUNA_MAX_LTV = 0.55
const USD_MAX_LTV = 0.75

const LUNA_USD_PRICE = 25
const USD_COLLATERAL = 100_000_000_000000
const LUNA_COLLATERAL = 1_000_000000
const USD_BORROW = LUNA_COLLATERAL * LUNA_USD_PRICE * LUNA_MAX_LTV // 13750

// multiples of coins to deposit and withdraw from the red bank
const X = 10_000_000000

// HELPERS

async function maAssetAddress(terra: LCDClient, redBank: string, denom: string) {
  const market = await queryContract(terra, redBank, { market: { asset: { native: { denom: denom } } } })
  return market.ma_token_address
}

async function setAsset(terra: LCDClient, wallet: Wallet, oracle: string, denom: string, price: number) {
  await executeContract(terra, wallet, oracle,
    {
      set_asset: {
        asset: { native: { denom: denom } },
        price_source: { fixed: { price: String(price) } }
      }
    }
  )
}

async function setAssetIncentive(terra: LCDClient, wallet: Wallet, incentives: string, maTokenAddress: string, umarsEmissionRate: number) {
  await executeContract(terra, wallet, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maTokenAddress,
        emission_per_second: String(umarsEmissionRate)
      }
    }
  )
}

async function txTimestamp(terra: LCDClient, result: BlockTxBroadcastResult) {
  const txInfo = await terra.tx.txInfo(result.txhash)
  return Date.parse(txInfo.timestamp) / 1000 // seconds
}

async function deposit(terra: LCDClient, wallet: Wallet, redBank: string, denom: string, amount: number) {
  const result = await executeContract(terra, wallet, redBank, { deposit_native: { denom: denom } }, `${amount}${denom}`)
  return await txTimestamp(terra, result)
}

async function claimRewards(terra: LCDClient, wallet: Wallet, incentives: string) {
  const result = await executeContract(terra, wallet, incentives, { claim_rewards: {} })
  return await txTimestamp(terra, result)
}

async function queryBalance(terra: LCDClient, wallet: Wallet, token: string) {
  const result = await queryContract(terra, token, { balance: { address: wallet.key.accAddress } })
  return parseInt(result.balance)
}

function computeExpectedRewards(startTime: number, endTime: number, umarsRate: number) {
  return (endTime - startTime) * umarsRate
}

async function withdrawUusd(terra: LCDClient, wallet: Wallet, redBank: string, amount: number) {
  const result = await executeContract(terra, wallet, redBank,
    {
      withdraw: {
        asset: { native: { denom: "uusd" } },
        amount: String(amount),
      }
    }
  )
  return await txTimestamp(terra, result)
}

function assertqueryBalance(balance: number, expectedBalance: number) {
  return strictEqual(balance, Math.floor(expectedBalance))
}

// TESTS

// async function testLiquidatorReceivesMaToken() {

// }

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()

  // addresses
  const deployer = terra.wallets.test1
  // const depositor = terra.wallets.test2
  // const borrower = terra.wallets.test3
  // const liquidator = terra.wallets.test4

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

  const tokenCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const terraswapFactory = await deployContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"),
    {
      "pair_code_id": pairCodeID,
      "token_code_id": tokenCodeID
    }
  )

  const staking = await deployContract(terra, deployer, "../artifacts/staking.wasm",
    {
      config: {
        owner: deployer.key.accAddress,
        address_provider_address: addressProvider,
        terraswap_factory_address: terraswapFactory,
        terraswap_max_spread: "0.05",
        cooldown_duration: 10,
        unstake_window: 300,
      }
    }
  )

  const mars = await deployContract(terra, deployer, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"),
    {
      name: "Mars",
      symbol: "MARS",
      decimals: 6,
      initial_balances: [],
      mint: { minter: incentives },
    }
  )

  const xMars = await deployContract(terra, deployer, "../artifacts/xmars_token.wasm",
    {
      name: "xMars",
      symbol: "xMARS",
      decimals: 6,
      initial_balances: [],
      mint: { minter: staking },
    }
  )

  // update address provider
  const tmp = await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          incentives_address: incentives,
          mars_token_address: mars,
          oracle_address: oracle,
          red_bank_address: redBank,
          staking_address: staking,
          xmars_token_address: xMars,
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
        asset: { native: { denom: "uluna" } },
        asset_params: {
          initial_borrow_rate: "0.1",
          max_loan_to_value: String(LUNA_MAX_LTV),
          reserve_factor: "0.2",
          maintenance_margin: String(LUNA_MAX_LTV + 0.001),
          liquidation_bonus: "0.1",
          interest_rate_strategy: {
            linear: {
              optimal_utilization_rate: "0",
              base: "10000",
              slope_1: "0",
              slope_2: "0",
            }
          }
        }
      }
    }
  )
  await setAsset(terra, deployer, oracle, "uluna", LUNA_USD_PRICE)
  const maUluna = await maAssetAddress(terra, redBank, "uluna")

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
              optimal_utilization_rate: "0",
              base: "10000",
              slope_1: "0",
              slope_2: "0",
            }
          }
        }
      }
    }
  )
  await setAsset(terra, deployer, oracle, "uusd", 1)
  const maUusd = await maAssetAddress(terra, redBank, "uusd")

  const provider = terra.wallets.test2
  const borrower = terra.wallets.test3
  const liquidator = terra.wallets.test4

  console.log("provider provides USD")

  await executeContract(terra, provider, redBank,
    {
      deposit_native: {
        denom: "uusd"
      }
    },
    `${USD_COLLATERAL}uusd`
  )

  console.log("borrower provides Luna")

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
        amount: String(Math.floor(USD_BORROW * 0.01))
      }
    }
  )

  assert(await executeContractFails(terra, liquidator, redBank,
    {
      liquidate_native: {
        collateral_asset: { native: { denom: "uluna" } },
        debt_asset: "uusd",
        user_address: borrower.key.accAddress,
        receive_ma_token: true,
      }
    }, `${Math.floor(USD_BORROW * 0.01)}uusd`
  ))

  console.log("borrow more")

  await executeContract(terra, borrower, redBank,
    {
      borrow: {
        asset: {
          native: {
            denom: "uusd"
          }
        },
        amount: String(Math.floor(USD_BORROW * 0.98))
      }
    }
  )

  const ulunaBalanceBefore = await getBalance(terra, liquidator.key.accAddress, "uluna")
  const uusdBalanceBefore = await getBalance(terra, liquidator.key.accAddress, "uusd")
  console.log(uusdBalanceBefore)

  // TODO use UserPosition query to get the health factor
  let backoff = 1
  while (true) {
    console.log(await queryContract(terra, redBank, { debt: { address: borrower.key.accAddress } }))
    console.log(await queryContract(terra, redBank, { collateral: { address: borrower.key.accAddress } }))

    const result = await mayExecuteContract(terra, liquidator, redBank,
      {
        liquidate_native: {
          collateral_asset: { native: { denom: "uluna" } },
          debt_asset: "uusd",
          user_address: borrower.key.accAddress,
          receive_ma_token: false,
        }
      }, `${Math.floor(USD_BORROW * 0.4)}uusd`
    )

    if (!isTxError(result)) {
      break
    }

    console.log(`backing off ${backoff} s`)
    await sleep(backoff * 1000)
    backoff *= 2
  }

  const ulunaBalanceAfter = await getBalance(terra, liquidator.key.accAddress, "uluna")
  const uusdBalanceAfter = await getBalance(terra, liquidator.key.accAddress, "uusd")

  const uusdBalanceDifference = uusdBalanceBefore.sub(uusdBalanceAfter)




  console.log(uusdBalanceAfter)
  console.log("uluna delta", ulunaBalanceAfter.sub(ulunaBalanceBefore))
  console.log("uusd delta", uusdBalanceDifference)
  assert(ulunaBalanceAfter.gt(ulunaBalanceBefore))
  assert(uusdBalanceAfter.equals(uusdBalanceBefore))
  // console.log(await queryContract(terra, maUluna, { balance: { address: liquidator.key.accAddress } }))



  console.log("OK")
}

async function getBalance(terra: LCDClient, address: string, denom: string) {
  const balances = await terra.bank.balance(address)
  const balance = balances.get(denom)
  if (balance === undefined) {
    return new Int(0)
  }
  return balance.amount
}

main().catch(err => console.log(err))
