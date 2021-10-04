/*
Script to deploy a cw20 token to Terra Columbus-5, setting the token minter and token contract owner
to a cw1 whitelist contract that has a multisig as the sole admin.

Dependencies:
  - terrad v0.5
  - cw-plus v0.8
  - LocalTerra (optional)
  - Add accounts and multisig to terrad
  - Set environment variables in a .env file (see below for details of the required variables)
*/

import {
  isTxError,
  LCDClient,
  LocalTerra,
  MsgExecuteContract,
  MsgSend,
  MsgUpdateContractAdmin,
  // StdTx,
  Wallet
} from "@terra-money/terra.js"
import { StdTx } from "@terra-money/terra.js/dist/core/StdTx"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import { strictEqual } from "assert"
import { execSync } from "child_process"
import {
  unlinkSync,
  writeFileSync
} from "fs"
import 'dotenv/config.js'
import { join } from "path"
import {
  broadcastTransaction,
  createTransaction,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  recover,
  setTimeoutDuration,
  toEncodedBinary,
  TransactionError,
  uploadContract
} from "./helpers.js"

// CONSTS

// Required environment variables:
const MULTISIG_ADDRESS = process.env.MULTISIG_ADDRESS!
// Name of the multisig in terracli
const MULTISIG_NAME = process.env.MULTISIG_NAME!
// Names of the multisig keys in terracli
const MULTISIG_KEYS = process.env.MULTISIG_KEYS!.split(",")
const MULTISIG_THRESHOLD = parseInt(process.env.MULTISIG_THRESHOLD!)
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!

const CHAIN_ID = process.env.CHAIN_ID
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL

// Token info
const TOKEN_NAME = "Mars"
const TOKEN_SYMBOL = "MARS"
const TOKEN_DECIMALS = 6
const TOKEN_CAP = 1_000_000_000_000000
const TOKEN_DESCRIPTION = "Mars is a fully automated, on-chain credit protocol built on Terra " +
  "and governed by a decentralised community of users and developers."
const TOKEN_PROJECT = "https://marsprotocol.io"
const TOKEN_LOGO = "https://marsprotocol.io/logo.png"; // TODO

// MAIN

