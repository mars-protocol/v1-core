import {isTxError, MsgExecuteContract, MsgInstantiateContract, MsgStoreCode} from '@terra-money/terra.js';
import { readFileSync } from 'fs';

export async function performTransaction(terra, wallet, msg) {
  const tx = await wallet.createAndSignTx({msgs: [msg]});
  const result = await terra.tx.broadcast(tx);
  if (isTxError(result)) {
    throw new Error(
      `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }
  return result
}

export async function uploadContract(terra, wallet, filepath) {
  const contract = readFileSync(filepath, 'base64');
  const uploadMsg = new MsgStoreCode(wallet.key.accAddress, contract);
  let result = await performTransaction(terra, wallet, uploadMsg);
  return Number(result.logs[0].events[1].attributes[1].value) //code_id
}

export async function instantiateContract(terra, wallet, codeId, msg) {
  const instantiateMsg = new MsgInstantiateContract(wallet.key.accAddress, codeId, msg);
  let result = await performTransaction(terra, wallet, instantiateMsg)
  return result.logs[0].events[0].attributes[2].value //contract address
}

export async function executeContract(terra, wallet, contractAddress, msg) {
  const executeMsg = new MsgExecuteContract(wallet.key.accAddress, contractAddress, msg);
  return await performTransaction(terra, wallet, executeMsg);
}

export async function queryContract(terra, contractAddress, query) {
  return await terra.wasm.contractQuery(
    contractAddress,
    query
  )
}

export async function deployContract(terra, wallet, filepath, initMsg) {
  const codeId = await uploadContract(terra, wallet, filepath);
  return await instantiateContract(terra, wallet, codeId, initMsg);
}

export async function deploy(terra, wallet) {
  let maCodeId = await uploadContract(terra, wallet, './artifacts/ma_token.wasm');
  console.log("Uploaded wa_token contract");

  const lpInitMsg = {"ma_token_code_id": maCodeId};
  const lpContractAddress = await deployContract(terra, wallet,'./artifacts/liquidity_pool.wasm', lpInitMsg);

  console.log("Uploaded and instantiated liquidity_pool contract");

  const lpLunaExecuteMsg = {"init_asset": {"symbol": "luna"}};
  const lpUsdExecuteMsg = {"init_asset": {"symbol": "usd"}};

  await executeContract(terra, wallet, lpContractAddress, lpLunaExecuteMsg);
  await executeContract(terra, wallet, lpContractAddress, lpUsdExecuteMsg);
  console.log("Initialized luna and usd assets for liquidity_pool");

  console.log("LP Contract Address: " + lpContractAddress);
  return lpContractAddress;
}



