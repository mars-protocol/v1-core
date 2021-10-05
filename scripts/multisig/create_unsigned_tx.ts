import { LCDClient, MsgExecuteContract, Wallet } from "@terra-money/terra.js"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import { writeFileSync } from "fs"
import 'dotenv/config.js'
import { createTransaction } from "../helpers.js"

// Required environment variables:
// Terra network details:
const CHAIN_ID = process.env.CHAIN_ID!
const LCD_CLIENT_URL = process.env.LCD_CLIENT_URL!
// Multisig details:
// The name of the multisig account in terracli
const MULTISIG_NAME = process.env.MULTISIG_NAME!
// Transaction details:
// The address that the tx will be sent to
const CONTRACT_ADDRESS = process.env.CONTRACT_ADDRESS!
// A JSON object of the operation to be executed on the contract
const EXECUTE_MSG = JSON.parse(process.env.EXECUTE_MSG!);

// MAIN

(async () => {
  const terra = new LCDClient({
    URL: LCD_CLIENT_URL,
    chainID: CHAIN_ID
  })

  // Create an unsigned tx
  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  const multisigAddress = multisig.key.accAddress

  const tx = await createTransaction(multisig,
    new MsgExecuteContract(multisigAddress, CONTRACT_ADDRESS, EXECUTE_MSG)
  )

  const accInfo = await terra.auth.accountInfo(multisigAddress)

  // The unsigned tx file should be distributed to the multisig key holders
  const unsignedTx = "unsigned_tx.json"
  writeFileSync(unsignedTx, JSON.stringify(tx.toData()))

  // Prints a command that should be run by the multisig key holders to generate signatures
  // TODO add Ledger support
  console.log(`
# Set \`from\` to your address that is a key to the multisig: ${multisigAddress}

from=terra1...

terrad tx sign ${unsignedTx} \\
  --multisig ${multisigAddress} \\
  --from \$from \\
  --chain-id ${terra.config.chainID} \\
  --offline \\
  --account-number ${accInfo.getAccountNumber()} \\
  --sequence ${accInfo.getSequenceNumber()} \\
  --output-document \${from}_sig.json
`)

  // Run `broadcast_tx.ts` to aggregate at least K of N signatures and broadcast the signed tx to the network
})()
