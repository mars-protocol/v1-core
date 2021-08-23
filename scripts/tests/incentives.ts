/*
LocalTerra oracle needs ~1500 ms timeouts to work. Set these with:

```
sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/1500ms/' config/config.toml
```
*/
import { BlockTxBroadcastResult, Coin, Deposit, LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import {
  deployContract,
  executeContract,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "../helpers.js"
import { strictEqual } from "assert"
import { join } from "path"

const CW_PLUS_ARTIFACTS_PATH = "../../cw-plus/artifacts"
const TERRASWAP_ARTIFACTS_PATH = "../../terraswap/artifacts"

// const USD_COLLATERAL = 100_000_000_000000
// const LUNA_COLLATERAL = 100_000_000_000000
// const USD_BORROW = 100_000_000_000000

const ULUNA_UMARS_EMISSION_RATE = 2_000000
const UUSD_UMARS_EMISSION_RATE = 4_000000

// async function getDebt(terra: LCDClient, borrower: Wallet, redBank: string) {
//   const debts = await queryContract(terra, redBank,
//     {
//       debt: {
//         address: borrower.key.accAddress
//       }
//     }
//   )

//   const debt = debts.debts.filter((coin: Coin) => coin.denom == "uusd")[0].amount

//   return parseInt(debt)
// }

async function main() {
  setTimeoutDuration(100)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

  // Check Terra uusd oracle is available, if not, try again in a few seconds
  // const activeDenoms = await terra.oracle.activeDenoms()
  // if (!activeDenoms.includes("uusd")) {
  //   throw new Error("Terra uusd oracle unavailable")
  // }

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
      // TODO is this required?
      initial_balances: [{ address: incentives, amount: String(1_000_000_000000) }],
      // initial_balances: [],
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
          fixed: {
            price: "25.0"
          }
        }
      }
    }
  )

  // maUluna token address
  const maUlunaMarket = await queryContract(terra, redBank,
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

  const maUluna = maUlunaMarket.ma_token_address

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
          fixed: {
            price: "1.0"
          }
        }
      }
    }
  )

  // maUusd token address
  const maUusdMarket = await queryContract(terra, redBank,
    {
      market: {
        asset: {
          native: {
            denom: "uusd"
          }
        }
      }
    }
  )

  const maUusd = maUusdMarket.ma_token_address

  console.log("set incentives")

  await executeContract(terra, deployer, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maUluna,
        emission_per_second: String(ULUNA_UMARS_EMISSION_RATE)
      }
    }
  )

  await executeContract(terra, deployer, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maUusd,
        emission_per_second: String(UUSD_UMARS_EMISSION_RATE)
      }
    }
  )

  // console.log(await queryContract(terra, incentives, { asset_incentive: { ma_token_address: maUluna } }))
  // console.log(await queryContract(terra, incentives, { asset_incentive: { ma_token_address: maUusd } }))

  await new Promise(resolve => setTimeout(resolve, 1001))

  console.log("deposit assets")

  // addresses
  const alice = terra.wallets.test2
  const bob = terra.wallets.test3
  const carol = terra.wallets.test4
  const dan = terra.wallets.test5
  const eve = terra.wallets.test6

  const deposit = async (wallet: Wallet, denom: string, amount: number) => {
    const result = await executeContract(terra, wallet, redBank,
      {
        deposit_native: {
          denom: denom
        }
      },
      `${amount}${denom}`
    )
    const txInfo = await terra.tx.txInfo(result.txhash)
    return Date.parse(txInfo.timestamp) / 1000 // seconds
  }

  const X = 1_000_000000

  const aliceLunaDepositTime = await deposit(alice, "uluna", X)
  const aliceUsdDepositTime = await deposit(alice, "uusd", X)
  const bobLunaDepositTime = await deposit(bob, "uluna", X)
  const carolLunaDepositTime = await deposit(carol, "uluna", 2 * X)
  const danUsdDepositTime = await deposit(dan, "uusd", X)

  // claim rewards
  const claimRewards = async (wallet: Wallet) => {
    const result = await executeContract(terra, wallet, incentives, { claim_rewards: {} })
    const txInfo = await terra.tx.txInfo(result.txhash)
    return Date.parse(txInfo.timestamp) / 1000 // seconds
  }

  const xMarsBalance = async (wallet: Wallet) => {
    const result = await queryContract(terra, xMars, { balance: { address: wallet.key.accAddress } })
    return parseInt(result.balance)
  }

  const aliceClaimRewardsTime = await claimRewards(alice)
  let aliceXmarsBalance = await xMarsBalance(alice)
  let expectedAliceXmarsBalance =
    (bobLunaDepositTime - aliceLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE +
    (carolLunaDepositTime - bobLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE / 2 +
    (aliceClaimRewardsTime - carolLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE / 4 +
    (danUsdDepositTime - aliceUsdDepositTime) * UUSD_UMARS_EMISSION_RATE +
    (aliceClaimRewardsTime - danUsdDepositTime) * UUSD_UMARS_EMISSION_RATE / 2
  strictEqual(aliceXmarsBalance, expectedAliceXmarsBalance)

  const bobClaimRewardsTime = await claimRewards(bob)
  let bobXmarsBalance = await xMarsBalance(bob)
  let expectedBobXmarsBalance =
    (carolLunaDepositTime - bobLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE / 2 +
    (bobClaimRewardsTime - carolLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE / 4
  strictEqual(bobXmarsBalance, expectedBobXmarsBalance)

  const carolClaimRewardsTime = await claimRewards(carol)
  const carolXmarsBalance = await xMarsBalance(carol)
  const expectedCarolXmarsBalance = (carolClaimRewardsTime - carolLunaDepositTime) * ULUNA_UMARS_EMISSION_RATE / 2
  strictEqual(carolXmarsBalance, expectedCarolXmarsBalance)

  const danClaimRewardsTime = await claimRewards(dan)
  const danXmarsBalance = await xMarsBalance(dan)
  const expectedDanXmarsBalance = (danClaimRewardsTime - danUsdDepositTime) * UUSD_UMARS_EMISSION_RATE / 2
  strictEqual(danXmarsBalance, expectedDanXmarsBalance)

  console.log("turn off uluna incentives")

  let result = await executeContract(terra, deployer, incentives,
    {
      set_asset_incentive: {
        ma_token_address: maUluna,
        emission_per_second: "0"
      }
    }
  )
  let txInfo = await terra.tx.txInfo(result.txhash)
  const ulunaIncentivesEndTime = Date.parse(txInfo.timestamp) / 1000

  // Bob accrues rewards for uluna until the rewards were turned off
  await claimRewards(bob)
  bobXmarsBalance = await xMarsBalance(bob)
  expectedBobXmarsBalance += (ulunaIncentivesEndTime - bobClaimRewardsTime) * ULUNA_UMARS_EMISSION_RATE / 4
  strictEqual(bobXmarsBalance, expectedBobXmarsBalance)

  // Alice accrues rewards for uluna until the rewards were turned off,
  // and continues to accrue rewards for uusd
  const aliceClaimRewardsTime2 = await claimRewards(alice)
  aliceXmarsBalance = await xMarsBalance(alice)
  expectedAliceXmarsBalance +=
    (ulunaIncentivesEndTime - aliceClaimRewardsTime) * ULUNA_UMARS_EMISSION_RATE / 4 +
    (aliceClaimRewardsTime2 - aliceClaimRewardsTime) * UUSD_UMARS_EMISSION_RATE / 2
  strictEqual(aliceXmarsBalance, expectedAliceXmarsBalance)

  console.log("transfer uusd")

  result = await executeContract(terra, alice, maUusd,
    {
      transfer: {
        recipient: bob.key.accAddress,
        amount: String(Math.floor(X / 2)),
      }
    }
  )
  txInfo = await terra.tx.txInfo(result.txhash)
  const uusdTransferTime = Date.parse(txInfo.timestamp) / 1000

  // Alice accrues rewards for X uusd until they transferred X/2 uusd to Bob,
  // then accrues rewards for X/2 uusd
  const aliceClaimRewardsTime3 = await claimRewards(alice)
  aliceXmarsBalance = await xMarsBalance(alice)
  expectedAliceXmarsBalance +=
    (uusdTransferTime - aliceClaimRewardsTime2) * UUSD_UMARS_EMISSION_RATE / 2 +
    (aliceClaimRewardsTime3 - uusdTransferTime) * UUSD_UMARS_EMISSION_RATE / 4
  strictEqual(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for uusd after receiving X/2 uusd from Alice
  const bobClaimRewardsTime3 = await claimRewards(bob)
  bobXmarsBalance = await xMarsBalance(bob)
  expectedBobXmarsBalance += (bobClaimRewardsTime3 - uusdTransferTime) * UUSD_UMARS_EMISSION_RATE / 4
  strictEqual(bobXmarsBalance, expectedBobXmarsBalance)

  console.log("withdraw uusd")

  const withdrawUusd = async (wallet: Wallet, amount: number) => {
    const result = await executeContract(terra, wallet, redBank,
      {
        withdraw: {
          asset: {
            native: {
              denom: "uusd"
            }
          },
          amount: String(amount),
        }
      }
    )
    const txInfo = await terra.tx.txInfo(result.txhash)
    return Date.parse(txInfo.timestamp) / 1000
  }

  // Alice accrues rewards for X/2 uusd until withdrawing
  const aliceWithdrawUusdTime = await withdrawUusd(alice, X / 2)
  await claimRewards(alice)
  aliceXmarsBalance = await xMarsBalance(alice)
  expectedAliceXmarsBalance +=
    (aliceWithdrawUusdTime - aliceClaimRewardsTime3) * UUSD_UMARS_EMISSION_RATE / 4
  strictEqual(aliceXmarsBalance, expectedAliceXmarsBalance)

  // Bob accrues rewards for X/2 uusd until withdrawing
  const bobWithdrawUusdTime = await withdrawUusd(alice, X / 2)
  await claimRewards(bob)
  bobXmarsBalance = await xMarsBalance(bob)
  expectedBobXmarsBalance +=
    (aliceWithdrawUusdTime - bobClaimRewardsTime3) * UUSD_UMARS_EMISSION_RATE / 4 +
    (bobWithdrawUusdTime - aliceWithdrawUusdTime) * UUSD_UMARS_EMISSION_RATE / 3
  strictEqual(bobXmarsBalance, Math.floor(expectedBobXmarsBalance))

  console.log("OK")
}

main().catch(err => console.log(err))
