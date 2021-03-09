import {isTxError, LocalTerra, MsgExecuteContract, MsgInstantiateContract, MsgStoreCode} from '@terra-money/terra.js';
import {readFileSync} from 'fs';

async function perform_transaction(wallet, msg) {
  const tx = await wallet.createAndSignTx({msgs: [msg]});
  const result = await terra.tx.broadcast(tx);
  if (isTxError(result)) {
    throw new Error(
      `instantiate failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }
  return result
}

async function upload_contract(wallet, filepath) {
  const contract = readFileSync(filepath, 'base64');
  const upload_msg = new MsgStoreCode(wallet.key.accAddress, contract);
  let result = await perform_transaction(wallet, upload_msg);
  return Number(result.logs[0].events[1].attributes[1].value) //code_id
}

async function instantiate_contract(wallet, code_id, msg) {
  const instantiate_msg = new MsgInstantiateContract(wallet.key.accAddress, code_id, msg);
  let result = await perform_transaction(wallet, instantiate_msg)
  return result.logs[0].events[0].attributes[2].value //contract address
}

async function execute_contract(wallet, contract_address, msg) {
  const execute_msg = new MsgExecuteContract(wallet.key.accAddress, contract_address, msg);
  return await perform_transaction(wallet, execute_msg);
}

async function query_contract(contract_address, query) {
  return await terra.wasm.contractQuery(
    contract_address,
    query  // query msg
  )
}


const terra = new LocalTerra();
const test1 = terra.wallets.test1;
const code_id = await upload_contract(test1, './my_first_contract.wasm');
const contract_address = await instantiate_contract(test1, code_id, {"count": 0});
let result = await execute_contract(test1, contract_address, {"reset": {"count": 5}});
console.log(result);
let query_result = await query_contract(contract_address, {"get_count": {}});
console.log(query_result)
