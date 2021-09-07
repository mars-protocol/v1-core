import {
  BlockTxBroadcastResult,
  LCDClient,
  LocalTerra,
  MsgSend,
} from "@terra-money/terra.js"
import { strictEqual, strict as assert } from "assert"
import { join } from "path"
import 'dotenv/config.js'
import {
  deployContract,
  executeContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  sleep,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"

// CONSTS

// required environment variables:
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!
const TERRASWAP_ARTIFACTS_PATH = process.env.TERRASWAP_ARTIFACTS_PATH!
// targetted block time in ms, which is set in LocalTerra/config/config.toml.
// used to correct for LocalTerra's clock not being accurate
const BLOCK_TIME_MILLISECONDS = parseInt(process.env.BLOCK_TIME_MILLISECONDS!)
const BLOCK_TIME_SECONDS = BLOCK_TIME_MILLISECONDS / 1000

const COOLDOWN_DURATION_SECONDS = 2
const UNSTAKE_WINDOW_DURATION_SECONDS = 3
const MARS_STAKE_AMOUNT = 1_000_000000
const ULUNA_SWAP_AMOUNT = 100_000000

const LUNA_USD_PRICE = 25
const ULUNA_UUSD_PAIR_ULUNA_LP_AMOUNT = 1_000_000_000000
const ULUNA_UUSD_PAIR_UUSD_LP_AMOUNT = ULUNA_UUSD_PAIR_ULUNA_LP_AMOUNT * LUNA_USD_PRICE
const MARS_USD_PRICE = 2
const MARS_UUSD_PAIR_MARS_LP_AMOUNT = 1_000_000_000000
const MARS_UUSD_PAIR_UUSD_LP_AMOUNT = MARS_UUSD_PAIR_MARS_LP_AMOUNT * MARS_USD_PRICE

// HELPERS

async function assertXmarsBalance(terra: LCDClient, xMars: string, address: string, expectedBalance: number) {
  const balance = await queryCw20Balance(terra, address, xMars)
  strictEqual(balance, expectedBalance)
}

async function assertXmarsBalanceAt(terra: LCDClient, xMars: string, address: string, block: number, expectedBalance: number) {
  const xMarsBalance = await queryContract(terra, xMars, { balance_at: { address, block } })
  strictEqual(parseInt(xMarsBalance.balance), expectedBalance)
}

async function assertXmarsTotalSupplyAt(terra: LCDClient, xMars: string, block: number, expectedTotalSupply: number) {
  const xMarsTotalSupply = await queryContract(terra, xMars, { total_supply_at: { block } })
  strictEqual(parseInt(xMarsTotalSupply.total_supply), expectedTotalSupply)
}

async function queryNativeBalance(terra: LCDClient, address: string, denom: string) {
  const balances = await terra.bank.balance(address)
  const balance = balances.get(denom)
  if (balance === undefined) {
    return 0
  }
  return balance.amount.toNumber()
}

async function queryCw20Balance(terra: LCDClient, userAddress: string, contractAddress: string) {
  const result = await queryContract(terra, contractAddress, { balance: { address: userAddress } })
  return parseInt(result.balance)
}

async function getBlockHeight(terra: LCDClient, txResult: BlockTxBroadcastResult) {
  await sleep(100)
  const txInfo = await terra.tx.txInfo(txResult.txhash)
  return txInfo.height
}

// MAIN

async function main() {
  // SETUP

  setTimeoutDuration(0)

  const terra = new LocalTerra()

  // addresses
  const deployer = terra.wallets.test1
  const alice = terra.wallets.test2
  const bob = terra.wallets.test3
  const carol = terra.wallets.test4

  console.log("upload contracts")

  const addressProvider = await deployContract(terra, deployer, "../artifacts/address_provider.wasm",
    { owner: deployer.key.accAddress }
  )

  const tokenCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_token.wasm"))
  const pairCodeID = await uploadContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_pair.wasm"))
  const terraswapFactory = await deployContract(terra, deployer, join(TERRASWAP_ARTIFACTS_PATH, "terraswap_factory.wasm"),
    {
      pair_code_id: pairCodeID,
      token_code_id: tokenCodeID
    }
  )

  const staking = await deployContract(terra, deployer, "../artifacts/staking.wasm",
    {
      config: {
        owner: deployer.key.accAddress,
        address_provider_address: addressProvider,
        terraswap_factory_address: terraswapFactory,
        terraswap_max_spread: "0.05",
        cooldown_duration: COOLDOWN_DURATION_SECONDS,
        unstake_window: UNSTAKE_WINDOW_DURATION_SECONDS,
      }
    }
  )

  const mars = await deployContract(terra, deployer, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"),
    {
      name: "Mars",
      symbol: "MARS",
      decimals: 6,
      initial_balances: [],
      mint: { minter: deployer.key.accAddress },
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
          mars_token_address: mars,
          staking_address: staking,
          xmars_token_address: xMars,
          protocol_admin_address: deployer.key.accAddress,
        }
      }
    }
  )

  // terraswap pairs

  let result = await executeContract(terra, deployer, terraswapFactory,
    {
      create_pair: {
        asset_infos: [
          { token: { contract_addr: mars } },
          { native_token: { denom: "uusd" } }
        ]
      }
    }
  )
  const marsUusdPair = result.logs[0].eventsByType.wasm.pair_contract_addr[0]

  result = await executeContract(terra, deployer, terraswapFactory,
    {
      create_pair: {
        asset_infos: [
          { native_token: { denom: "uluna" } },
          { native_token: { denom: "uusd" } }
        ]
      }
    }
  )
  const ulunaUusdPair = result.logs[0].eventsByType.wasm.pair_contract_addr[0]

  await executeContract(terra, deployer, ulunaUusdPair,
    {
      provide_liquidity: {
        assets: [
          {
            info: { native_token: { denom: "uluna" } },
            amount: String(ULUNA_UUSD_PAIR_ULUNA_LP_AMOUNT)
          }, {
            info: { native_token: { denom: "uusd" } },
            amount: String(ULUNA_UUSD_PAIR_UUSD_LP_AMOUNT)
          }
        ]
      }
    },
    `${ULUNA_UUSD_PAIR_ULUNA_LP_AMOUNT}uluna,${ULUNA_UUSD_PAIR_UUSD_LP_AMOUNT}uusd`,
  )

  await executeContract(terra, deployer, mars,
    {
      mint: {
        recipient: deployer.key.accAddress,
        amount: String(MARS_UUSD_PAIR_MARS_LP_AMOUNT)
      }
    }
  )

  await executeContract(terra, deployer, mars,
    {
      increase_allowance: {
        spender: marsUusdPair,
        amount: String(MARS_UUSD_PAIR_MARS_LP_AMOUNT),
      }
    }
  )

  await executeContract(terra, deployer, marsUusdPair,
    {
      provide_liquidity: {
        assets: [
          {
            info: { token: { contract_addr: mars } },
            amount: String(MARS_UUSD_PAIR_MARS_LP_AMOUNT)
          }, {
            info: { native_token: { denom: "uusd" } },
            amount: String(MARS_UUSD_PAIR_UUSD_LP_AMOUNT)
          }
        ]
      }
    },
    `${MARS_UUSD_PAIR_UUSD_LP_AMOUNT}uusd`,
  )


  // TESTS

  let xMarsTotalSupply = 0

  {
    console.log("alice stakes Mars and receives the same amount of xMars")

    await executeContract(terra, deployer, mars,
      {
        mint: {
          recipient: alice.key.accAddress,
          amount: String(MARS_STAKE_AMOUNT)
        }
      }
    )

    const txResult = await executeContract(terra, alice, mars,
      {
        send: {
          contract: staking,
          amount: String(MARS_STAKE_AMOUNT),
          msg: toEncodedBinary({ stake: {} })
        }
      }
    )
    const block = await getBlockHeight(terra, txResult)

    // before staking
    await assertXmarsBalanceAt(terra, xMars, alice.key.accAddress, block - 1, 0)
    await assertXmarsTotalSupplyAt(terra, xMars, block - 1, xMarsTotalSupply)

    // after staking
    xMarsTotalSupply += MARS_STAKE_AMOUNT
    await assertXmarsBalance(terra, xMars, alice.key.accAddress, MARS_STAKE_AMOUNT)
    await assertXmarsBalanceAt(terra, xMars, alice.key.accAddress, block + 1, MARS_STAKE_AMOUNT)
    await assertXmarsTotalSupplyAt(terra, xMars, block + 1, xMarsTotalSupply)
  }

  {
    console.log("bob stakes Mars and receives the same amount of xMars")

    await executeContract(terra, deployer, mars,
      {
        mint: {
          recipient: bob.key.accAddress,
          amount: String(MARS_STAKE_AMOUNT)
        }
      }
    )

    const txResult = await executeContract(terra, bob, mars,
      {
        send: {
          contract: staking,
          amount: String(MARS_STAKE_AMOUNT),
          msg: toEncodedBinary({ stake: {} })
        }
      }
    )
    const block = await getBlockHeight(terra, txResult)

    // before staking
    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block - 1, 0)
    await assertXmarsTotalSupplyAt(terra, xMars, block - 1, xMarsTotalSupply)

    // after staking
    xMarsTotalSupply += MARS_STAKE_AMOUNT
    await assertXmarsBalance(terra, xMars, bob.key.accAddress, MARS_STAKE_AMOUNT)
    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block + 1, MARS_STAKE_AMOUNT)
    await assertXmarsTotalSupplyAt(terra, xMars, block + 1, xMarsTotalSupply)
  }

  {
    console.log("bob transfers half of his xMars to alice")

    const txResult = await executeContract(terra, bob, xMars,
      {
        transfer: {
          recipient: alice.key.accAddress,
          amount: String(MARS_STAKE_AMOUNT / 2)
        }
      }
    )
    const block = await getBlockHeight(terra, txResult)

    // before staking
    await assertXmarsBalanceAt(terra, xMars, alice.key.accAddress, block - 1, MARS_STAKE_AMOUNT)
    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block - 1, MARS_STAKE_AMOUNT)
    await assertXmarsTotalSupplyAt(terra, xMars, block - 1, xMarsTotalSupply)

    // after staking
    await assertXmarsBalance(terra, xMars, alice.key.accAddress, 3 * MARS_STAKE_AMOUNT / 2)
    await assertXmarsBalance(terra, xMars, bob.key.accAddress, MARS_STAKE_AMOUNT / 2)
    await assertXmarsBalanceAt(terra, xMars, alice.key.accAddress, block + 1, 3 * MARS_STAKE_AMOUNT / 2)
    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block + 1, MARS_STAKE_AMOUNT / 2)
    await assertXmarsTotalSupplyAt(terra, xMars, block + 1, xMarsTotalSupply)
  }

  {
    console.log("swap protocol rewards to USD, then USD to Mars")

    // send luna to the staking contract to simulate rewards accrued to stakers from activity on the
    // protocol
    await performTransaction(terra, deployer,
      new MsgSend(deployer.key.accAddress, staking, { uluna: ULUNA_SWAP_AMOUNT })
    )

    // swap luna to usd
    const uusdBalanceBeforeSwapToUusd = await queryNativeBalance(terra, staking, "uusd")

    await executeContract(terra, deployer, staking,
      {
        swap_asset_to_uusd: {
          offer_asset_info: { native_token: { denom: "uluna" } },
          amount: String(ULUNA_SWAP_AMOUNT)
        }
      }
    )

    const ulunaBalanceAfterSwapToUusd = await queryNativeBalance(terra, staking, "uluna")
    const uusdBalanceAfterSwapToUusd = await queryNativeBalance(terra, staking, "uusd")

    strictEqual(ulunaBalanceAfterSwapToUusd, 0)
    assert(uusdBalanceAfterSwapToUusd > uusdBalanceBeforeSwapToUusd)

    // swap usd to mars
    const uusdBalanceBeforeSwapToMars = await queryNativeBalance(terra, staking, "uusd")
    const marsBalanceBeforeSwapToMars = await queryCw20Balance(terra, staking, mars)

    // don't swap the entire uusd balance, otherwise there won't be enough to pay the tx fee
    const uusdSwapAmount = uusdBalanceAfterSwapToUusd - 10_000000

    await executeContract(terra, deployer, staking,
      {
        swap_uusd_to_mars: {
          amount: String(uusdSwapAmount)
        }
      }
    )

    const marsBalanceAfterSwapToMars = await queryCw20Balance(terra, staking, mars)
    const uusdBalanceAfterSwapToMars = await queryNativeBalance(terra, staking, "uusd")

    assert(uusdBalanceAfterSwapToMars < uusdBalanceBeforeSwapToMars)
    assert(marsBalanceAfterSwapToMars > marsBalanceBeforeSwapToMars)
  }

  {
    console.log("carol stakes Mars and receives a smaller amount of xMars")

    await executeContract(terra, deployer, mars,
      {
        mint: {
          recipient: carol.key.accAddress,
          amount: String(MARS_STAKE_AMOUNT)
        }
      }
    )

    const txResult = await executeContract(terra, carol, mars,
      {
        send: {
          contract: staking,
          amount: String(MARS_STAKE_AMOUNT),
          msg: toEncodedBinary({ stake: {} })
        }
      }
    )
    const block = await getBlockHeight(terra, txResult)

    // before staking
    await assertXmarsBalanceAt(terra, xMars, carol.key.accAddress, block - 1, 0)
    await assertXmarsTotalSupplyAt(terra, xMars, block - 1, xMarsTotalSupply)

    // after staking
    const carolXmarsBalance = await queryCw20Balance(terra, carol.key.accAddress, xMars)
    assert(carolXmarsBalance < MARS_STAKE_AMOUNT)
    xMarsTotalSupply += carolXmarsBalance
    await assertXmarsBalanceAt(terra, xMars, carol.key.accAddress, block + 1, carolXmarsBalance)
    await assertXmarsTotalSupplyAt(terra, xMars, block + 1, xMarsTotalSupply)
  }

  {
    console.log("bob unstakes xMars")

    let bobXmarsBalance = await queryCw20Balance(terra, bob.key.accAddress, xMars)
    const cooldownAmount = bobXmarsBalance

    await assert.rejects(
      executeContract(terra, bob, xMars,
        {
          send: {
            contract: staking,
            amount: String(cooldownAmount),
            msg: toEncodedBinary({ unstake: {} })
          }
        }
      ),
      (error: any) => {
        return error.response.data.error.includes("Address must have a valid cooldown to unstake")
      }
    )

    console.log("- activates cooldown")

    await executeContract(terra, bob, staking, { cooldown: {} })

    const cooldownStart = Date.now()
    const cooldownEnd = cooldownStart + COOLDOWN_DURATION_SECONDS * 1000 // ms

    let cooldown = await queryContract(terra, staking, { cooldown: { user_address: bob.key.accAddress } })
    strictEqual(parseInt(cooldown.amount), cooldownAmount)

    console.log("- unstaking before cooldown has ended fails")

    await assert.rejects(
      executeContract(terra, bob, xMars,
        {
          send: {
            contract: staking,
            amount: String(cooldownAmount),
            msg: toEncodedBinary({ unstake: {} })
          }
        }
      ),
      (error: any) => {
        return error.response.data.error.includes("Cooldown has not finished")
      }
    )

    console.log("- alice transfers some xMars to bob")

    await executeContract(terra, alice, xMars,
      {
        transfer: {
          recipient: bob.key.accAddress,
          amount: String(MARS_STAKE_AMOUNT / 2)
        }
      }
    )

    cooldown = await queryContract(terra, staking, { cooldown: { user_address: bob.key.accAddress } })
    strictEqual(parseInt(cooldown.amount), cooldownAmount)

    console.log("- bob tries to unstake all xMars")

    bobXmarsBalance = await queryCw20Balance(terra, bob.key.accAddress, xMars)

    await assert.rejects(
      executeContract(terra, bob, xMars,
        {
          send: {
            contract: staking,
            amount: String(bobXmarsBalance),
            msg: toEncodedBinary({ unstake: {} })
          }
        }
      ),
      (error: any) => {
        return error.response.data.error.includes("Unstake amount must not be greater than cooldown amount")
      }
    )

    console.log("- bob unstakes the amount of xMars he requested in the cooldown")

    const cooldownRemaining = Math.max(cooldownEnd - Date.now(), 0)
    await sleep(
      cooldownRemaining
      // account for LocalTerra's clock running at 1/t the speed of realworld time,
      // where t is the targetted block time in seconds
      * BLOCK_TIME_SECONDS
    )

    const txResult = await executeContract(terra, bob, xMars,
      {
        send: {
          contract: staking,
          amount: String(cooldownAmount),
          msg: toEncodedBinary({ unstake: {} })
        }
      }
    )
    const block = await getBlockHeight(terra, txResult)

    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block - 1, MARS_STAKE_AMOUNT)
    await assertXmarsTotalSupplyAt(terra, xMars, block - 1, xMarsTotalSupply)

    await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, block + 1, MARS_STAKE_AMOUNT / 2)
    await assertXmarsTotalSupplyAt(terra, xMars, block + 1, xMarsTotalSupply - MARS_STAKE_AMOUNT / 2)
  }

  {
    console.log("alice unstakes xMars")

    console.log("- activates cooldown")

    await executeContract(terra, alice, staking, { cooldown: {} })

    console.log("- tries to unstake after the unstake window has ended")

    await sleep(
      ((
        // wait until the unstake window ends
        COOLDOWN_DURATION_SECONDS + UNSTAKE_WINDOW_DURATION_SECONDS
        // then a bit more to ensure the unstake window has ended
        + 1
      ) * 1000)
      // account for LocalTerra's clock running at 1/t the speed of realworld time,
      // where t is the targetted block time in seconds
      * BLOCK_TIME_SECONDS
    )

    const aliceXmarsBalance = await queryCw20Balance(terra, alice.key.accAddress, xMars)

    await assert.rejects(
      executeContract(terra, alice, xMars,
        {
          send: {
            contract: staking,
            amount: String(aliceXmarsBalance),
            msg: toEncodedBinary({ unstake: {} })
          }
        }
      ),
      (error: any) => {
        return error.rawLog.includes("Cooldown has expired")
      }
    )
  }

  console.log("OK")
}

main().catch(err => console.log(err))
