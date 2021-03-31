import 'dotenv/config.js';
import { LCDClient } from "@terra-money/terra.js";
import { deploy, setup, recover } from "./helpers.mjs";

const terra = new LCDClient({
    URL: 'https://tequila-lcd.terra.dev',
    chainID: 'tequila-0004'
})
const wallet = await recover(terra, process.env.TEST_MAIN);
let lpContractAddress = await deploy(terra, wallet);

const initialAssets = ["uluna", "uusd"];

await setup(terra, wallet, lpContractAddress, { initialAssets });
