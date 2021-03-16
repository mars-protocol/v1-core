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
    assets: {"uluna": 6000000000, "uusd": 5000000000, "umnt": 7000000000, "ukrw": 3000000000, "usdr": 8000000000}
  },
  {
    account: terra.wallets.test2,
    assets: {"uluna": 2000000000, "uusd": 9000000000, "umnt": 4000000000, "ukrw": 7000000000, "usdr": 1000000000}
  }
]

await setup(terra, wallet, lpContractAddress, {initialAssets, initialDeposits});
