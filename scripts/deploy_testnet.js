import 'dotenv/config.js';
import { LCDClient } from "@terra-money/terra.js";
import { deploy, setup, recover } from "./helpers.mjs";

const terra = new LCDClient({
    URL: 'https://tequila-lcd.terra.dev',
    chainID: 'tequila-0004'
})
const wallet = await recover(terra, process.env.TEST_MAIN);
let lpContractAddress = await deploy(terra, wallet);

const initialAssets = [
    { denom: "uluna", borrow_slope: "0.1", loan_to_value: "0.5" },
    { denom: "uusd", borrow_slope: "0.5", loan_to_value: "0.8" }
];

await setup(terra, wallet, lpContractAddress, { initialAssets });
