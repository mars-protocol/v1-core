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
    assets: {"uluna": 6000000000, "uusd": 5000000000, "umnt": 7000000000}
  },
  {
    account: terra.wallets.test2,
    assets: {"ukrw": 7000000000, "usdr": 8000000000}
  }
]

const initialBorrows = [
  {
    account: terra.wallets.test1,
    assets: {"ukrw": 3500000000, "usdr": 4000000000}
  },
  {
    account: terra.wallets.test2,
    assets: {"uluna": 3000000000, "uusd": 2500000000, "umnt": 3500000000}
  },
]

await setup(terra, wallet, lpContractAddress, {initialAssets, initialDeposits, initialBorrows});

