/*
Script to deploy a cw20 token on Terra Columbus-4.

Dependencies:
  - cw-plus v0.2.0
  - Set environment variables in a .env file (see below for details of the required variables)

Dependencies to run on LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
*/

import { LCDClient, LocalTerra } from "@terra-money/terra.js"
import 'dotenv/config.js'
import { instantiateContract, recover, setTimeoutDuration, uploadContract } from "./helpers.js"

// CONSTS

// Required environment variables:
const TOKEN_MINTER_MNEMONIC = process.env.TOKEN_MINTER_MNEMONIC!

// Testnet:
const CHAIN_ID = process.env.CHAIN_ID
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL
const CW20_CODE_ID = process.env.CW20_CODE_ID

// LocalTerra:
const CW20_BINARY_PATH = process.env.CW20_BINARY_PATH

// MAIN

async function main() {
  const isLocalTerra = CHAIN_ID === undefined

  let terra: LCDClient | LocalTerra

  if (isLocalTerra) {
    setTimeoutDuration(0)

    terra = new LocalTerra()
  } else {
    terra = new LCDClient({
      URL: LCD_CLIENT_URL!,
      chainID: CHAIN_ID!
    })
  }

  const wallet = recover(terra, TOKEN_MINTER_MNEMONIC)

  let cw20CodeId: number

  if (isLocalTerra) {
    cw20CodeId = await uploadContract(terra, wallet, CW20_BINARY_PATH!)
  } else {
    cw20CodeId = parseInt(CW20_CODE_ID!)
  }

  const TOKEN_NAME = "Mars"
  const TOKEN_SYMBOL = "MARS"
  const TOKEN_DECIMALS = 6
  const TOKEN_MINTER = wallet.key.accAddress
  const TOKEN_CAP = 1_000_000_000_000000 // TODO check this

  const TOKEN_INFO = {
    name: TOKEN_NAME,
    symbol: TOKEN_SYMBOL,
    decimals: TOKEN_DECIMALS,
    initial_balances: [],
    mint: {
      minter: TOKEN_MINTER,
      cap: String(TOKEN_CAP)
    }
  }

  const tokenAddress = await instantiateContract(terra, wallet, cw20CodeId, TOKEN_INFO)
  console.log("token address", tokenAddress)

  if (!isLocalTerra) {
    console.log(`https://finder.terra.money/${CHAIN_ID}/address/${tokenAddress}`)
  }

  console.log("OK")
}

main().catch(err => console.log(err))
