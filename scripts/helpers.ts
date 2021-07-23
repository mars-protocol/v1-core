import {
  Coin,
  isTxError,
  LCDClient,
  MnemonicKey,
  Msg,
  MsgExecuteContract,
  MsgInstantiateContract,
  MsgMigrateContract,
  MsgStoreCode,
  StdFee,
  Wallet
} from '@terra-money/terra.js';
import { readFileSync } from 'fs';

// Tequila lcd is load balanced, so txs can't be sent too fast, otherwise account sequence queries
// may resolve an older state depending on which lcd you end up with. Generally 1000 ms is is enough
// for all nodes to sync up.
let TIMEOUT = 1000

export function setTimeoutDuration(t: number) {
  TIMEOUT = t
}

export function getTimeoutDuration() {
  return TIMEOUT
}

export async function performTransaction(terra: LCDClient, wallet: Wallet, msg: Msg) {
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
  await new Promise(resolve => setTimeout(resolve, TIMEOUT));
  return result
}

export async function uploadContract(terra: LCDClient, wallet: Wallet, filepath: string) {
  const contract = readFileSync(filepath, 'base64');
  const uploadMsg = new MsgStoreCode(wallet.key.accAddress, contract);
  let result = await performTransaction(terra, wallet, uploadMsg);
  return Number(result.logs[0].eventsByType.store_code.code_id[0]) // code_id
}

export async function instantiateContract(terra: LCDClient, wallet: Wallet, codeId: number, msg: object) {
  const instantiateMsg = new MsgInstantiateContract(wallet.key.accAddress, codeId, msg, undefined, true);
  let result = await performTransaction(terra, wallet, instantiateMsg)
  return result.logs[0].events[0].attributes[2].value // contract address
}

export async function executeContract(terra: LCDClient, wallet: Wallet, contractAddress: string, msg: object, coins?: string) {
  const executeMsg = new MsgExecuteContract(wallet.key.accAddress, contractAddress, msg, coins);
  return await performTransaction(terra, wallet, executeMsg);
}

export async function queryContract(terra: LCDClient, contractAddress: string, query: object): Promise<any> {
  return await terra.wasm.contractQuery(contractAddress, query)
}

export async function deployContract(terra: LCDClient, wallet: Wallet, filepath: string, initMsg: object) {
  const codeId = await uploadContract(terra, wallet, filepath);
  return await instantiateContract(terra, wallet, codeId, initMsg);
}

export async function migrate(terra: LCDClient, wallet: Wallet, contractAddress: string, newCodeId: number) {
  const migrateMsg = new MsgMigrateContract(wallet.key.accAddress, contractAddress, newCodeId, {});
  return await performTransaction(terra, wallet, migrateMsg);
}

export function recover(terra: LCDClient, mnemonic: string) {
  const mk = new MnemonicKey({ mnemonic: mnemonic });
  return terra.wallet(mk);
}

export function initialize(terra: LCDClient) {
  const mk = new MnemonicKey();

  console.log(`Account Address: ${mk.accAddress}`);
  console.log(`MnemonicKey: ${mk.mnemonic}`);

  return terra.wallet(mk);
}

export async function setupRedBank(terra: LCDClient, wallet: Wallet, contractAddress: string, options: any) {
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
    let assetParams: Asset = {
      initial_borrow_rate: asset.initial_borrow_rate,
      min_borrow_rate: asset.min_borrow_rate,
      max_borrow_rate: asset.max_borrow_rate,
      max_loan_to_value: asset.max_loan_to_value,
      reserve_factor: asset.reserve_factor,
      maintenance_margin: asset.maintenance_margin,
      liquidation_bonus: asset.liquidation_bonus,
      kp_1: asset.kp_1,
      optimal_utilization_rate: asset.optimal_utilization_rate,
      kp_augmentation_threshold: asset.kp_augmentation_threshold,
      kp_2: asset.kp_2
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
    for (const asset of Object.keys(assets)) {
      const amount = assets[asset]
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
    for (const asset of Object.keys(assets)) {
      const amount = assets[asset]
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

export function toEncodedBinary(object: any) {
  return Buffer.from(JSON.stringify(object)).toString('base64');
}
