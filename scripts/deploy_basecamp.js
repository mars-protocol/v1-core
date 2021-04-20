import 'dotenv/config.js';
import {deployBasecampContract, recover} from "./helpers.mjs";
import {LCDClient, LocalTerra} from "@terra-money/terra.js";

async function main() {
  let terra;
  let wallet;

  if (process.env.NETWORK === "testnet") {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = await recover(terra, process.env.TEST_MAIN);
    console.log(wallet.key.accAddress);
  } else {
    terra = new LocalTerra();
    wallet = terra.wallets.test1;
  }

  let cooldownDuration;
  let unstakeWindow;

  if (process.env.NETWORK === "testnet") {
    cooldownDuration = 300;
    unstakeWindow = 300;
  } else {
    cooldownDuration = 1;
    unstakeWindow = 30;
  }

  await deployBasecampContract(terra, wallet, cooldownDuration, unstakeWindow);
}

main().catch(console.log);
