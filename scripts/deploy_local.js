import {deploy, setup} from "./helpers.mjs";
import { LocalTerra } from "@terra-money/terra.js";

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
let lpContractAddress = await deploy(terra, wallet);

const initialAssets = ["luna", "usd", "mnt", "krw", "sdt"];
await setup(terra, wallet, lpContractAddress, {initialAssets});


