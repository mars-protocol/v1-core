import { LCDClient, StdTx } from "@terra-money/terra.js"
import { execSync } from "child_process"
import { readFileSync } from "fs"
import 'dotenv/config.js'
import { broadcastTransaction } from "../helpers"

// Required environment variables:

// Terra network details
const CHAIN_ID = process.env.CHAIN_ID!
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL!

// Multisig details
const MULTISIG_NAME = process.env.MULTISIG_NAME!
// Returned by `create_unsigned_tx.ts` when the unsigned tx was generated
const ACCOUNT_NUMBER = process.env.ACCOUNT_NUMBER!
// Returned by `create_unsigned_tx.ts` when the unsigned tx was generated
const SEQUENCE = process.env.SEQUENCE!

// Signatures
const SIGNATURES = (process.env.SIGNATURES!).split(",")

// Main

async function main() {
  const terra = new LCDClient({
    URL: LCD_CLIENT_URL,
    chainID: CHAIN_ID
  })

  // Sign the tx using the signatures from the multisig key holders
  const signedTx = "signed_tx.json"
  execSync(`
    terracli tx multisign unsigned_tx.json ${MULTISIG_NAME} ${SIGNATURES.join(" ")} \
      --chain-id ${CHAIN_ID} \
      --offline \
      --account-number ${ACCOUNT_NUMBER} \
      --sequence ${SEQUENCE} \
      --output-document ${signedTx}
  `)
  const tx = StdTx.fromData(JSON.parse(readFileSync(signedTx).toString()))

  // Broadcast the tx
  const result = await broadcastTransaction(terra, tx)
  console.log(`https://finder.terra.money/${CHAIN_ID}/tx/${result.txhash}`)
}

main().catch(err => console.log(err))
