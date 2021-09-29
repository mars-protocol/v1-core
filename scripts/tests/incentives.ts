import {
  LCDClient,
  LocalTerra,
  Wallet
} from "@terra-money/terra.js"
import { strictEqual } from "assert"
import { join } from "path"
import 'dotenv/config.js'
import {
  deployContract,
  executeContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import {
  depositNative,
  getTxTimestamp,
  queryBalanceCw20,
  queryMaAssetAddress,
  setAssetOraclePriceSource,
  transferCw20,
  withdraw
} from "./test_helpers.js"

// CONSTS

// required environment variables:
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!
const TERRASWAP_ARTIFACTS_PATH = process.env.TERRASWAP_ARTIFACTS_PATH!

const INCENTIVES_UMARS_BALANCE = 1_000_000_000000
const ULUNA_UMARS_EMISSION_RATE = 2_000000
const UUSD_UMARS_EMISSION_RATE = 4_000000
const MA_TOKEN_SCALING_FACTOR = 1_000_000

// multiples of coins to deposit and withdraw from the red bank
const X = 10_000_000000

// HELPERS

async function setAssetIncentive(
  terra: LCDClient,
  wallet: Wallet,
  incentives: string,
  maTokenAddress: string,
  umarsEmissionRate: number,
) {
  await executeContract(terra, wallet, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maTokenAddress,
        emission_per_second: String(umarsEmissionRate)
      }
    }
  )
}

async function claimRewards(
  terra: LCDClient,
  wallet: Wallet,
  incentives: string,
) {
  const result = await executeContract(terra, wallet, incentives, { claim_rewards: {} })
  return await getTxTimestamp(terra, result)
}

function computeExpectedRewards(
  startTime: number,
  endTime: number,
  umarsRate: number,
) {
  return (endTime - startTime) * umarsRate
}

function assertBalance(
  balance: number,
  expectedBalance: number,
) {
  return strictEqual(balance, Math.floor(expectedBalance))
}

// MAIN

