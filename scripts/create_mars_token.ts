import { LocalTerra, MsgUpdateContractOwner } from "@terra-money/terra.js"
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

// const MULTISIG = "terra1e0fx0q9meawrcq7fmma9x60gk35lpr4xk3884m"
const TOKEN_NAME = "Mars"
const TOKEN_SYMBOL = "MARS"
const TOKEN_DECIMALS = 6
// const TOKEN_MINTER = MULTISIG
const TOKEN_CAP = 1_000_000_000_000000

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const wallet = terra.wallets.test1

  // Upload contract code
  const codeID = await uploadContract(terra, wallet, "../../cosmwasm-plus/target/wasm32-unknown-unknown/release/cw20_base.wasm")
  console.log(codeID)

  // Token info
  // tmp: should be TOKEN_MINTER in prod
  const minter = wallet.key.accAddress

  // tmp: check if we want initial balances in prod
  const initialAmount = 1_000_000000
  const initialBalance = { "address": minter, "amount": String(initialAmount) }

  const TOKEN_INFO = {
    "name": TOKEN_NAME,
    "symbol": TOKEN_SYMBOL,
    "decimals": TOKEN_DECIMALS,
    "initial_balances": [initialBalance],
    "mint": {
      "minter": minter, // TOKEN_MINTER,
      "cap": String(TOKEN_CAP)
    }
  }

  // Instantiate contract
  const contractAdress = await instantiateContract(terra, wallet, codeID, TOKEN_INFO)
  console.log(contractAdress)
  console.log(await queryContract(terra, contractAdress, { "token_info": {} }))
  console.log(await queryContract(terra, contractAdress, { "minter": {} }))

  let balance = await queryContract(terra, contractAdress, { "balance": { "address": initialBalance.address } })
  strictEqual(balance.balance, initialBalance.amount)

  // Mint tokens
  const mintAmount = 100
  const recipient = terra.wallets.test2.key.accAddress

  await executeContract(terra, wallet, contractAdress, { "mint": { "recipient": recipient, "amount": String(mintAmount) } })

  const tokenInfo = await queryContract(terra, contractAdress, { "token_info": {} })
  strictEqual(tokenInfo.total_supply, String(initialAmount + mintAmount))

  balance = await queryContract(terra, contractAdress, { "balance": { "address": recipient } })
  strictEqual(balance.balance, String(mintAmount))

  // Update contract owner
  const newOwner = terra.wallets.test2

  await performTransaction(terra, wallet, new MsgUpdateContractOwner(wallet.key.accAddress, newOwner.key.accAddress, contractAdress))

  const contractInfo = await terra.wasm.contractInfo(contractAdress)
  strictEqual(contractInfo.owner, newOwner.key.accAddress)

  // Migrate contract version
  try {
    await migrate(terra, wallet, contractAdress, codeID)
  } catch (err) {
    // Contracts cannot be migrated to the same contract version, so we catch this error.
    // If we get this error, then the wallet has permissions to migrate the contract.
    if (!err.message.includes("migrate wasm contract failed: generic: Unknown version 0.2.0")) {
      throw err
    }
  }

  console.log("OK")

  // let tx: any
  // let data = readFileSync('signed_tx.json', 'utf-8');

  // // (err, data) => {
  // //   if (err) {
  // //     throw err;
  // //   }

  // // parse JSON object
  // tx = JSON.parse(data.toString());

  // // print JSON object
  // // console.log(tx.value.signatures);

  // // console.log(tx)

  // const msg = MsgInstantiateContract.fromData(tx.value.msg[0])

  // const fee = StdFee.fromData(tx.value.fee)
  // const sig = StdSignature.fromData(tx.value.signatures[0])
  // const signedTx = new StdTx([msg], fee, [sig])

  // console.log(signedTx)

  // console.log(signedTx.toData())

  // const result = await terra.tx.broadcast(signedTx);
  // if (isTxError(result)) {
  //   throw new Error(
  //     `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
  //   );
  // }

  // const signMsg = await wallet.createTx({
  //   msgs: [instantiateMsg],
  //   fee: new StdFee(30000000, [
  //     new Coin('uusd', 45000000)
  //   ]),
  // });

  // writeFile('unsigned_tx.json', signMsg.toJSON(), (err) => {
  //   if (err) {
  //     throw err;
  //   }
  //   console.log("JSON data is saved.");
  // });

  // const sig = await wallet.key.createSignature(signMsg)
  // writeFile('test1sig.json', sig.toJSON(), (err) => {
  //   if (err) {
  //     throw err;
  //   }
  //   console.log("JSON data is saved.");
  // });
  // const sig2 = await wallet2.key.createSignature(signMsg)
  // writeFile('test2sig.json', sig2.toJSON(), (err) => {
  //   if (err) {
  //     throw err;
  //   }
  //   console.log("JSON data is saved.");
  // });


  // const tx = new StdTx(signMsg.msgs, signMsg.fee, [sig, sig2], signMsg.memo)
  // console.log(tx)

  // const result = await terra.tx.broadcast(tx);
  // if (isTxError(result)) {
  //   throw new Error(
  //     `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
  //   );
  // }

  // tx.account_number

  // new StdSignMsg(tx.chain_id, tx.account_number, tx.sequence, tx.fee, tx.msgs)

}

main().catch(err => console.log(err))
