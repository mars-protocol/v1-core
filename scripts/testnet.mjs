import 'dotenv/config.js';
import {LCDClient, MnemonicKey} from "@terra-money/terra.js";
import {deploy} from "./helpers.mjs";

export function initialize(terra) {
  const mk = new MnemonicKey();

  console.log(`Account Address: ${mk.accAddress}`);
  console.log(`MnemonicKey: ${mk.mnemonic}`);

  return terra.wallet(mk);
}

export function recover(terra, mnemonic) {
  const mk = new MnemonicKey({mnemonic: mnemonic});
  return terra.wallet(mk);
}

const terra = new LCDClient({
  URL: 'https://tequila-lcd.terra.dev',
  chainID: 'tequila-0004'
});

// const wallet = initialize(terra);
const wallet = await recover(terra, process.env.TEST_MAIN);
