import { Helpers } from "./helpers.mjs";
import { LocalTerra } from "@terra-money/terra.js";

export class LocalEnv extends Helpers {

  constructor() {
    const terra = new LocalTerra();
    const wallet = terra.wallets.test1;
    super(terra, wallet);
  }

  async deploy_local() {
    let ma_code_id = await super.upload_contract('../artifacts/ma_token.wasm');

    const lp_init_msg = {"ma_token_code_id": ma_code_id};
    const lp_contract_address = await super.deploy_contract('../artifacts/liquidity_pool.wasm', lp_init_msg)

    const lp_luna_execute_msg = {"init_asset": {"symbol": "luna"}};
    const lp_usd_execute_msg = {"init_asset": {"symbol": "usd"}};

    await super.execute_contract(lp_contract_address, lp_luna_execute_msg);
    await super.execute_contract(lp_contract_address, lp_usd_execute_msg);

    console.log("LP Contract Address: " + lp_contract_address);

    return lp_contract_address;
  }

}