async function main() {
  // SETUP

  setTimeoutDuration(100)

  const terra = new LocalTerra()

  // addresses
  const deployer = terra.wallets.test1
  const alice = terra.wallets.test2
  const bob = terra.wallets.test3
  const carol = terra.wallets.test4
  const dan = terra.wallets.test5

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
        safety_fund_fee_share: "0.1",
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
        astroport_factory_address: terraswapFactory,
        astroport_max_spread: "0.05",
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
      initial_balances: [{ address: incentives, amount: String(INCENTIVES_UMARS_BALANCE) }],
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
  await executeContract(terra, deployer, addressProvider,
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

  await setAssetOraclePriceSource(terra, deployer, oracle,
    { native: { denom: "uluna" } },
    25
  )

  const maUluna = await queryMaAssetAddress(terra, redBank, { native: { denom: "uluna" } })

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
          },
          active: true,
          deposit_enabled: true,
          borrow_enabled: true
        }
      }
    }
  )

  await setAssetOraclePriceSource(terra, deployer, oracle,
    { native: { denom: "uusd" } },
    1
  )

  const maUusd = await queryMaAssetAddress(terra, redBank, { native: { denom: "uusd" } })

  console.log("set incentives")

  await setAssetIncentive(terra, deployer, incentives, maUluna, ULUNA_UMARS_EMISSION_RATE)
  await setAssetIncentive(terra, deployer, incentives, maUusd, UUSD_UMARS_EMISSION_RATE)

  // TESTS

  console.log("deposit assets")

  let result = await depositNative(terra, alice, redBank, "uluna", X)
  const aliceLunaDepositTime = await getTxTimestamp(terra, result)

  result = await depositNative(terra, alice, redBank, "uusd", X)
  const aliceUsdDepositTime = await getTxTimestamp(terra, result)

  result = await depositNative(terra, bob, redBank, "uluna", X)
  const bobLunaDepositTime = await getTxTimestamp(terra, result)

  result = await depositNative(terra, carol, redBank, "uluna", 2 * X)
  const carolLunaDepositTime = await getTxTimestamp(terra, result)

  result = await depositNative(terra, dan, redBank, "uusd", X)
  const danUsdDepositTime = await getTxTimestamp(terra, result)

  const aliceClaimRewardsTime = await claimRewards(terra, alice, incentives)
  let aliceXmarsBalance = await queryBalanceCw20(terra, alice.key.accAddress, xMars)
  let expectedAliceXmarsBalance =
    computeExpectedRewards(aliceLunaDepositTime, bobLunaDepositTime, ULUNA_UMARS_EMISSION_RATE) +
    computeExpectedRewards(bobLunaDepositTime, carolLunaDepositTime, ULUNA_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(carolLunaDepositTime, aliceClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceUsdDepositTime, danUsdDepositTime, UUSD_UMARS_EMISSION_RATE) +
    computeExpectedRewards(danUsdDepositTime, aliceClaimRewardsTime, UUSD_UMARS_EMISSION_RATE / 2)
  assertBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  const bobClaimRewardsTime = await claimRewards(terra, bob, incentives)
  let bobXmarsBalance = await queryBalanceCw20(terra, bob.key.accAddress, xMars)
  let expectedBobXmarsBalance =
    computeExpectedRewards(bobLunaDepositTime, carolLunaDepositTime, ULUNA_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(carolLunaDepositTime, bobClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 4)
  assertBalance(bobXmarsBalance, expectedBobXmarsBalance)

  const carolClaimRewardsTime = await claimRewards(terra, carol, incentives)
  const carolXmarsBalance = await queryBalanceCw20(terra, carol.key.accAddress, xMars)
  const expectedCarolXmarsBalance = computeExpectedRewards(carolLunaDepositTime, carolClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 2)
  assertBalance(carolXmarsBalance, expectedCarolXmarsBalance)

  const danClaimRewardsTime = await claimRewards(terra, dan, incentives)
  const danXmarsBalance = await queryBalanceCw20(terra, dan.key.accAddress, xMars)
  const expectedDanXmarsBalance = computeExpectedRewards(danUsdDepositTime, danClaimRewardsTime, UUSD_UMARS_EMISSION_RATE / 2)
  assertBalance(danXmarsBalance, expectedDanXmarsBalance)

  console.log("turn off uluna incentives")

  result = await executeContract(terra, deployer, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maUluna,
        emission_per_second: "0"
      }
    }
  )
  const ulunaIncentiveEndTime = await getTxTimestamp(terra, result)

  // Bob accrues rewards for uluna until the rewards were turned off
  await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalanceCw20(terra, bob.key.accAddress, xMars)
  expectedBobXmarsBalance +=
    computeExpectedRewards(bobClaimRewardsTime, ulunaIncentiveEndTime, ULUNA_UMARS_EMISSION_RATE / 4)
  assertBalance(bobXmarsBalance, expectedBobXmarsBalance)

  // Alice accrues rewards for uluna until the rewards were turned off,
  // and continues to accrue rewards for uusd
  const aliceClaimRewardsTime2 = await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalanceCw20(terra, alice.key.accAddress, xMars)
  expectedAliceXmarsBalance +=
    computeExpectedRewards(aliceClaimRewardsTime, ulunaIncentiveEndTime, ULUNA_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceClaimRewardsTime, aliceClaimRewardsTime2, UUSD_UMARS_EMISSION_RATE / 2)
  assertBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  console.log("transfer maUusd")

  result = await transferCw20(terra, alice, maUusd, bob.key.accAddress, X / 2 * MA_TOKEN_SCALING_FACTOR)
  const uusdTransferTime = await getTxTimestamp(terra, result)

  // Alice accrues rewards for X uusd until transferring X/2 uusd to Bob,
  // then accrues rewards for X/2 uusd
  const aliceClaimRewardsTime3 = await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalanceCw20(terra, alice.key.accAddress, xMars)
  expectedAliceXmarsBalance +=
    computeExpectedRewards(aliceClaimRewardsTime2, uusdTransferTime, UUSD_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(uusdTransferTime, aliceClaimRewardsTime3, UUSD_UMARS_EMISSION_RATE / 4)
  assertBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for uusd after receiving X/2 uusd from Alice
  const bobClaimRewardsTime3 = await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalanceCw20(terra, bob.key.accAddress, xMars)
  expectedBobXmarsBalance +=
    computeExpectedRewards(uusdTransferTime, bobClaimRewardsTime3, UUSD_UMARS_EMISSION_RATE / 4)
  assertBalance(bobXmarsBalance, expectedBobXmarsBalance)

  console.log("withdraw uusd")

  result = await withdraw(terra, alice, redBank, { native: { denom: "uusd" } }, X / 2)
  const aliceWithdrawUusdTime = await getTxTimestamp(terra, result)
  result = await withdraw(terra, bob, redBank, { native: { denom: "uusd" } }, X / 2)
  const bobWithdrawUusdTime = await getTxTimestamp(terra, result)

  // Alice accrues rewards for X/2 uusd until withdrawing
  await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalanceCw20(terra, alice.key.accAddress, xMars)
  expectedAliceXmarsBalance +=
    computeExpectedRewards(aliceClaimRewardsTime3, aliceWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 4)
  assertBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for X/2 uusd until withdrawing
  await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalanceCw20(terra, bob.key.accAddress, xMars)
  expectedBobXmarsBalance +=
    computeExpectedRewards(bobClaimRewardsTime3, aliceWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceWithdrawUusdTime, bobWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 3)
  assertBalance(bobXmarsBalance, expectedBobXmarsBalance)

  console.log("OK")
}

main().catch(err => console.log(err))
