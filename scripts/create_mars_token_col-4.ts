/*
Script to deploy a cw20 token on Terra Columbus-4.

Dependencies:
  - cw-plus v0.2.0
  - Set environment variables in a .env file (see below for details of the required variables)

Dependencies to run on LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
*/

import { LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import 'dotenv/config.js'
import { executeContract, instantiateContract, queryContract, recover, setTimeoutDuration, uploadContract } from "./helpers.js"

// Required environment variables:


// Testnet:
const CHAIN_ID = process.env.CHAIN_ID
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL
const WALLET = process.env.WALLET
const CW20_CODE_ID = process.env.CW20_CODE_ID

// LocalTerra:
const CW20_BINARY_PATH = process.env.CW20_BINARY_PATH

// Main

async function main() {
  const isTestnet = CHAIN_ID !== undefined

  let terra: LCDClient | LocalTerra
  let wallet: Wallet
  let cw20CodeId: number

  if (isTestnet) {
    terra = new LCDClient({
      URL: LCD_CLIENT_URL!,
      chainID: CHAIN_ID!
    })

    wallet = recover(terra, WALLET!)

    cw20CodeId = parseInt(CW20_CODE_ID!)

  } else {
    setTimeoutDuration(0)

    terra = new LocalTerra()

    wallet = (terra as LocalTerra).wallets.test1

    // Upload contract code
    cw20CodeId = await uploadContract(terra, wallet, CW20_BINARY_PATH!)
    console.log(cw20CodeId)
  }

  // Token info
  const TOKEN_NAME = "Mars"
  const TOKEN_SYMBOL = "MARS"
  const TOKEN_DECIMALS = 6
  const TOKEN_MINTER = wallet.key.accAddress
  const TOKEN_CAP = 1_000_000_000_000000

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

  // Instantiate Mars token contract
  const marsAddress = await instantiateContract(terra, wallet, cw20CodeId, TOKEN_INFO)
  console.log("mars:", marsAddress)
  console.log(await queryContract(terra, marsAddress, { token_info: {} }))
  console.log(await queryContract(terra, marsAddress, { minter: {} }))

  // Try minting
  await executeContract(terra, wallet, marsAddress,
    {
      mint: {
        recipient: wallet.key.accAddress,
        amount: String(1_000_000000)
      }
    }
  )

  console.log(await queryContract(terra, marsAddress, { balance: { address: wallet.key.accAddress } }))

  console.log("OK")
}

main().catch(err => console.log(err))
