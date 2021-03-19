import 'dotenv/config.js';
import {migrate, executeContract, deploy} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";

async function main() {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;
  const contractAddress = await deploy(terra, wallet);
  console.log('migrating...');
  let res = await migrate(terra, wallet, contractAddress);
  console.log(res);

  let initAssetMsg = {"init_asset": {"denom": "uusd"}};
  res = await executeContract(terra, wallet, contractAddress, initAssetMsg);
  console.log(res);
}

main().catch(err => console.log(err));
