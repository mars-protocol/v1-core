import 'dotenv/config.js';
import {deployLiquidityPool, setupLiquidityPool} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";

const terra = new LocalTerra();
const wallet = terra.wallets.test1;

let returned_vals = await deployLiquidityPool(terra, wallet);

const initialAssets = [
  {denom: "uluna", borrow_slope: "0.1", loan_to_value: "0.5"},
  {denom: "uusd", borrow_slope: "0.5", loan_to_value: "0.8"}
];

const initialDeposits = [
  {
    account: terra.wallets.test1,
    assets: {"uluna": 6000000000, "uusd": 5000000000}
  },
  {
    account: terra.wallets.test2,
    assets: { "uluna": 7000000000, "uusd": 8000000000}
  }
]

const initialBorrows = [
  {
    account: terra.wallets.test1,
    assets: { "uluna": 3500000, "uusd": 4000000}
  },
  {
    account: terra.wallets.test2,
    assets: {"uluna": 3000000, "uusd": 2500000}
  },
]

await setupLiquidityPool(terra, wallet, returned_vals.lpAddress, {initialAssets, initialDeposits, initialBorrows});
