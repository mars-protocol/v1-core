import 'dotenv/config.js';
import {deployContract, recover, uploadContract} from "./helpers.mjs";
import {LCDClient, LocalTerra} from "@terra-money/terra.js";

async function deployBasecamp(cooldownDuration, unstakeWindow, codeId=undefined) {
  if (!codeId) {
    console.log("Uploading Cw20 Contract...");
    codeId = await uploadContract(terra, wallet, './artifacts/cw20_token.wasm');
  }

  console.log("Deploying Basecamp...");
  let initMsg = {"cw20_code_id": codeId, "cooldown_duration": cooldownDuration.toString(), "unstake_window": unstakeWindow.toString()};
  let basecampContractAddress = await deployContract(terra, wallet, './artifacts/basecamp.wasm', initMsg);

  console.log("Basecamp Contract Address: " + basecampContractAddress);
}

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

await deployBasecamp(cooldownDuration, unstakeWindow);

