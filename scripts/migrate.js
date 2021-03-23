import 'dotenv/config.js';
import {migrate, uploadContract} from "./helpers.mjs";
import {LCDClient, LocalTerra} from "@terra-money/terra.js";
import {recover} from "./testnet.mjs";

async function main() {
  let terra;
  let wallet;
  let lpContractAddress;

  if (process.env.NETWORK === "testnet") {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = await recover(terra, process.env.TEST_MAIN);
    lpContractAddress = process.env.LP_TESTNET;
  } else {
    terra = new LocalTerra();
    wallet = terra.wallets.test1;
    lpContractAddress = process.env.LP_LOCAL;
  }

  console.log("uploading...");
  const newCodeId = await uploadContract(terra, wallet, process.env.LP_FILEPATH);

  console.log('migrating...');
  const migrateResult = await migrate(terra, wallet, lpContractAddress, newCodeId);

  console.log("migration complete: ");
  console.log(migrateResult);
}

main().catch(err => console.log(err));
