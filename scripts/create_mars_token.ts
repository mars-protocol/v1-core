/*
Script to deploy a cw20 token from a multisig account, mint tokens, and migrate the contract.

This script is designed to work with Terra Columbus-4.

Dependencies:
  - rust
  - terracli 58602320d2907814cfccdf43e9679468bb4bd8d3
  - cosmwasm-plus v0.2.0
  - Add accounts and multisig to terracli
  - Set environment variables in a .env file (see below for details of the required variables)

Dependencies to run on LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
*/

import {
  isTxError,
  LCDClient,
  LocalTerra,
  MsgExecuteContract,
  MsgSend,
  MsgUpdateContractOwner,
  StdTx,
  Wallet
} from "@terra-money/terra.js"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import { strictEqual } from "assert"
import { execSync } from "child_process"
import { unlinkSync, writeFileSync } from "fs"
import 'dotenv/config.js'
import {
  createTransaction,
  instantiateContract,
  migrate,
  performTransaction,
  queryContract,
  recover,
  setTimeoutDuration,
  uploadContract
} from "./helpers.js"

// Required environment variables:

// All:
const MULTISIG_ADDRESS = process.env.MULTISIG_ADDRESS!
// Name of the multisig in terracli
const MULTISIG_NAME = process.env.MULTISIG_NAME!
// Names of the multisig keys in terracli
const MULTISIG_KEYS = process.env.MULTISIG_KEYS!.split(",")
const MULTISIG_THRESHOLD = parseInt(process.env.MULTISIG_THRESHOLD!)

// Testnet:
const CHAIN_ID = process.env.CHAIN_ID
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL
const CW20_CODE_ID = process.env.CW20_CODE_ID

// LocalTerra:
const CW20_BINARY_PATH = process.env.CW20_BINARY_PATH

// Main

async function main() {
  const isTestnet = CHAIN_ID !== undefined

  let terra: LCDClient | LocalTerra
  let wallet: Wallet
  let codeID: number

  if (isTestnet) {
    terra = new LCDClient({
      URL: LCD_CLIENT_URL!,
      chainID: CHAIN_ID!
    })

    wallet = recover(terra, process.env.WALLET!)

    codeID = parseInt(CW20_CODE_ID!)

  } else {
    setTimeoutDuration(0)

    terra = new LocalTerra()

    wallet = (terra as LocalTerra).wallets.test1

    // Upload contract code
    codeID = await uploadContract(terra, wallet, CW20_BINARY_PATH!)
    console.log(codeID)
  }

  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  // Token info
  const TOKEN_NAME = "Mars"
  const TOKEN_SYMBOL = "MARS"
  const TOKEN_DECIMALS = 6
  // The minter address cannot be changed after the contract is instantiated
  const TOKEN_MINTER = MULTISIG_ADDRESS
  // The cap cannot be changed after the contract is instantiated
  const TOKEN_CAP = 1_000_000_000_000000
  // TODO check if we want initial balances in prod
  const TOKEN_INITIAL_AMOUNT = 1_000_000_000000
  const TOKEN_INITIAL_AMOUNT_ADDRESS = TOKEN_MINTER

  const TOKEN_INFO = {
    name: TOKEN_NAME,
    symbol: TOKEN_SYMBOL,
    decimals: TOKEN_DECIMALS,
    initial_balances: [
      {
        address: TOKEN_INITIAL_AMOUNT_ADDRESS,
        amount: String(TOKEN_INITIAL_AMOUNT)
      }
    ],
    mint: {
      minter: TOKEN_MINTER,
      cap: String(TOKEN_CAP)
    }
  }

  // Instantiate contract
  const contractAddress = await instantiateContract(terra, wallet, codeID, TOKEN_INFO)
  console.log(contractAddress)
  console.log(await queryContract(terra, contractAddress, { token_info: {} }))
  console.log(await queryContract(terra, contractAddress, { minter: {} }))

  let balance = await queryContract(terra, contractAddress, { balance: { address: TOKEN_INFO.initial_balances[0].address } })
  strictEqual(balance.balance, TOKEN_INFO.initial_balances[0].amount)

  // Mint tokens
  const mintAmount = 1_000_000000
  const recipient = wallet.key.accAddress

  // Send coins to the multisig address. On testnet, use the faucet to initialise the multisig balance.
  if (!isTestnet) {
    await performTransaction(terra, wallet, new MsgSend(
      wallet.key.accAddress,
      MULTISIG_ADDRESS,
      { uluna: 1_000_000000, uusd: 1_000_000000 }
    ))
  }

  // Create an unsigned tx
  const mintMsg = { mint: { recipient: recipient, amount: String(mintAmount) } }
  const tx = await createTransaction(terra, multisig, new MsgExecuteContract(MULTISIG_ADDRESS, contractAddress, mintMsg))
  writeFileSync('unsigned_tx.json', tx.toStdTx().toJSON())

  // Create K of N signatures for the tx
  let fns: Array<string> = []
  for (const key of MULTISIG_KEYS.slice(0, MULTISIG_THRESHOLD)) {
    const cli = new CLIKey({ keyName: key, multisig: MULTISIG_ADDRESS })
    const sig = await cli.createSignature(tx)

    const fn = `${key}_sig.json`
    writeFileSync(fn, sig.toJSON())
    fns.push(fn)
  }

  // Create a signed tx by aggregating the K signatures
  const signedTxData = execSync(
    `terracli tx multisign unsigned_tx.json ${MULTISIG_NAME} ${fns.join(" ")} ` +
    `--offline ` +
    `--chain-id ${tx.chain_id} --account-number ${tx.account_number} --sequence ${tx.sequence} `,
    { encoding: 'utf-8' }
  )

  // Broadcast the tx
  const signedTx = StdTx.fromData(JSON.parse(signedTxData.toString()))
  const result = await terra.tx.broadcast(signedTx);
  if (isTxError(result)) {
    throw new Error(
      `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }

  const tokenInfo = await queryContract(terra, contractAddress, { token_info: {} })
  console.log(tokenInfo)
  strictEqual(tokenInfo.total_supply, String(TOKEN_INITIAL_AMOUNT + mintAmount))

  balance = await queryContract(terra, contractAddress, { balance: { address: recipient } })
  console.log(balance)
  strictEqual(balance.balance, String(mintAmount))

  // Remove tmp files
  for (const fn of [...fns, "unsigned_tx.json"]) {
    unlinkSync(fn)
  }

  // Update contract owner
  const newOwner = MULTISIG_ADDRESS

  await performTransaction(terra, wallet, new MsgUpdateContractOwner(wallet.key.accAddress, newOwner, contractAddress))

  const contractInfo = await terra.wasm.contractInfo(contractAddress)
  strictEqual(contractInfo.owner, newOwner)

  // Migrate contract version
  try {
    await migrate(terra, multisig, contractAddress, codeID)
  } catch (err) {
    // Contracts cannot be migrated to the same contract version, so we catch this error.
    // If we get this error, then the wallet has permissions to migrate the contract.
    const errMsg = "migrate wasm contract failed: generic: Unknown version 0.2.0"
    if (!(err.message.includes(errMsg) || err.response.data.error.includes(errMsg))) {
      throw err
    }
  }

  console.log("OK")
}

main().catch(err => console.log(err))
