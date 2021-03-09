import {LocalTerra, MsgExecuteContract, MsgInstantiateContract, MsgStoreCode} from '@terra-money/terra.js';
import {readFileSync} from 'fs';

function perform_transaction(msg) {
  return terra.wallets.test1.createAndSignTx({msgs: [msg]})
    .then(tx => terra.tx.broadcast(tx))
    .then(res => res)
}

function upload_contract(filepath) {
  const contract = readFileSync(filepath, 'base64');
  const upload_msg = new MsgStoreCode(sender, contract);
  return perform_transaction(upload_msg)
    .then(res => {
      return res.logs[0].events[1].attributes[1].value //code_id
    });
}

function instantiate_contract(code_id, msg) {
  const instantiate_msg = new MsgInstantiateContract(sender, 1, msg);
  return perform_transaction(instantiate_msg).then(res => {
    return res.logs[0].events[0].attributes[2].value //contract address
  });
}

function execute_contract(contract_address, msg) {
  const execute_msg = new MsgExecuteContract(sender, contract_address, msg);
  return perform_transaction(execute_msg).then(res => res);
}

const terra = new LocalTerra();
const sender = 'terra1x46rqay4d3cssq8gxxvqz8xt6nwlz4td20k38v';
const code_id = await upload_contract('./my_first_contract.wasm');
const contract_address = await instantiate_contract(code_id, {"count": 0});
let result = await execute_contract(contract_address, {"reset": {"count": 5}})
console.log(result);
