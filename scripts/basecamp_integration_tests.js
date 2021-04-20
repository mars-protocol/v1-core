import {LocalTerra} from "@terra-money/terra.js";
import {queryContract} from "./helpers.mjs";
import {deployBasecampContract} from "./helpers.mjs";

let terra = new LocalTerra();
let wallet = terra.wallets.test1;

let cooldownDuration = 1;
let unstakeWindow = 30;

let basecampContractAddress = await deployBasecampContract(terra, wallet, cooldownDuration, unstakeWindow);

// query config for mars and xmars contracts
let queryConfigMsg = {"config": {}};
let {mars_token_address, xmars_token_address} = await terra.wasm.contractQuery(basecampContractAddress, queryConfigMsg);

// check token symbols
console.log("### Testing Token Info...");
let queryTokenInfoMsg = {"token_info": {}};
let {symbol: marsSymbol} = await queryContract(terra, mars_token_address, queryTokenInfoMsg);
if (marsSymbol !== "Mars") {
  throw new Error(`Incorrect symbol ${marsSymbol}, expected Mars`);
}

let {symbol: xMarsSymbol} = await queryContract(terra, xmars_token_address, queryTokenInfoMsg);
if (xMarsSymbol !== "xMars") {
  throw new Error(`Incorrect symbol ${xMarsSymbol}, expected xMars`);
}

// check minter for both contracts is the basecamp contract
console.log("### Testing Minter...");
let queryMinterMsg = {"minter": {}};
let {minter: marsMinter} = await queryContract(terra, mars_token_address, queryMinterMsg);
if (marsMinter !== basecampContractAddress) {
  throw new Error(`mars minter is ${marsMinter}, expected ${basecampContractAddress}`);
}

let {minter: xMarsMinter} = await queryContract(terra, xmars_token_address, queryMinterMsg);
if (xMarsMinter !== basecampContractAddress) {
  throw new Error(`xMars minter is ${xMarsMinter}, expected ${basecampContractAddress}`);
}

console.log("Testing Complete");

