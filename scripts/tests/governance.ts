import { BlockTxBroadcastResult, LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import { join } from "path"
import { strictEqual } from "assert"
import fetch from "node-fetch"
import 'dotenv/config.js'
import {
  deployContract,
  executeContract,
  queryContract,
  setTimeoutDuration,
  sleep,
  toEncodedBinary,
  uploadContract
} from "../helpers.js"

// CONSTS

// required environment variables:
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!

// there is no way to query the latest block height in terra.js, so we query the LocalTerra LCD
// endpoint instead
const LOCAL_TERRA_LCD_URL = "http://localhost:1317"

const PROPOSAL_EFFECTIVE_DELAY = 5
const PROPOSAL_REQUIRED_DEPOSIT = 100_000000
const PROPOSAL_VOTING_PERIOD = 10
// require almost all of the xMars voting power to vote, in order to test that xMars balances at the
// block before proposals were submitted are used
const PROPOSAL_REQUIRED_QUORUM = 0.99

const ALICE_XMARS_BALANCE = 2_000_000000
const ALICE_PROPOSAL_DEPOSIT = PROPOSAL_REQUIRED_DEPOSIT
const BOB_XMARS_BALANCE = 1_000_000000
const BOB_PROPOSAL_DEPOSIT = PROPOSAL_REQUIRED_DEPOSIT + 5_000000

const LUNA_USD_PRICE = 25

// HELPERS

async function getBlockHeight(terra: LCDClient, txResult: BlockTxBroadcastResult) {
  await sleep(100)
  const txInfo = await terra.tx.txInfo(txResult.txhash)
  return txInfo.height
}

async function getLatestBlockHeight() {
  const response = await fetch(`${LOCAL_TERRA_LCD_URL}/blocks/latest`)
  const obj: any = await response.json()
  return parseInt(obj.block.header.height)
}

async function assertXmarsBalanceAt(terra: LCDClient, xMars: string, address: string, block: number, expectedBalance: number) {
  const xMarsBalance = await queryContract(terra, xMars, { balance_at: { address, block } })
  strictEqual(parseInt(xMarsBalance.balance), expectedBalance)
}

async function queryCw20Balance(terra: LCDClient, userAddress: string, contractAddress: string) {
  const result = await queryContract(terra, contractAddress, { balance: { address: userAddress } })
  return parseInt(result.balance)
}

async function mintCw20(terra: LCDClient, wallet: Wallet, contract: string, recipient: string, amount: number) {
  return await executeContract(terra, wallet, contract,
    {
      mint: {
        recipient,
        amount: String(amount)
      }
    }
  )
}

async function castVote(terra: LCDClient, wallet: Wallet, council: string, proposalId: number, vote: string) {
  return await executeContract(terra, wallet, council,
    {
      cast_vote: {
        proposal_id: proposalId,
        vote
      }
    }
  )
}

async function waitUntilBlockHeight(blockHeight: number) {
  const maxTries = 10
  let tries = 0
  let backoff = 1
  while (true) {
    const latestBlockHeight = await getLatestBlockHeight()
    if (latestBlockHeight >= blockHeight) {
      break
    }

    // timeout
    tries++
    if (tries == maxTries) {
      throw new Error(
        `timed out waiting for block height ${blockHeight}, current block height: ${latestBlockHeight}`
      )
    }

    // exponential backoff
    await sleep(backoff * 1000)
    backoff *= 2
  }
}

// MAIN

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()

  // addresses
  const deployer = terra.wallets.test1
  const alice = terra.wallets.test2
  const bob = terra.wallets.test3
  const carol = terra.wallets.test4
  // mock contract address
  const incentives = terra.wallets.test9.key.accAddress
  const staking = terra.wallets.test10.key.accAddress

  console.log("upload contracts")

  const addressProvider = await deployContract(terra, deployer, "../artifacts/address_provider.wasm",
    { owner: deployer.key.accAddress }
  )

  const council = await deployContract(terra, deployer, "../artifacts/council.wasm",
    {
      config: {
        address_provider_address: addressProvider,
        proposal_voting_period: PROPOSAL_VOTING_PERIOD,
        proposal_effective_delay: PROPOSAL_EFFECTIVE_DELAY,
        proposal_expiration_period: 3000,
        proposal_required_deposit: String(PROPOSAL_REQUIRED_DEPOSIT),
        proposal_required_quorum: String(PROPOSAL_REQUIRED_QUORUM),
        proposal_required_threshold: "0.05"
      }
    }
  )

  const oracle = await deployContract(terra, deployer, "../artifacts/oracle.wasm",
    {
      owner: council
    }
  )

  const maTokenCodeId = await uploadContract(terra, deployer, "../artifacts/ma_token.wasm")

  const redBank = await deployContract(terra, deployer, "../artifacts/red_bank.wasm",
    {
      config: {
        owner: council,
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
      mint: { minter: deployer.key.accAddress },
    }
  )

  // update address provider
  await executeContract(terra, deployer, addressProvider,
    {
      update_config: {
        config: {
          owner: deployer.key.accAddress,
          council_address: council,
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

  // mint tokens
  await mintCw20(terra, deployer, mars, alice.key.accAddress, ALICE_PROPOSAL_DEPOSIT)
  await mintCw20(terra, deployer, mars, bob.key.accAddress, BOB_PROPOSAL_DEPOSIT)
  await mintCw20(terra, deployer, xMars, alice.key.accAddress, ALICE_XMARS_BALANCE)
  await mintCw20(terra, deployer, xMars, bob.key.accAddress, BOB_XMARS_BALANCE)

  // TESTS

  console.log("alice submits a proposal to initialise a new asset in the red bank")

  let txResult = await executeContract(terra, alice, mars,
    {
      send: {
        contract: council,
        amount: String(ALICE_PROPOSAL_DEPOSIT),
        msg: toEncodedBinary({
          submit_proposal: {
            title: "Init Luna",
            description: "Initialise Luna",
            link: "http://www.terra.money",
            execute_calls: [
              // init luna as an asset in the red bank contract
              {
                execution_order: 1,
                target_contract_address: redBank,
                msg: toEncodedBinary({
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
                      }
                    }
                  }
                })
              },
              // set a fixed price for luna in the oracle contract
              {
                execution_order: 2,
                target_contract_address: oracle,
                msg: toEncodedBinary({
                  set_asset: {
                    asset: { native: { denom: "uluna" } },
                    price_source: { fixed: { price: String(LUNA_USD_PRICE) } }
                  }
                })
              }
            ]
          }
        })
      }
    }
  )
  let blockHeight = await getBlockHeight(terra, txResult)
  const aliceProposalVotingPeriodEnd = blockHeight + PROPOSAL_VOTING_PERIOD
  const aliceProposalEffectiveDelayEnd = aliceProposalVotingPeriodEnd + PROPOSAL_EFFECTIVE_DELAY
  const aliceProposalId = parseInt(txResult.logs[0].eventsByType.wasm.proposal_id[0])

  console.log("bob submits a proposal that will fail")

  txResult = await executeContract(terra, bob, mars,
    {
      send: {
        contract: council,
        amount: String(BOB_PROPOSAL_DEPOSIT),
        msg: toEncodedBinary({
          submit_proposal: {
            title: "Null",
            description: "An empty proposal",
            execute_calls: []
          }
        })
      }
    }
  )
  blockHeight = await getBlockHeight(terra, txResult)
  const bobProposalVotingPeriodEnd = blockHeight + PROPOSAL_VOTING_PERIOD
  const bobProposalId = parseInt(txResult.logs[0].eventsByType.wasm.proposal_id[0])

  console.log("alice sends entire xMars balance to bob")

  await executeContract(terra, alice, xMars,
    {
      transfer: {
        recipient: bob.key.accAddress,
        amount: String(ALICE_XMARS_BALANCE)
      }
    }
  )

  await assertXmarsBalanceAt(terra, xMars, alice.key.accAddress, blockHeight - 1, ALICE_XMARS_BALANCE)
  await assertXmarsBalanceAt(terra, xMars, bob.key.accAddress, blockHeight - 1, BOB_XMARS_BALANCE)

  console.log("mint a large amount of xMars to carol")

  // proposal quorum should use xMars balances from the blockHeight before a proposal was submitted.
  // so, proposal quorum should still be reached by alice's and bob's votes, even after a large
  // amount of xMars is minted to carol.
  await mintCw20(terra, deployer, xMars, carol.key.accAddress, ALICE_XMARS_BALANCE * BOB_XMARS_BALANCE * 100)

  await assertXmarsBalanceAt(terra, xMars, carol.key.accAddress, blockHeight - 1, 0)

  console.log("vote")

  await castVote(terra, alice, council, aliceProposalId, "for")
  await castVote(terra, bob, council, aliceProposalId, "against")

  console.log("wait for voting periods to end")

  await waitUntilBlockHeight(Math.max(aliceProposalVotingPeriodEnd, bobProposalVotingPeriodEnd))

  console.log("end proposals")

  console.log("- alice's proposal passes, so her Mars deposit is returned")

  const aliceMarsBalanceBefore = await queryCw20Balance(terra, alice.key.accAddress, mars)

  await executeContract(terra, deployer, council, { end_proposal: { proposal_id: aliceProposalId } })

  const aliceProposalStatus = await queryContract(terra, council, { proposal: { proposal_id: aliceProposalId } })
  strictEqual(aliceProposalStatus.status, "passed")

  const aliceMarsBalanceAfter = await queryCw20Balance(terra, alice.key.accAddress, mars)
  strictEqual(aliceMarsBalanceAfter, aliceMarsBalanceBefore + ALICE_PROPOSAL_DEPOSIT)

  console.log("- bob's proposal was rejected, so his Mars deposit is sent to the staking contract")

  const bobMarsBalanceBefore = await queryCw20Balance(terra, bob.key.accAddress, mars)
  const stakingContractMarsBalanceBefore = await queryCw20Balance(terra, staking, mars)

  await executeContract(terra, deployer, council, { end_proposal: { proposal_id: bobProposalId } })

  const bobProposalStatus = await queryContract(terra, council, { proposal: { proposal_id: bobProposalId } })
  strictEqual(bobProposalStatus.status, "rejected")

  const bobMarsBalanceAfter = await queryCw20Balance(terra, bob.key.accAddress, mars)
  const stakingContractMarsBalanceAfter = await queryCw20Balance(terra, staking, mars)
  strictEqual(bobMarsBalanceAfter, bobMarsBalanceBefore)
  strictEqual(stakingContractMarsBalanceAfter, stakingContractMarsBalanceBefore + BOB_PROPOSAL_DEPOSIT)

  console.log("wait for effective delay period to end")

  await waitUntilBlockHeight(aliceProposalEffectiveDelayEnd)

  console.log("execute proposal")

  await executeContract(terra, deployer, council, { execute_proposal: { proposal_id: aliceProposalId } })

  // check that the asset has been initialised on the red bank
  const marketsList = await queryContract(terra, redBank, { markets_list: {} })
  strictEqual(marketsList.markets_list[0].denom, "uluna")

  // check that the asset has been initialised in the oracle contract
  const assetPriceConfig = await queryContract(terra, oracle,
    { asset_price_config: { asset: { native: { denom: "uluna" } } } }
  )
  strictEqual(parseInt(assetPriceConfig.price_source.fixed.price), LUNA_USD_PRICE)

  console.log("OK")
}

main().catch(err => console.log(err))
