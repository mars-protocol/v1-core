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
      new Coin('uusd', 45000000)
    ]),
  });
  const result = await terra.tx.broadcast(tx);
  if (isTxError(result)) {
    throw new Error(
      `transaction failed. code: ${result.code}, codespace: ${result.codespace}, raw_log: ${result.raw_log}`
    );
  }

  // Can't send txs too fast, tequila lcd is load balanced, 
  // account sequence query may resolve an older state depending on which lcd you end up with,
  // generally 1 sec is enough for all nodes to sync up.
  await new Promise(resolve => setTimeout(resolve, 1000));

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

export async function executeContract(terra, wallet, contractAddress, msg, coins = undefined) {
  const executeMsg = new MsgExecuteContract(wallet.key.accAddress, contractAddress, msg, coins);
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

export async function setupRedBank(terra, wallet, contractAddress, options) {
  console.log("Setting up initial asset liquidity pools...");

  const initialAssets = options.initialAssets ?? [];
  const initialDeposits = options.initialDeposits ?? [];
  const initialBorrows = options.initialBorrows ?? [];

  for (let asset of initialAssets) {
    console.log(`Initializing ${asset.denom || asset.symbol || asset.contract_addr}`);

    let assetType = asset.denom
      ? {
        "native": {
          "denom": asset.denom,
        }
      }
      : asset.contract_addr
        ? {
          "cw20": {
            "contract_addr": asset.contract_addr,
          }
        }
        : undefined
    let assetParams = {
      initial_borrow_rate: asset.initial_borrow_rate,
      min_borrow_rate: asset.min_borrow_rate,
      max_borrow_rate: asset.max_borrow_rate,
      max_loan_to_value: asset.max_loan_to_value,
      reserve_factor: asset.reserve_factor,
      maintenance_margin: asset.maintenance_margin,
      liquidation_bonus: asset.liquidation_bonus,
      kp: asset.kp,
      optimal_utilization_rate: asset.optimal_utilization_rate,
      kp_augmentation_threshold: asset.kp_augmentation_threshold,
      kp_multiplier: asset.kp_multiplier
    }

    let initAssetMsg = {
      "init_asset": {
        "asset": assetType,
        "asset_params": assetParams,
      },
    };

    await executeContract(terra, wallet, contractAddress, initAssetMsg);
    console.log(`Initialized ${asset.denom || asset.symbol || asset.contract_addr}`);
  }

  for (let deposit of initialDeposits) {
    const { account, assets } = deposit;
    console.log(`### Deposits for account ${account.key.accAddress}: `);
    for (const [asset, amount] of Object.entries(assets)) {
      const coins = new Coin(asset, amount);
      const depositMsg = { "deposit_native": { "denom": asset } };
      const executeDepositMsg = new MsgExecuteContract(account.key.accAddress, contractAddress, depositMsg, [coins]);
      await performTransaction(terra, account, executeDepositMsg);
      console.log(`Deposited ${amount} ${asset}`);
    }
  }

  for (let borrow of initialBorrows) {
    const { account, assets } = borrow;
    console.log(`### Borrows for account ${account.key.accAddress}: `);
    for (const [asset, amount] of Object.entries(assets)) {
      const borrowMsg = {
        "borrow": {
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

export function toEncodedBinary(object) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}