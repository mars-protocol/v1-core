import {
  Coin,
  isTxError,
  MsgExecuteContract,
  MsgInstantiateContract,
  MsgMigrateContract,
  MsgStoreCode,
  StdFee,
  MnemonicKey
} from '@terra-money/terra.js';
import { readFileSync } from 'fs';

export async function performTransaction(terra, wallet, msg) {
  const tx = await wallet.createAndSignTx({
    msgs: [msg],
    fee: new StdFee(30000000, [
      new Coin('uluna', 4500000),
      new Coin('uusd', 4500000)
    ]),
  });
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
  return Number(result.logs[0].eventsByType.store_code.code_id[0]) //code_id
}

export async function instantiateContract(terra, wallet, codeId, msg) {
  const instantiateMsg = new MsgInstantiateContract(wallet.key.accAddress, codeId, msg, undefined, true);
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

export async function deployLiquidityPool(terra, wallet, cache = {}) {
  let cw20CodeId = cache.cw20CodeId;
  if (cw20CodeId) {
    console.log(`Using existing cw20_token, code_id: ${cw20CodeId}`)
  } else {
    cw20CodeId = await uploadContract(terra, wallet, './artifacts/cw20_token.wasm');
    console.log(`Uploaded cw20_token contract code: ${cw20CodeId}`);
  }

  let lpCodeId = cache.lpCodeId;
  if (lpCodeId) {
    console.log(`Using existing lpCodeId, code_id: ${lpCodeId}`)
  } else {
    lpCodeId = await uploadContract(terra, wallet, './artifacts/liquidity_pool.wasm');
    console.log(`Instantiated liquidity_pool contract, code_id: ${lpCodeId}`);
  }

  const lpInitMsg = {"ma_token_code_id": cw20CodeId};
  const lpAddress = await instantiateContract(terra, wallet, lpCodeId, lpInitMsg);
  console.log(`Instantiated liquidity_pool contract: address: ${lpAddress}`);

  return {
    lpAddress,
    lpCodeId,
    cw20CodeId,
  };
}

export async function setupLiquidityPool(terra, wallet, contractAddress, options) {
  const initialAssets = options.initialAssets ?? [];
  const initialDeposits = options.initialDeposits ?? [];
  const initialBorrows = options.initialBorrows ?? [];

  for (let asset of initialAssets) {
    if (asset.denom) {
      let initAssetMsg = {
        "init_asset": {
          "asset": {
            "native": {
              "denom": asset.denom,
            }
          },
          "asset_params": {
            "borrow_slope": asset.borrow_slope,
            "loan_to_value": asset.loan_to_value
          },
        },
      };
      await executeContract(terra, wallet, contractAddress, initAssetMsg);
      console.log("Initialized " + asset.denom);
    } else if (asset.contract_addr) {
      let initAssetMsg = {
        "init_asset": {
          "asset": {
            "cw20": {
              "contract_addr": asset.contract_addr,
            }
          },
          "asset_params": {
            "borrow_slope": asset.borrow_slope,
            "loan_to_value": asset.loan_to_value
          },
        },
      };
      await executeContract(terra, wallet, contractAddress, initAssetMsg);
      console.log(`Initialized ${asset.symbol || asset.contract_addr}`);
    }
  }

  for (let deposit of initialDeposits) {
    const { account, assets } = deposit;
    console.log(`### Deposits for account ${account.key.accAddress}: `);
    for (const [asset, amount] of Object.entries(assets)) {
      const coins = new Coin(asset, amount);
      const depositMsg = {"deposit_native": {"denom": asset}};
      const executeDepositMsg = new MsgExecuteContract(account.key.accAddress, contractAddress, depositMsg, [coins]);
      await performTransaction(terra, account, executeDepositMsg);
      console.log(`Deposited ${amount} ${asset}`);
    }
  }

  for (let borrow of initialBorrows) {
    const { account, assets } = borrow;
    console.log(`### Borrows for account ${account.key.accAddress}: `);
    for (const [asset, amount] of Object.entries(assets)) {
      const borrowMsg = {"borrow": {
        "asset": {
          "native": {
            "denom": asset
          }
        },
        "amount": amount.toString()
        }
      };
      const executeBorrowMsg = new MsgExecuteContract(account.key.accAddress, contractAddress, borrowMsg);
      await performTransaction(terra, account, executeBorrowMsg);
      console.log(`Borrowed ${amount} ${asset}`);
    }
  }
}

export async function migrate(terra, wallet, contractAddress, newCodeId) {
  const migrateMsg = new MsgMigrateContract(wallet.key.accAddress, contractAddress, newCodeId, {});
  return await performTransaction(terra, wallet, migrateMsg);
}

export function recover(terra, mnemonic) {
  const mk = new MnemonicKey({ mnemonic: mnemonic });
  return terra.wallet(mk);
}

export function initialize(terra) {
  const mk = new MnemonicKey();

  console.log(`Account Address: ${mk.accAddress}`);
  console.log(`MnemonicKey: ${mk.mnemonic}`);

  return terra.wallet(mk);
}

export async function deployBasecampContract(terra, wallet, cooldownDuration, unstakeWindow, codeId=undefined) {
  if (!codeId) {
    console.log("Uploading Cw20 Contract...");
    codeId = await uploadContract(terra, wallet, './artifacts/cw20_token.wasm');
  }

  console.log("Deploying Basecamp...");
  let initMsg = {"cw20_code_id": codeId, "cooldown_duration": cooldownDuration, "unstake_window": unstakeWindow};
  let basecampCodeId = await uploadContract(terra, wallet, './artifacts/basecamp.wasm');
  const instantiateMsg = new MsgInstantiateContract(wallet.key.accAddress, basecampCodeId, initMsg, undefined, true);
  let result = await performTransaction(terra, wallet, instantiateMsg);

  let basecampContractAddress = result.logs[0].eventsByType.from_contract.contract_address[0];

  console.log("Basecamp Contract Address: " + basecampContractAddress);
  return basecampContractAddress
}
