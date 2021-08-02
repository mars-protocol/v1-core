/*
Script to deploy a cw20 token from a multisig account, mint tokens, and migrate the contract.

This script is designed to work with Terra Columbus-4.

Dependencies:
  - rust
  - terracli 58602320d2907814cfccdf43e9679468bb4bd8d3
  - cosmwasm-plus v0.2.0 (NB set `COSMWASM_PLUS_PATH` below)
  - Add test accounts and multisig to terracli (see below)

Dependencies to run on LocalTerra:
  - docker
  - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5

tequila-0004 testnet:
```
terracli keys add tequila1 --recover
giggle ride master symbol south mail desert mother three endless edit draw flush aware hub parent tiny discover convince fox execute bulb promote walnut

terracli keys add tequila2 --recover
electric clarify defy one aisle south monitor float nature comic ring slice return try uncover evidence regret daughter shy rack shine dish bitter pulse

terracli keys add tequila3 --recover
save churn cousin clown valve exit worth wave major ozone hub pyramid speak dawn unusual pyramid gold hole lottery guilt solve urge join indoor

# Multisig
terracli keys add tequilamulti \
  --multisig=tequila1,tequila2,tequila3 \
  --multisig-threshold=2
```

LocalTerra:
```
terracli keys add test1 --recover
notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius

terracli keys add test2 --recover
quality vacuum heart guard buzz spike sight swarm shove special gym robust assume sudden deposit grid alcohol choice devote leader tilt noodle tide penalty

terracli keys add test3 --recover
symbol force gallery make bulk round subway violin worry mixture penalty kingdom boring survey tool fringe patrol sausage hard admit remember broken alien absorb

# Multisig
terracli keys add multi \
  --multisig=test1,test2,test3 \
  --multisig-threshold=2
```
*/

import {
  Coin,
  isTxError,
  LCDClient,
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
import { join } from "path"
import {
  instantiateContract,
  migrate,
  performTransaction,
  queryContract,
  recover,
  setTimeoutDuration,
  uploadContract
} from "./helpers.js"

const NETWORK: string = "localterra"
// const NETWORK: string = "testnet"

const COSMWASM_PLUS_PATH = join("..", "..", "cosmwasm-plus")

async function main() {
  let terra: LCDClient | LocalTerra
  let wallet: Wallet

  let MULTISIG_ADDRESS: string
  let MULTISIG_NAME: string
  let MULTISIG_KEYS: Array<string>
  let MULTISIG_THRESHOLD: number

  let codeID: number

  const isTestnet = NETWORK === "testnet" || NETWORK === "tequila-0004"

  if (isTestnet) {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = recover(terra, "giggle ride master symbol south mail desert mother three endless edit draw flush aware hub parent tiny discover convince fox execute bulb promote walnut")

    MULTISIG_ADDRESS = "terra1sl6fqdmx9qexqz72qreg5lw8cnngu396u6gryu"
    MULTISIG_NAME = "tequilamulti"
    MULTISIG_KEYS = ["tequila1", "tequila2", "tequila3"]
    MULTISIG_THRESHOLD = 2

    // Code ID on tequila-0004 for cw20_base.wasm v0.2.0 built with workspace-optimizer:0.11.4
    codeID = 7117

  } else {
    setTimeoutDuration(0)

    terra = new LocalTerra()

    wallet = (terra as LocalTerra).wallets.test1

    MULTISIG_ADDRESS = "terra1e0fx0q9meawrcq7fmma9x60gk35lpr4xk3884m"
    MULTISIG_NAME = "multi"
    MULTISIG_KEYS = ["test1", "test2", "test3"]
    MULTISIG_THRESHOLD = 2

    // Compile contract
    execSync(
      `cd ${join(COSMWASM_PLUS_PATH, "contracts", "cw20-base")} ` +
      `&& RUSTFLAGS='-C link-arg=-s' cargo wasm `,
      { encoding: 'utf-8' }
    )

    // Upload contract code
    codeID = await uploadContract(terra, wallet, join(COSMWASM_PLUS_PATH, "target", "wasm32-unknown-unknown", "release", "cw20_base.wasm"))
  }

  // Token info
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

  const multisig = new Wallet(terra, new CLIKey({ keyName: MULTISIG_NAME }))

  // Instantiate contract
  const contractAddress = await instantiateContract(terra, wallet, codeID, TOKEN_INFO)
  console.log(codeID)
  console.log(contractAddress)
  console.log(await queryContract(terra, contractAddress, { token_info: {} }))
  console.log(await queryContract(terra, contractAddress, { minter: {} }))

  let balance = await queryContract(terra, contractAddress, { balance: { address: TOKEN_INFO.initial_balances[0].address } })
  strictEqual(balance.balance, TOKEN_INFO.initial_balances[0].amount)

  // Mint tokens
  const mintAmount = 1_000_000000
  const recipient = wallet.key.accAddress

  // Send coins to the multisig address. On testnet, use the faucet to initialise the multisig balance.
  if (!isTestnet) {
    await performTransaction(terra, wallet, new MsgSend(
      wallet.key.accAddress,
      MULTISIG_ADDRESS,
      { uluna: 1_000_000000, uusd: 1_000_000000 }
    ))
  }

  // Create an unsigned tx
  const mintMsg = { mint: { recipient: recipient, amount: String(mintAmount) } }
  const tx = await multisig.createTx({
    msgs: [new MsgExecuteContract(MULTISIG_ADDRESS, contractAddress, mintMsg)],
    fee: new StdFee(30000000, [
      new Coin('uusd', 45000000)
    ]),
  })
  writeFileSync('unsigned_tx.json', tx.toStdTx().toJSON())

  // Create K of N signatures for the tx
  let fns: Array<string> = []
  for (const key of MULTISIG_KEYS.slice(0, MULTISIG_THRESHOLD)) {
    const cli = new CLIKey({ keyName: key, multisig: MULTISIG_ADDRESS })
    const sig = await cli.createSignature(tx)

    const fn = `${key}_sig.json`
    writeFileSync(fn, sig.toJSON())
    fns.push(fn)
  }

  // Create a signed tx by aggregating the K signatures
  const signedTxData = execSync(
    `terracli tx multisign unsigned_tx.json ${MULTISIG_NAME} ${fns.join(" ")} ` +
    `--offline ` +
    `--chain-id ${tx.chain_id} --account-number ${tx.account_number} --sequence ${tx.sequence} `,
    { encoding: 'utf-8' }
  )

  // Broadcast the tx
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
  const newOwner = MULTISIG_ADDRESS

  await performTransaction(terra, wallet, new MsgUpdateContractOwner(wallet.key.accAddress, newOwner, contractAddress))

  const contractInfo = await terra.wasm.contractInfo(contractAddress)
  strictEqual(contractInfo.owner, newOwner)

  console.log("OK")
}

main().catch(err => console.log(err))
