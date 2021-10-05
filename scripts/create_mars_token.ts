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
  LCDClient,
  LegacyAminoMultisigPublicKey,
  LocalTerra,
  MnemonicKey,
  MsgExecuteContract,
  MsgSend,
  MsgUpdateContractAdmin,
  SignDoc,
  SimplePublicKey,
  Wallet
} from "@terra-money/terra.js"
import { MultiSignature } from '@terra-money/terra.js/dist/core/MultiSignature.js';
import { SignatureV2 } from '@terra-money/terra.js/dist/core/SignatureV2.js';
import { strictEqual } from "assert"
import 'dotenv/config.js'
import { join } from "path"
import {
  broadcastTransaction,
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  recover,
  setTimeoutDuration,
  toEncodedBinary,
  uploadContract
} from "./helpers.js"

// CONSTS

// Required environment variables:
const CW_PLUS_ARTIFACTS_PATH = process.env.CW_PLUS_ARTIFACTS_PATH!

// For networks other than LocalTerra:
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
    console.log("wallet", wallet.key.accAddress)
  }

  // Multisig
  // NOTE this is for testnet use only -- do not mint tokens like this on mainnet
  // For mainnet, load the multisigPubKey like this:
  // const multisigPubKey = new LegacyAminoMultisigPublicKey(n, [
  //   new SimplePublicKey("PUBKEY"),
  //   new SimplePublicKey("PUBKEY"),
  //   ...
  // ])
  const mk1 = new MnemonicKey({
    mnemonic:
      "notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius"
  })
  const mk2 = new MnemonicKey({
    mnemonic:
      "quality vacuum heart guard buzz spike sight swarm shove special gym robust assume sudden deposit grid alcohol choice devote leader tilt noodle tide penalty"
  })

  const multisigPubKey = new LegacyAminoMultisigPublicKey(2, [
    mk1.publicKey as SimplePublicKey,
    mk2.publicKey as SimplePublicKey,
  ])
  const multisigAddress = multisigPubKey.address()
  const multisig = new MultiSignature(multisigPubKey)

  // Instantiate the token minter proxy contract
  const cw1WhitelistCodeId = await uploadContract(terra, wallet, join(CW_PLUS_ARTIFACTS_PATH, "cw1_whitelist.wasm"))

  console.log("cw1 whitelist code ID:", cw1WhitelistCodeId)

  const proxyAddress = await instantiateContract(terra, wallet, cw1WhitelistCodeId,
    {
      mutable: true,
      admins: [
        wallet.key.accAddress,
        multisigAddress
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
    new MsgUpdateContractAdmin(wallet.key.accAddress, multisigAddress, marsAddress)
  )

  strictEqual((await terra.wasm.contractInfo(marsAddress)).admin, multisigAddress)

  // Remove wallet from mars-minter admins
  await executeContract(terra, wallet, proxyAddress, { update_admins: { admins: [multisigAddress] } })

  console.log(await queryContract(terra, proxyAddress, { admin_list: {} }))

  // Mint tokens
  // NOTE this is for testnet use only -- do not mint tokens like this on mainnet
  const mintAmount = 1_000_000000
  const recipient = wallet.key.accAddress

  // Send coins to the multisig address. On testnet, use the faucet to initialise the multisig balance.
  if (isLocalTerra) {
    await performTransaction(terra, wallet, new MsgSend(
      wallet.key.accAddress,
      multisigAddress,
      {
        uluna: 1_000_000000,
        uusd: 1_000_000000
      }
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

  const accInfo = await terra.auth.accountInfo(multisigAddress)

  const tx = await terra.tx.create(
    [
      {
        address: multisigAddress,
        sequenceNumber: accInfo.getSequenceNumber(),
        publicKey: accInfo.getPublicKey(),
      },
    ],
    {
      msgs: [
        new MsgExecuteContract(
          multisigAddress,
          proxyAddress,
          proxyExecuteMsg
        )
      ]
    }
  )

  const signDoc = new SignDoc(
    terra.config.chainID,
    accInfo.getAccountNumber(),
    accInfo.getSequenceNumber(),
    tx.auth_info,
    tx.body
  )

  // Create K of N signatures for the tx
  const sig1 = await mk1.createSignatureAmino(signDoc)
  const sig2 = await mk2.createSignatureAmino(signDoc)

  multisig.appendSignatureV2s([sig1, sig2])

  // Create a signed tx by aggregating the K signatures
  tx.appendSignatures([
    new SignatureV2(
      multisigPubKey,
      multisig.toSignatureDescriptor(),
      accInfo.getSequenceNumber()
    )
  ])

  // Broadcast the tx
  const result = await broadcastTransaction(terra, tx)
  console.log(result.txhash)

  // Test
  const tokenInfo = await queryContract(terra, marsAddress, { token_info: {} })
  console.log(tokenInfo)
  strictEqual(parseInt(tokenInfo.total_supply), mintAmount)

  console.log("OK")
})()
