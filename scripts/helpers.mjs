import {isTxError, MsgExecuteContract, MsgInstantiateContract, MsgStoreCode} from '@terra-money/terra.js';
import {readFileSync} from 'fs';

export async function perform_transaction(terra, wallet, msg) {
  const tx = await wallet.createAndSignTx({msgs: [msg]});
  const result = await terra.tx.broadcast(tx);
  if (isTxError(result)) {
    throw new Error(
      `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }
  return result
}

export async function upload_contract(terra, wallet, filepath) {
  const contract = readFileSync(filepath, 'base64');
  const upload_msg = new MsgStoreCode(wallet.key.accAddress, contract);
  let result = await perform_transaction(terra, wallet, upload_msg);
  return Number(result.logs[0].events[1].attributes[1].value) //code_id
}

export async function instantiate_contract(terra, wallet, code_id, msg) {
  const instantiate_msg = new MsgInstantiateContract(wallet.key.accAddress, code_id, msg);
  let result = await perform_transaction(terra, wallet, instantiate_msg)
  return result.logs[0].events[0].attributes[2].value //contract address
}

export async function execute_contract(terra, wallet, contract_address, msg) {
  const execute_msg = new MsgExecuteContract(wallet.key.accAddress, contract_address, msg);
  return await perform_transaction(terra, wallet, execute_msg);
}

export async function query_contract(terra, contract_address, query) {
  return await terra.wasm.contractQuery(
    contract_address,
    query
  )
}

export async function deploy_contract(terra, wallet, filepath, init_msg) {
  const code_id = await upload_contract(terra, wallet, filepath);
  return await instantiate_contract(terra, wallet, code_id, init_msg);
}


