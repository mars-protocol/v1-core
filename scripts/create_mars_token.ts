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
import { unlinkSync, writeFileSync } from "fs"

const MULTISIG_ADDRESS = "terra1e0fx0q9meawrcq7fmma9x60gk35lpr4xk3884m"
const MULTISIG_NAME = "multi"
const MULTISIG_KEYS = ["test1", "test2"]

const TOKEN_NAME = "Mars"
const TOKEN_SYMBOL = "MARS"
const TOKEN_DECIMALS = 6
const TOKEN_MINTER = MULTISIG_ADDRESS
const TOKEN_CAP = 1_000_000_000_000000
const TOKEN_INITIAL_AMOUNT = 1_000_000_000000
const TOKEN_INITIAL_AMOUNT_ADDRESS = TOKEN_MINTER

const COSMWASM_PLUS_PATH = "../../cosmwasm-plus"

async function main() {
  setTimeoutDuration(10)

  const terra = new LocalTerra()
  const wallet = terra.wallets.test1

  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  // Compile contract
  // TODO use rust-optimizer
  execSync(
    `cd ${COSMWASM_PLUS_PATH}/contracts/cw20-base ` +
    `&& RUSTFLAGS='-C link-arg=-s' cargo wasm`,
    { encoding: 'utf-8' }
  )

  // Upload contract code
  const codeID = await uploadContract(terra, wallet, "../../cosmwasm-plus/target/wasm32-unknown-unknown/release/cw20_base.wasm")
  console.log(codeID)

  // Token info
  // TODO check if we want initial balances in prod
  const initialBalance = {
    address: TOKEN_INITIAL_AMOUNT_ADDRESS,
    amount: String(TOKEN_INITIAL_AMOUNT)
  }

  const TOKEN_INFO = {
    name: TOKEN_NAME,
    symbol: TOKEN_SYMBOL,
    decimals: TOKEN_DECIMALS,
    initial_balances: [initialBalance],
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

  let balance = await queryContract(terra, contractAddress, { balance: { address: initialBalance.address } })
  strictEqual(balance.balance, initialBalance.amount)

  // Mint tokens
  const mintAmount = 1_000_000000
  const recipient = terra.wallets.test2.key.accAddress

  await performTransaction(terra, wallet, new MsgSend(
    wallet.key.accAddress,
    MULTISIG_ADDRESS,
    { uluna: 1_000_000000, uusd: 1_000_000000 }
  ))

  const mintMsg = { mint: { recipient: recipient, amount: String(mintAmount) } }

  const tx = await multisig.createTx({
    msgs: [new MsgExecuteContract(MULTISIG_ADDRESS, contractAddress, mintMsg)],
    fee: new StdFee(30000000, [
      new Coin('uusd', 45000000)
    ]),
  })

  writeFileSync('unsigned_tx.json', tx.toStdTx().toJSON())

  let fns: Array<string> = []

  for (const key of MULTISIG_KEYS) {
    const cli = new CLIKey({ keyName: key, multisig: MULTISIG_ADDRESS })
    const sig = await cli.createSignature(tx)

    const fn = `${key}_sig.json`
    writeFileSync(fn, sig.toJSON())
    fns.push(fn)
  }

  // TODO extend CLIKey class and encapsulate this logic
  const signedTxData = execSync(
    `terracli tx multisign unsigned_tx.json multi ${fns.join(" ")} ` +
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
