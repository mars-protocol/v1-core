/*
Script to mint Mars tokens on Terra Columbus-4.

Dependencies:
  - Set environment variables in a .env file (see below for details of the required variables)

Dependencies to run on LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
*/

import { LCDClient, LocalTerra } from "@terra-money/terra.js"
import 'dotenv/config.js'
import { executeContract, queryContract, recover, setTimeoutDuration } from "./helpers.js"

// CONSTS

// Required environment variables:
const TOKEN_ADDRESS = process.env.TOKEN_ADDRESS!
const TOKEN_MINTER_MNEMONIC = process.env.TOKEN_MINTER_MNEMONIC!
const RECIPIENT_ADDRESS = process.env.RECIPIENT_ADDRESS!
const MINT_AMOUNT = process.env.MINT_AMOUNT!

// Testnet:
const CHAIN_ID = process.env.CHAIN_ID
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL

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

  const oldBalance = await queryContract(terra, TOKEN_ADDRESS,
    { balance: { address: RECIPIENT_ADDRESS } }
  )
  console.log("oldBalance:", oldBalance.balance)

  const result = await executeContract(terra, wallet, TOKEN_ADDRESS,
    {
      mint: {
        recipient: RECIPIENT_ADDRESS,
        amount: MINT_AMOUNT
      }
    }
  )

  const newBalance = await queryContract(terra, TOKEN_ADDRESS,
    { balance: { address: RECIPIENT_ADDRESS } }
  )
  console.log("newBalance:", newBalance.balance)

  if (!isLocalTerra) {
    console.log(`https://finder.terra.money/${CHAIN_ID}/tx/${result.txhash}`)
  }

  console.log("OK")
}

main().catch(err => console.log(err))
