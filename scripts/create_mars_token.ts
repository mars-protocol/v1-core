/*
Script to deploy a cw20 token from a multisig account, mint tokens, and migrate the contract.

This script is designed to work with Terra Columbus-4.

Dependencies:
  - rust
  - terracli 58602320d2907814cfccdf43e9679468bb4bd8d3
  - cosmwasm-plus v0.2.0 (NB set `COSMWASM_PLUS_PATH` below)

Dependencies to run LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
  - Add test accounts and multisig to terracli (see below)

LocalTerra test accounts:
```
terracli keys add test1 --recover
notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius

terracli keys add test2 --recover
quality vacuum heart guard buzz spike sight swarm shove special gym robust assume sudden deposit grid alcohol choice devote leader tilt noodle tide penalty

terracli keys add test3 --recover
symbol force gallery make bulk round subway violin worry mixture penalty kingdom boring survey tool fringe patrol sausage hard admit remember broken alien absorb
```

Multisig:
```
terracli keys add multi \
  --multisig=test1,test2,test3 \
  --multisig-threshold=2
```
*/

import {
  Coin,
  isTxError,
  LocalTerra,
  MsgExecuteContract,
  MsgSend,
  MsgUpdateContractOwner,
  StdFee,
  StdTx,
  Wallet
} from "@terra-money/terra.js"
import { CLIKey } from "@terra-money/terra.js/dist/key/CLIKey.js"
import { strictEqual } from "assert"
import { execSync } from "child_process"
import { unlinkSync, writeFileSync } from "fs"
import {
  instantiateContract,
  migrate,
  performTransaction,
  queryContract,
  setTimeoutDuration,
  uploadContract
} from "./helpers.js"

const MULTISIG_ADDRESS = "terra1e0fx0q9meawrcq7fmma9x60gk35lpr4xk3884m"
const MULTISIG_NAME = "multi"
const MULTISIG_KEYS = ["test1", "test2", "test3"]
const MULTISIG_THRESHOLD = 2

const TOKEN_NAME = "Mars"
const TOKEN_SYMBOL = "MARS"
const TOKEN_DECIMALS = 6
const TOKEN_MINTER = MULTISIG_ADDRESS
const TOKEN_CAP = 1_000_000_000_000000
// TODO check if we want initial balances in prod
const TOKEN_INITIAL_AMOUNT = 1_000_000_000000
const TOKEN_INITIAL_AMOUNT_ADDRESS = TOKEN_MINTER

const TOKEN_INFO = {
  name: TOKEN_NAME,
  symbol: TOKEN_SYMBOL,
  decimals: TOKEN_DECIMALS,
  initial_balances: [
    {
      address: TOKEN_INITIAL_AMOUNT_ADDRESS,
      amount: String(TOKEN_INITIAL_AMOUNT)
    }
  ],
  mint: {
    minter: TOKEN_MINTER,
    cap: String(TOKEN_CAP)
  }
}

const COSMWASM_PLUS_PATH = "../../cosmwasm-plus"

async function main() {
  setTimeoutDuration(0)

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

  // Instantiate contract
  const contractAddress = await instantiateContract(terra, wallet, codeID, TOKEN_INFO)
  console.log(contractAddress)
  console.log(await queryContract(terra, contractAddress, { token_info: {} }))
  console.log(await queryContract(terra, contractAddress, { minter: {} }))

  let balance = await queryContract(terra, contractAddress, { balance: { address: TOKEN_INFO.initial_balances[0].address } })
  strictEqual(balance.balance, TOKEN_INFO.initial_balances[0].amount)

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

  for (const key of MULTISIG_KEYS.slice(0, MULTISIG_THRESHOLD)) {
    const cli = new CLIKey({ keyName: key, multisig: MULTISIG_ADDRESS })
    const sig = await cli.createSignature(tx)

    const fn = `${key}_sig.json`
    writeFileSync(fn, sig.toJSON())
    fns.push(fn)
  }

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
  const newOwner = terra.wallets.test2.key.accAddress

  await performTransaction(terra, wallet, new MsgUpdateContractOwner(wallet.key.accAddress, newOwner, contractAddress))

  const contractInfo = await terra.wasm.contractInfo(contractAddress)
  strictEqual(contractInfo.owner, newOwner)

  console.log("OK")
}

main().catch(err => console.log(err))
