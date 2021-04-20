import {LocalTerra} from "@terra-money/terra.js";
import {deployContract, uploadContract, queryContract} from "./helpers.mjs";

let terra = new LocalTerra();
let wallet = terra.wallets.test1;

let cooldownDuration = 1;
let unstakeWindow = 30;

console.log("Uploading Cw20 Token Contract...");
let cw20TokenId = await uploadContract(terra, wallet, './artifacts/cw20_token.wasm');

console.log("Deploying Basecamp...")
let initMsg = {"cw20_code_id": cw20TokenId, "cooldown_duration": cooldownDuration.toString(), "unstake_window": unstakeWindow.toString()};
let basecampContractAddress = await deployContract(terra, wallet, './artifacts/basecamp.wasm', initMsg);
console.log(basecampContractAddress);

let queryConfigMsg = {"token_info": {}};
let queryConfigResult = await terra.wasm.contractQuery(basecampContractAddress, queryConfigMsg);
console.log(queryConfigResult);
