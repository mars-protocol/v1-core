import { Wallet, StdFee, Coin, isTxError, MsgSend, LocalTerra, MsgUpdateContractOwner, StdTx, MsgExecuteContract } from "@terra-money/terra.js"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import {
  executeContract,
  instantiateContract,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  uploadContract,
  migrate
} from "./helpers.js"
import { strictEqual } from "assert"
import { execSync } from "child_process"
import { readFileSync, writeFileSync } from "fs"


const MULTISIG_ADDRESS = "terra1e0fx0q9meawrcq7fmma9x60gk35lpr4xk3884m"
const MULTISIG_NAME = "multi"

const TOKEN_NAME = "Mars"
const TOKEN_SYMBOL = "MARS"
const TOKEN_DECIMALS = 6
const TOKEN_MINTER = MULTISIG_ADDRESS
const TOKEN_CAP = 1_000_000_000_000000

const COSMWASM_PLUS_PATH = "../../cosmwasm-plus"

async function main() {
  setTimeoutDuration(10)

  const terra = new LocalTerra()
  const wallet = terra.wallets.test1

  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  // Compile contract
  // TODO use rust-optimizer
  execSync(
    `cd ${COSMWASM_PLUS_PATH}/contracts/cw20-base \
      && RUSTFLAGS='-C link-arg=-s' cargo wasm`,
    { encoding: 'utf-8' }
  )

  // Upload contract code
  const codeID = await uploadContract(terra, wallet, "../../cosmwasm-plus/target/wasm32-unknown-unknown/release/cw20_base.wasm")
  console.log(codeID)

  // Token info
  // tmp: should be TOKEN_MINTER in prod
  const minter = TOKEN_MINTER // wallet.key.accAddress

  // tmp: check if we want initial balances in prod
  const initialAmount = 1_000_000000
  const initialBalance = { "address": minter, "amount": String(initialAmount) }

  const TOKEN_INFO = {
    "name": TOKEN_NAME,
    "symbol": TOKEN_SYMBOL,
    "decimals": TOKEN_DECIMALS,
    "initial_balances": [initialBalance],
    "mint": {
      "minter": minter,
      "cap": String(TOKEN_CAP)
    }
  }

  // Instantiate contract
  const contractAddress = await instantiateContract(terra, wallet, codeID, TOKEN_INFO)
  console.log(contractAddress)
  console.log(await queryContract(terra, contractAddress, { "token_info": {} }))
  console.log(await queryContract(terra, contractAddress, { "minter": {} }))

  let balance = await queryContract(terra, contractAddress, { "balance": { "address": initialBalance.address } })
  strictEqual(balance.balance, initialBalance.amount)

  // Mint tokens
  const mintAmount = 1_000_000000
  const recipient = terra.wallets.test2.key.accAddress

  await performTransaction(terra, wallet, new MsgSend(
    wallet.key.accAddress,
    MULTISIG_ADDRESS,
    { uluna: 1_000_000000, uusd: 1_000_000000 }
  ))

  const mintMsg = { "mint": { "recipient": recipient, "amount": String(mintAmount) } }

  const tx = await multisig.createTx({
    msgs: [new MsgExecuteContract(MULTISIG_ADDRESS, contractAddress, mintMsg)],
    fee: new StdFee(30000000, [
      new Coin('uusd', 45000000)
    ]),
  })

  // TODO use temp files
  writeFileSync('unsigned_tx.json', tx.toStdTx().toJSON())

  // TODO loop over arbitrary number of keys
  let cli = new CLIKey({ keyName: "test1", multisig: MULTISIG_ADDRESS })
  let sig = await cli.createSignature(tx)
  writeFileSync('test1sig.json', sig.toJSON())

  let cli2 = new CLIKey({ keyName: "test2", multisig: MULTISIG_ADDRESS })
  let sig2 = await cli2.createSignature(tx)
  writeFileSync('test2sig.json', sig2.toJSON())

  // TODO extend CLIKey class and encapsulate this logic
  const signedTxData = execSync(
    `terracli tx multisign unsigned_tx.json multi test1sig.json test2sig.json ` +
    `--offline ` +
    `--chain-id ${tx.chain_id} --account-number ${tx.account_number} --sequence ${tx.sequence} `,
    { encoding: 'utf-8' }
  )

  const signedTx = StdTx.fromData(JSON.parse(signedTxData.toString()))

  const result = await terra.tx.broadcast(signedTx);
  if (isTxError(result)) {
    throw new Error(
      `transaction failed.code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }

  const tokenInfo = await queryContract(terra, contractAddress, { "token_info": {} })
  console.log(tokenInfo)
  strictEqual(tokenInfo.total_supply, String(initialAmount + mintAmount))

  balance = await queryContract(terra, contractAddress, { "balance": { "address": recipient } })
  console.log(balance)
  strictEqual(balance.balance, String(mintAmount))

  // Migrate contract version
  try {
    await migrate(terra, wallet, contractAddress, codeID)
  } catch (err) {
    // Contracts cannot be migrated to the same contract version, so we catch this error.
    // If we get this error, then the wallet has permissions to migrate the contract.
    if (!err.message.includes("migrate wasm contract failed: generic: Unknown version 0.2.0")) {
      throw err
    }
  }

  // Update contract owner
  const newOwner = terra.wallets.test2

  await performTransaction(terra, wallet, new MsgUpdateContractOwner(wallet.key.accAddress, newOwner.key.accAddress, contractAddress))

  const contractInfo = await terra.wasm.contractInfo(contractAddress)
  strictEqual(contractInfo.owner, newOwner.key.accAddress)

  console.log("OK")
}

main().catch(err => console.log(err))