(async () => {
  const isLocalTerra = CHAIN_ID == "localterra" || CHAIN_ID == undefined

  let terra: LCDClient
  let wallet: Wallet

  if (isLocalTerra) {
    setTimeoutDuration(0)

    terra = new LocalTerra()

    wallet = (terra as LocalTerra).wallets.test1
  } else {
    terra = new LCDClient({
      URL: LCD_CLIENT_URL!,
      chainID: CHAIN_ID!
    })

    wallet = recover(terra, process.env.WALLET!)
  }

  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  // TODO get multisig address from wallet instance

  // Instantiate the token minter proxy contract
  const cw1WhitelistCodeId = await uploadContract(terra, wallet, join(CW_PLUS_ARTIFACTS_PATH, "cw1_whitelist.wasm"))

  console.log("cw1 whitelist code ID:", cw1WhitelistCodeId)

  const proxyAddress = await instantiateContract(terra, wallet, cw1WhitelistCodeId,
    {
      mutable: true,
      admins: [
        wallet.key.accAddress,
        MULTISIG_ADDRESS
      ]
    }
  )

  console.log("proxy:", proxyAddress)
  console.log(await queryContract(terra, proxyAddress, { admin_list: {} }))

  // Instantiate Mars token contract
  const cw20CodeId = await uploadContract(terra, wallet, join(CW_PLUS_ARTIFACTS_PATH, "cw20_base.wasm"))

  console.log("cw20 code ID:", cw20CodeId)

  const marsAddress = await instantiateContract(terra, wallet, cw20CodeId,
    {
      name: TOKEN_NAME,
      symbol: TOKEN_SYMBOL,
      decimals: TOKEN_DECIMALS,
      initial_balances: [],
      mint: {
        minter: proxyAddress,
        cap: String(TOKEN_CAP)
      },
      marketing: {
        marketing: proxyAddress,
        description: TOKEN_DESCRIPTION,
        project: TOKEN_PROJECT,
        logo: { url: TOKEN_LOGO }
      }
    }
  )

  console.log("mars:", marsAddress)
  console.log(await queryContract(terra, marsAddress, { token_info: {} }))
  console.log(await queryContract(terra, marsAddress, { minter: {} }))
  console.log(await queryContract(terra, marsAddress, { marketing_info: {} }))

  // Set the proxy as the Mars token contract owner
  await performTransaction(terra, wallet,
    new MsgUpdateContractAdmin(wallet.key.accAddress, MULTISIG_ADDRESS, marsAddress)
  )

  strictEqual((await terra.wasm.contractInfo(marsAddress)).admin, MULTISIG_ADDRESS)

  // Remove wallet from mars-minter admins
  await executeContract(terra, wallet, proxyAddress, { update_admins: { admins: [MULTISIG_ADDRESS] } })

  console.log(await queryContract(terra, proxyAddress, { admin_list: {} }))

  // Mint tokens
  // NOTE this is for testnet use only -- do not mint tokens like this on mainnet
  const mintAmount = 1_000_000000
  const recipient = wallet.key.accAddress

  // Send coins to the multisig address. On testnet, use the faucet to initialise the multisig balance.
  if (isLocalTerra) {
    await performTransaction(terra, wallet, new MsgSend(
      wallet.key.accAddress,
      MULTISIG_ADDRESS,
      { uluna: 1_000_000000, uusd: 1_000_000000 }
    ))
  }

  // Create an unsigned tx
  const mintMsg = {
    mint: {
      recipient: recipient,
      amount: String(mintAmount)
    }
  }

  const proxyExecuteMsg = {
    execute: {
      msgs: [
        {
          wasm: {
            execute: {
              contract_addr: marsAddress,
              msg: toEncodedBinary(mintMsg),
              funds: []
            }
          }
        }
      ]
    }
  }

  const tx = await createTransaction(multisig,
    new MsgExecuteContract(MULTISIG_ADDRESS, proxyAddress, proxyExecuteMsg)
  )

  console.log(JSON.stringify(tx.toData()))
  // let d = tx.toData()
  // new StdTx(tx.body.messages, tx.auth_info.fee, d.signatures, d.body.memo, d.body.timeout_height)
  // writeFileSync('unsigned_tx.json', tx.toStdTx().toJSON())

  // // Create K of N signatures for the tx
  // let fileNames: Array<string> = []
  // for (const key of MULTISIG_KEYS.slice(0, MULTISIG_THRESHOLD)) {
  //   const cliKey = new CLIKey({ keyName: key, multisig: MULTISIG_ADDRESS })
  //   const signature = await cliKey.createSignature(tx)

  //   const fileName = `${key}_sig.json`
  //   writeFileSync(fileName, signature.toJSON())
  //   fileNames.push(fileName)
  // }

  // // Create a signed tx by aggregating the K signatures
  // const signedTxData = execSync(
  //   `terracli tx multisign unsigned_tx.json ${MULTISIG_NAME} ${fileNames.join(" ")} ` +
  //   `--offline ` +
  //   `--chain-id ${tx.chain_id} --account-number ${tx.account_number} --sequence ${tx.sequence} `,
  //   { encoding: 'utf-8' }
  // )

  // // Broadcast the tx
  // const signedTx = StdTx.fromData(JSON.parse(signedTxData.toString()))
  // const result = await broadcastTransaction(terra, signedTx)
  // if (isTxError(result)) {
  //   throw new TransactionError(result.code, result.codespace, result.raw_log)
  // }

  // const tokenInfo = await queryContract(terra, marsAddress, { token_info: {} })
  // console.log(tokenInfo)
  // strictEqual(parseInt(tokenInfo.total_supply), mintAmount)

  // const balance = await queryContract(terra, marsAddress, { balance: { address: recipient } })
  // console.log(balance)
  // strictEqual(parseInt(balance.balance), mintAmount)

  // // Remove tmp files
  // for (const fileName of [...fileNames, "unsigned_tx.json"]) {
  //   unlinkSync(fileName)
  // }

  console.log("OK")
})()
