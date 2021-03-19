import 'dotenv/config.js';
import {migrate, executeContract, deploy} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";

async function main() {
  const terra = new LocalTerra();
  const wallet = terra.wallets.test1;
  const contractAddress = await deploy(terra, wallet);
  console.log('migrating...');
  const migrateResult = await migrate(terra, wallet, contractAddress);

  console.log("migration complete: ");
  console.log(migrateResult);
}

main().catch(err => console.log(err));
