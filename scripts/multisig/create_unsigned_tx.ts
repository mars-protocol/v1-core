import { LCDClient, MsgExecuteContract, Wallet } from "@terra-money/terra.js"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import { writeFileSync } from "fs"
import 'dotenv/config.js'
import { createTransaction } from "../helpers.js"

// Required environment variables:

// Terra network details
const CHAIN_ID = process.env.CHAIN_ID!
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL!

// Multisig details
// The address that the tx will be sent from
const MULTISIG_ADDRESS = process.env.MULTISIG_ADDRESS!
// The name of the multisig account in terracli
const MULTISIG_NAME = process.env.MULTISIG_NAME!

// Transaction details
// The address that the tx will be sent to
const CONTRACT_ADDRESS = process.env.CONTRACT_ADDRESS!
// A JSON object of the operation to be executed on the contract
const EXECUTE_MSG = JSON.parse(process.env.EXECUTE_MSG!)

// Main

async function main() {
  const terra = new LCDClient({
    URL: LCD_CLIENT_URL,
    chainID: CHAIN_ID
  })

  // Create an unsigned tx
  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  const msg = new MsgExecuteContract(MULTISIG_ADDRESS, CONTRACT_ADDRESS, EXECUTE_MSG)

  const tx = await createTransaction(terra, multisig, msg)

  // The unsigned tx file should be distributed to the multisig key holders
  const unsignedTx = "unsigned_tx.json"
  writeFileSync(unsignedTx, tx.toStdTx().toJSON())

  // Prints a command that should be run by the multisig key holders to generate signatures
  // TODO add Ledger support
  console.log(`
# Set \`from\` to your address that is a key to the multisig: ${MULTISIG_ADDRESS}
from=terra1...

terracli tx sign ${unsignedTx} \\
  --multisig ${MULTISIG_ADDRESS} \\
  --from \$from \\
  --chain-id ${tx.chain_id} \\
  --offline \\
  --account-number ${tx.account_number} \\
  --sequence ${tx.sequence} \\
  --output-document \${from}_sig.json
`)

  // Run `broadcast_tx.ts` to aggregate at least K of N signatures and broadcast the signed tx to the network
}

main().catch(err => console.log(err))
