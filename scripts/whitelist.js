import {deploy, queryContract, setup} from "./helpers.mjs";
import {LocalTerra} from "@terra-money/terra.js";
import {writeFileSync} from 'fs';

const terra = new LocalTerra();
const wallet = terra.wallets.test1;
// const lpContractAddress = await deploy(terra, wallet);
const lpContractAddress = "terra12jc40azjta9xrspl5pumxp97xwecyxctza5aqm";
//
// const initialAssets = ["uluna", "uusd", "umnt", "ukrw", "usdr"];
// await setup(terra, wallet, lpContractAddress, {initialAssets});

const reservesListResult = await queryContract(terra, lpContractAddress, {"reserves_list": {}});
const { reserves_list } = reservesListResult;

const reserveToTokenInfo = {};

for (let reserve of reserves_list) {
  const tokenInfoQuery = {"token_info": {}};
  const tokenInfoResult = await queryContract(terra, reserve, tokenInfoQuery);
  reserveToTokenInfo[reserve] = tokenInfoResult;
  console.log(tokenInfoResult);
}

console.log(reserveToTokenInfo);
const output = {};
output.contracts = {lpContractAddress};
output.whitelist = reserveToTokenInfo;

const json = JSON.stringify(output);

writeFileSync('whitelist.json', json, {'encoding': 'utf8'});
