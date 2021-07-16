import 'dotenv/config.js';
import {
  deployContract,
  executeContract,
  instantiateContract,
  queryContract,
  recover,
  setTimeoutDuration,
  setupRedBank,
  uploadContract,
} from "./helpers.js";
import { LCDClient, LocalTerra } from "@terra-money/terra.js";
import { Config, testnet, local } from "./deploy_configs.js"
import { join } from "path"

// consts

const MARS_ARTIFACTS_PATH = "../artifacts"

// main

async function main() {
  let terra;
  let wallet;
  const isTestnet = process.env.NETWORK === "testnet"

  if (isTestnet) {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = recover(terra, process.env.TEST_MAIN!);
    console.log(`Wallet address from seed: ${wallet.key.accAddress}`);
  } else {
    terra = new LocalTerra();
    wallet = terra.wallets.test1;
    setTimeoutDuration(0)
  }

  let deployConfig: Config;
  if (isTestnet) {
    deployConfig = testnet
  } else {
    deployConfig = local
  }

  /*************************************** Deploy Address Provider Contract *****************************************/
  console.log("Deploying Address Provider...");
  const addressProviderContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'address_provider.wasm'), { "owner": wallet.key.accAddress })
  console.log("Address Provider Contract Address: " + addressProviderContractAddress);

  /*************************************** Deploy Council Contract *****************************************/
  console.log("Deploying council...");
  deployConfig.councilInitMsg.config.address_provider_address = addressProviderContractAddress
  const councilContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'council.wasm'), deployConfig.councilInitMsg);
  console.log("Council Contract Address: " + councilContractAddress);

  /**************************************** Deploy Staking Contract *****************************************/
  console.log("Deploying Staking...");
  // TODO fix `factory_contract_address` in LocalTerra
  deployConfig.stakingInitMsg.config.owner = councilContractAddress
  deployConfig.stakingInitMsg.config.address_provider_address = addressProviderContractAddress
  const stakingContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'staking.wasm'), deployConfig.stakingInitMsg);
  console.log("Staking Contract Address: " + stakingContractAddress);

  /************************************* Deploy Insurance Fund Contract *************************************/
  console.log("Deploying Insurance Fund...");
  deployConfig.insuranceFundInitMsg.owner = councilContractAddress
  const insuranceFundContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'insurance_fund.wasm'), deployConfig.insuranceFundInitMsg)
  console.log("Insurance Fund Contract Address: " + insuranceFundContractAddress);

  /**************************************** Deploy Treasury Contract ****************************************/
  console.log("Deploying Treasury...");
  const treasuryContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'treasury.wasm'), { "owner": councilContractAddress })
  console.log("Treasury Contract Address: " + treasuryContractAddress);

  /**************************************** Deploy Incentives Contract ****************************************/
  console.log("Deploying Incentives...");
  const incentivesContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'incentives.wasm'), { "owner": councilContractAddress, "address_provider_address": addressProviderContractAddress })
  console.log("Incentives Contract Address: " + incentivesContractAddress);

  /************************************* Upload cw20 Token Contract *************************************/
  console.log("Uploading cw20 token contract");
  const cw20TokenCodeId = await uploadContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'cw20_token.wasm'));
  console.log(`Uploaded cw20 token contract, code: ${cw20TokenCodeId}`);

  /************************************* Instantiate Mars Token Contract *************************************/
  console.log("Deploying Mars token...");
  const marsTokenContractAddress = await instantiateContract(terra, wallet, cw20TokenCodeId, {
    "name": "Mars token",
    symbol: "Mars",
    decimals: 6,
    initial_balances: isTestnet ? [{ "address": wallet.key.accAddress, "amount": "1000000000000" }] : [],
    mint: { "minter": councilContractAddress },
  });
  console.log("Mars Token Contract Address: " + marsTokenContractAddress);

  const balanceResponse = await queryContract(terra, marsTokenContractAddress, { "balance": { "address": wallet.key.accAddress } })
  console.log(`Balance of adress ${wallet.key.accAddress}: ${balanceResponse.balance / 1e6} Mars`)

  /************************************* Instantiate xMars Token Contract *************************************/
  console.log("Deploying xMars token...");
  const xMarsTokenCodeId = await uploadContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'xmars_token.wasm'));
  console.log(`Uploaded xMars token contract, code: ${xMarsTokenCodeId}`);
  const xMarsTokenContractAddress = await instantiateContract(terra, wallet, xMarsTokenCodeId, {
    "name": "xMars token",
    symbol: "xMars",
    decimals: 6,
    initial_balances: [],
    mint: { "minter": stakingContractAddress },
  });
  console.log("xMars Token Contract Address: " + xMarsTokenContractAddress);

  /************************************* Upload ma_token Token Contract *************************************/
  console.log("Uploading ma_token contract");
  const maTokenCodeId = await uploadContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'ma_token.wasm'))
  console.log(`Uploaded ma_token contract code: ${maTokenCodeId}`);

  /************************************* Deploy Red Bank Contract *************************************/
  console.log("Deploying Red Bank...");
  deployConfig.redBankInitMsg.config.owner = wallet.key.accAddress
  deployConfig.redBankInitMsg.config.address_provider_address = addressProviderContractAddress
  deployConfig.redBankInitMsg.config.ma_token_code_id = maTokenCodeId
  const redBankContractAddress = await deployContract(terra, wallet, join(MARS_ARTIFACTS_PATH, 'red_bank.wasm'), deployConfig.redBankInitMsg)
  console.log(`Red Bank Contract Address: ${redBankContractAddress}`);

  /**************************************** Update Config in Address Provider Contract *****************************************/
  console.log('Setting addresses in address provider')
  await executeContract(terra, wallet, addressProviderContractAddress, {
    "update_config": {
      "config": {
        "owner": councilContractAddress,
        "council_address": councilContractAddress,
        "incentives_address": incentivesContractAddress,
        "insurance_fund_address": insuranceFundContractAddress,
        "mars_token_address": marsTokenContractAddress,
        "red_bank_address": redBankContractAddress,
        "staking_address": stakingContractAddress,
        "treasury_address": treasuryContractAddress,
        "xmars_token_address": xMarsTokenContractAddress
      }
    }
  })
  console.log("Address Provider config successfully setup: ", await queryContract(terra, addressProviderContractAddress, { "config": {} }))

  /************************************* Setup Initial Liquidity Pools **************************************/
  await setupRedBank(terra, wallet, redBankContractAddress, { initialAssets: deployConfig.initialAssets });
  console.log("Initial assets setup successfully")

  // Once initial assets initialized, set the owner of Red Bank to be Council rather than EOA
  console.log(`Updating Red Bank to be owned by Council contract ${councilContractAddress}`)
  deployConfig.redBankInitMsg.config.owner = councilContractAddress
  await executeContract(terra, wallet, redBankContractAddress, { "update_config": deployConfig.redBankInitMsg })
  console.log("Red Bank config successfully updated: ", await queryContract(terra, redBankContractAddress, { "config": {} }))
}

main().catch(console.log);
