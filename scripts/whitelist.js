import 'dotenv/config.js';
import { queryContract } from "./helpers.mjs";
import { LCDClient, LocalTerra } from "@terra-money/terra.js";
import { existsSync, mkdirSync, writeFileSync } from 'fs';

async function main() {
  let terra;
  let lpContractAddress = process.env.REDBANK_ADDRESS;

  if (process.env.NETWORK === "testnet") {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })
  } else {
    terra = new LocalTerra();
  }

  const marketsListResult = await queryContract(terra, lpContractAddress, { "markets_list": {} });
  const { markets_list } = marketsListResult;
  const marketInfo = {};

  for (let market of markets_list) {
    const { denom, ma_token_address } = market;
    const tokenInfoQuery = { "token_info": {} };
    let { decimals } = await queryContract(terra, ma_token_address, tokenInfoQuery);
    marketInfo[ma_token_address] = { denom, decimals }
  }

  const output = {};
  output.contracts = { lpContractAddress };
  output.whitelist = marketInfo;

  const json = JSON.stringify(output);

  const dir = "artifacts/whitelists"
  const fileName = `${process.env.NETWORK || 'localterra'}.json`
  if (!existsSync(dir)) {
    mkdirSync(dir);
  }
  writeFileSync(`${dir}/${fileName}`, json, { 'encoding': 'utf8' });
}

main().catch(err => console.log(err));
