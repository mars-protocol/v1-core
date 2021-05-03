import 'dotenv/config.js';
import { deployLiquidityPool, setupLiquidityPool, recover } from "./helpers.mjs";
import { LCDClient } from "@terra-money/terra.js";

const terra = new LCDClient({
    URL: 'https://tequila-lcd.terra.dev',
    chainID: 'tequila-0004'
})
const wallet = await recover(terra, process.env.TEST_MAIN);

let returned_vals = await deployLiquidityPool(terra, wallet);

// https://github.com/terra-project/assets/blob/master/cw20/tokens.json
const initialAssets = [
    { denom: "uluna", borrow_slope: "0.1", loan_to_value: "0.5" },
    { denom: "uusd", borrow_slope: "0.5", loan_to_value: "0.8" },
    { symbol: "ANC", contract_addr: "terra1747mad58h0w4y589y3sk84r5efqdev9q4r02pc", borrow_slope: "0.1", loan_to_value: "0.5" },
    { symbol: "MIR", contract_addr: "terra10llyp6v3j3her8u3ce66ragytu45kcmd9asj3u", borrow_slope: "0.1", loan_to_value: "0.5" },
];

await setupLiquidityPool(terra, wallet, returned_vals.lpAddress, { initialAssets });