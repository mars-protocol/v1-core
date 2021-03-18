import 'dotenv/config.js';
import {migrate, executeContract} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
const contractAddress = 'terra1svj7hv4lmct57z9l8gckp38gwepvxwazplqtqp';
console.log('migrating...');
let res = await migrate(terra, wallet, contractAddress);
console.log(res);

let initAssetMsg = {"init_asset": {"denom": "uusd"}};
let res = await executeContract(terra, wallet, contractAddress, initAssetMsg);
console.log(res);

