import { upload_contract, deploy_contract, execute_contract } from "./helpers.mjs";
import { LocalTerra } from "@terra-money/terra.js";
import { fileURLToPath } from 'url'

export async function deploy_local(terra, wallet) {
  let ma_code_id = await upload_contract(terra, wallet, '../artifacts/ma_token.wasm');

  const lp_init_msg = {"ma_token_code_id": ma_code_id};
  const lp_contract_address = await deploy_contract(terra, wallet,'../artifacts/liquidity_pool.wasm', lp_init_msg)

  const lp_luna_execute_msg = {"init_asset": {"symbol": "luna"}};
  const lp_usd_execute_msg = {"init_asset": {"symbol": "usd"}};

  await execute_contract(terra, wallet, lp_contract_address, lp_luna_execute_msg);
  await execute_contract(terra, wallet, lp_contract_address, lp_usd_execute_msg);

  console.log("LP Contract Address: " + lp_contract_address);

  return lp_contract_address;
}

// Checks if running directly
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;
  deploy_local(terra, wallet);
}

