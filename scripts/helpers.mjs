import {
  isTxError,
  MsgExecuteContract,
  MsgInstantiateContract,
  MsgStoreCode
} from '@terra-money/terra.js';
import {readFileSync} from 'fs';

export class Helpers {
  constructor(terra) {
    this.terra = terra;
  }

  async perform_transaction(wallet, msg) {
    const tx = await wallet.createAndSignTx({msgs: [msg]});
    const result = await this.terra.tx.broadcast(tx);
    if (isTxError(result)) {
      throw new Error(
        `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
      );
    }
    return result
  }

  async upload_contract(wallet, filepath) {
    const contract = readFileSync(filepath, 'base64');
    const upload_msg = new MsgStoreCode(wallet.key.accAddress, contract);
    let result = await this.perform_transaction(wallet, upload_msg);
    return Number(result.logs[0].events[1].attributes[1].value) //code_id
  }

  async instantiate_contract(wallet, code_id, msg) {
    const instantiate_msg = new MsgInstantiateContract(wallet.key.accAddress, code_id, msg);
    let result = await this.perform_transaction(wallet, instantiate_msg)
    return result.logs[0].events[0].attributes[2].value //contract address
  }

  async execute_contract(wallet, contract_address, msg) {
    const execute_msg = new MsgExecuteContract(wallet.key.accAddress, contract_address, msg);
    return await this.perform_transaction(wallet, execute_msg);
  }

  async query_contract(contract_address, query) {
    return await this.terra.wasm.contractQuery(
      contract_address,
      query
    )
  }
}

