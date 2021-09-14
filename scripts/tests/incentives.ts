import {
  BlockTxBroadcastResult,
  LCDClient,
  LocalTerra,
  Wallet
} from "@terra-money/terra.js"
import { strictEqual } from "assert"
import { join } from "path"
import {
  deployContract,
  executeContract,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"

// CONSTS

const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"

const INCENTIVES_UMARS_BALANCE = 1_000_000_000000

const ULUNA_UMARS_EMISSION_RATE = 2_000000
const UUSD_UMARS_EMISSION_RATE = 4_000000

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
  await setAsset(terra, deployer, oracle, "uluna", 25)
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
  await setAsset(terra, deployer, oracle, "uusd", 1)
  const maUusd = await maAssetAddress(terra, redBank, "uusd")

  console.log("set incentives")

  await setAssetIncentive(terra, deployer, incentives, maUluna, ULUNA_UMARS_EMISSION_RATE)
  await setAssetIncentive(terra, deployer, incentives, maUusd, UUSD_UMARS_EMISSION_RATE)

  // TESTS

  console.log("deposit assets")

  const aliceLunaDepositTime = await deposit(terra, alice, redBank, "uluna", X)
  const aliceUsdDepositTime = await deposit(terra, alice, redBank, "uusd", X)
  const bobLunaDepositTime = await deposit(terra, bob, redBank, "uluna", X)
  const carolLunaDepositTime = await deposit(terra, carol, redBank, "uluna", 2 * X)
  const danUsdDepositTime = await deposit(terra, dan, redBank, "uusd", X)

  const aliceClaimRewardsTime = await claimRewards(terra, alice, incentives)
  let aliceXmarsBalance = await queryBalance(terra, alice, xMars)
  let expectedAliceXmarsBalance =
    computeExpectedRewards(aliceLunaDepositTime, bobLunaDepositTime, ULUNA_UMARS_EMISSION_RATE) +
    computeExpectedRewards(bobLunaDepositTime, carolLunaDepositTime, ULUNA_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(carolLunaDepositTime, aliceClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceUsdDepositTime, danUsdDepositTime, UUSD_UMARS_EMISSION_RATE) +
    computeExpectedRewards(danUsdDepositTime, aliceClaimRewardsTime, UUSD_UMARS_EMISSION_RATE / 2)
  assertqueryBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  const bobClaimRewardsTime = await claimRewards(terra, bob, incentives)
  let bobXmarsBalance = await queryBalance(terra, bob, xMars)
  let expectedBobXmarsBalance =
    computeExpectedRewards(bobLunaDepositTime, carolLunaDepositTime, ULUNA_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(carolLunaDepositTime, bobClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 4)
  assertqueryBalance(bobXmarsBalance, expectedBobXmarsBalance)

  const carolClaimRewardsTime = await claimRewards(terra, carol, incentives)
  const carolXmarsBalance = await queryBalance(terra, carol, xMars)
  const expectedCarolXmarsBalance = computeExpectedRewards(carolLunaDepositTime, carolClaimRewardsTime, ULUNA_UMARS_EMISSION_RATE / 2)
  assertqueryBalance(carolXmarsBalance, expectedCarolXmarsBalance)

  const danClaimRewardsTime = await claimRewards(terra, dan, incentives)
  const danXmarsBalance = await queryBalance(terra, dan, xMars)
  const expectedDanXmarsBalance = computeExpectedRewards(danUsdDepositTime, danClaimRewardsTime, UUSD_UMARS_EMISSION_RATE / 2)
  assertqueryBalance(danXmarsBalance, expectedDanXmarsBalance)

  console.log("turn off uluna incentives")

  const ulunaIncentiveEndTime = await txTimestamp(
    terra,
    await executeContract(terra, deployer, incentives,
      {
        set_asset_incentive: {
          ma_token_address: maUluna,
          emission_per_second: "0"
        }
      }
    )
  )

  // Bob accrues rewards for uluna until the rewards were turned off
  await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalance(terra, bob, xMars)
  expectedBobXmarsBalance += computeExpectedRewards(bobClaimRewardsTime, ulunaIncentiveEndTime, ULUNA_UMARS_EMISSION_RATE / 4)
  assertqueryBalance(bobXmarsBalance, expectedBobXmarsBalance)

  // Alice accrues rewards for uluna until the rewards were turned off,
  // and continues to accrue rewards for uusd
  const aliceClaimRewardsTime2 = await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalance(terra, alice, xMars)
  expectedAliceXmarsBalance +=
    computeExpectedRewards(aliceClaimRewardsTime, ulunaIncentiveEndTime, ULUNA_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceClaimRewardsTime, aliceClaimRewardsTime2, UUSD_UMARS_EMISSION_RATE / 2)
  assertqueryBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  console.log("transfer uusd")

  const uusdTransferTime = await txTimestamp(
    terra,
    await executeContract(terra, alice, maUusd,
      {
        transfer: {
          recipient: bob.key.accAddress,
          amount: String(X / 2),
        }
      }
    )
  )

  // Alice accrues rewards for X uusd until transferring X/2 uusd to Bob,
  // then accrues rewards for X/2 uusd
  const aliceClaimRewardsTime3 = await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalance(terra, alice, xMars)
  expectedAliceXmarsBalance +=
    computeExpectedRewards(aliceClaimRewardsTime2, uusdTransferTime, UUSD_UMARS_EMISSION_RATE / 2) +
    computeExpectedRewards(uusdTransferTime, aliceClaimRewardsTime3, UUSD_UMARS_EMISSION_RATE / 4)
  assertqueryBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for uusd after receiving X/2 uusd from Alice
  const bobClaimRewardsTime3 = await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalance(terra, bob, xMars)
  expectedBobXmarsBalance += computeExpectedRewards(uusdTransferTime, bobClaimRewardsTime3, UUSD_UMARS_EMISSION_RATE / 4)
  assertqueryBalance(bobXmarsBalance, expectedBobXmarsBalance)

  console.log("withdraw uusd")

  const aliceWithdrawUusdTime = await withdrawUusd(terra, alice, redBank, X / 2)
  const bobWithdrawUusdTime = await withdrawUusd(terra, bob, redBank, X / 2)

  // Alice accrues rewards for X/2 uusd until withdrawing
  await claimRewards(terra, alice, incentives)
  aliceXmarsBalance = await queryBalance(terra, alice, xMars)
  expectedAliceXmarsBalance += computeExpectedRewards(aliceClaimRewardsTime3, aliceWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 4)
  assertqueryBalance(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for X/2 uusd until withdrawing
  await claimRewards(terra, bob, incentives)
  bobXmarsBalance = await queryBalance(terra, bob, xMars)
  expectedBobXmarsBalance +=
    computeExpectedRewards(bobClaimRewardsTime3, aliceWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 4) +
    computeExpectedRewards(aliceWithdrawUusdTime, bobWithdrawUusdTime, UUSD_UMARS_EMISSION_RATE / 3)
  assertqueryBalance(bobXmarsBalance, expectedBobXmarsBalance)

  console.log("OK")
}

main().catch(err => console.log(err))
