import 'dotenv/config.js';
import {deploy, queryContract, setup} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
let lpContractAddress = await deploy(terra, wallet);

const initialAssets = ["uluna", "uusd", "umnt", "ukrw", "usdr"];
const initialDeposits = [
  {
    account: terra.wallets.test1,
    assets: {"uluna": 1000, "uusd": 2000, "umnt": 3000, "ukrw": 4000, "usdr": 5000}
  },
  {
    account: terra.wallets.test2,
    assets: {"uluna": 6000, "uusd": 7000, "umnt": 8000, "ukrw": 9000, "usdr": 10000}
  }
]

await setup(terra, wallet, lpContractAddress, {initialAssets, initialDeposits});
