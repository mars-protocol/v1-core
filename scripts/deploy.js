import 'dotenv/config.js';
import {
  recover,
  uploadContract,
  instantiateContract,
  deployContract,
  queryContract,
  executeContract,
  setupRedBank,
} from "./helpers.mjs";
import { LCDClient, LocalTerra } from "@terra-money/terra.js";

async function main() {
  let terra;
  let wallet;
  const isTestnet = process.env.NETWORK === "testnet"

  if (isTestnet) {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = await recover(terra, process.env.TEST_MAIN);
    console.log(`Wallet address from seed: ${wallet.key.accAddress}`);
  } else {
    terra = new LocalTerra();
    wallet = terra.wallets.test1;
  }

  let councilInitMsg;
  let stakingInitMsg;
  let insuranceFundInitMsg;
  let redBankInitMsg;
  let initialRedBankAssets = []

  if (isTestnet) {
    let terraswap_factory_address = "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf"

    councilInitMsg = {
      "config": {
        "address_provider_address": undefined,

        "proposal_voting_period": 1000,
        "proposal_effective_delay": 150,
        "proposal_expiration_period": 3000,
        "proposal_required_deposit": "100000000",
        "proposal_required_quorum": "0.1",
        "proposal_required_threshold": "0.05"
      }
    };

    stakingInitMsg = {
      "config": {
        "owner": undefined,
        "address_provider_address": undefined,
        "terraswap_factory_address": terraswap_factory_address,
        "terraswap_max_spread": "0.05",
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }

    insuranceFundInitMsg = {
      "owner": undefined,
      "terraswap_factory_address": terraswap_factory_address,
      "terraswap_max_spread": "0.05",
    }

    redBankInitMsg = {
      "config": {
        "owner": wallet.key.accAddress,
        "address_provider_address": undefined,
        "insurance_fund_fee_share": "0.1",
        "treasury_fee_share": "0.2",
        "ma_token_code_id": undefined,
        "close_factor": "0.5"
      }
    }

    // find contract addresses of CW20's here: https://github.com/terra-project/assets/blob/master/cw20/tokens.json
    initialRedBankAssets = [
      {
        denom: "uluna",
        initial_borrow_rate: "0.1",
        min_borrow_rate: "0.03",
        max_borrow_rate: "0.6",
        max_loan_to_value: "0.7",
        reserve_factor: "0.3",
        maintenance_margin: "0.75",
        liquidation_bonus: "0.05",
        kp: "0.5",
        optimal_utilization_rate: "0.75",
        kp_augmentation_threshold: "0.1",
        kp_multiplier: "0.5"
      },
      {
        denom: "uusd",
        initial_borrow_rate: "0.1",
        min_borrow_rate: "0.01",
        max_borrow_rate: "0.8",
        max_loan_to_value: "0.8",
        reserve_factor: "0.3",
        maintenance_margin: "0.825",
        liquidation_bonus: "0.05",
        kp: "0.5",
        optimal_utilization_rate: "0.75",
        kp_augmentation_threshold: "0.1",
        kp_multiplier: "0.5"
      },
      {
        symbol: "ANC",
        contract_addr: "terra1747mad58h0w4y589y3sk84r5efqdev9q4r02pc",
        initial_borrow_rate: "0.1",
        min_borrow_rate: "0.03",
        max_borrow_rate: "0.6",
        max_loan_to_value: "0.5",
        reserve_factor: "0.3",
        maintenance_margin: "0.55",
        liquidation_bonus: "0.1",
        kp: "0.5",
        optimal_utilization_rate: "0.75",
        kp_augmentation_threshold: "0.1",
        kp_multiplier: "0.5"
      },
      {
        symbol: "MIR",
        contract_addr: "terra10llyp6v3j3her8u3ce66ragytu45kcmd9asj3u",
        initial_borrow_rate: "0.1",
        min_borrow_rate: "0.03",
        max_borrow_rate: "0.6",
        max_loan_to_value: "0.5",
        reserve_factor: "0.3",
        maintenance_margin: "0.55",
        liquidation_bonus: "0.1",
        kp: "0.5",
        optimal_utilization_rate: "0.75",
        kp_augmentation_threshold: "0.1",
        kp_multiplier: "0.5"
      },
    ]
  } else {
    councilInitMsg = {
      "config": {
        "address_provider_address": undefined,

        "proposal_voting_period": 1000,
        "proposal_effective_delay": 150,
        "proposal_expiration_period": 3000,
        "proposal_required_deposit": "100000000",
        "proposal_required_quorum": "0.1",
        "proposal_required_threshold": "0.05"
      }
    };

    stakingInitMsg = {
      "config": {
        "owner": undefined,
        "address_provider_address": undefined,
        "terraswap_factory_address": undefined,
        "terraswap_max_spread": "0.05",
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }

    insuranceFundInitMsg = {
      "owner": undefined,
      "terraswap_factory_address": undefined,
      "terraswap_max_spread": "0.05",
    }

    redBankInitMsg = {
      "config": {
        "owner": wallet.key.accAddress,
        "address_provider_address": undefined,
        "insurance_fund_fee_share": "0.1",
        "treasury_fee_share": "0.2",
        "ma_token_code_id": undefined,
        "close_factor": "0.5"
      }
    }
  }

  /*************************************** Deploy Address Provider Contract *****************************************/
  console.log("Deploying Address Provider...");
  const addressProviderContractAddress = "terra1te2nuy58y28axrxfwpq8acnh6xmwsntn9gvf2s" // TODO await deployContract(terra, wallet, './artifacts/address_provider.wasm', { "owner": wallet.key.accAddress })
  console.log("Address Provider Contract Address: " + addressProviderContractAddress);

  /*************************************** Deploy Council Contract *****************************************/
  console.log("Deploying council...");
  councilInitMsg.config.address_provider_address = addressProviderContractAddress
  const councilContractAddress = "terra1pe7v32flg42thzwhjlsprljtz0vxrkchafcn0m" // TODO await deployContract(terra, wallet, './artifacts/council.wasm', councilInitMsg);
  console.log("Council Contract Address: " + councilContractAddress);

  /**************************************** Deploy Staking Contract *****************************************/
  console.log("Deploying Staking...");
  stakingInitMsg.config.owner = councilContractAddress
  stakingInitMsg.config.address_provider_address = addressProviderContractAddress
  const stakingContractAddress = "terra14x8g3q557de7sdjur7zdr695ddntq9pzmnle23" // TODO await deployContract(terra, wallet, './artifacts/staking.wasm', stakingInitMsg);
  console.log("Staking Contract Address: " + stakingContractAddress);

  /************************************* Deploy Insurance Fund Contract *************************************/
  console.log("Deploying Insurance Fund...");
  insuranceFundInitMsg.owner = councilContractAddress
  const insuranceFundContractAddress = "terra1k56clmduszx3s5d4yz2ek0zx83986t9anl0zrl" // TODO await deployContract(terra, wallet, './artifacts/insurance_fund.wasm', insuranceFundInitMsg)
  console.log("Insurance Fund Contract Address: " + insuranceFundContractAddress);

  /**************************************** Deploy Treasury Contract ****************************************/
  console.log("Deploying Treasury...");
  const treasuryContractAddress = "terra1hmaz3aqshvhesmv8xu6wuw8v9stv83yq7qz6zg" // TODO await deployContract(terra, wallet, './artifacts/treasury.wasm', { "owner": councilContractAddress })
  console.log("Treasury Contract Address: " + treasuryContractAddress);

  /**************************************** Deploy Incentives Contract ****************************************/
  console.log("Deploying Incentives...");
  const incentivesContractAddress = "terra1e6fm7aw5gr3qjh5qhe33p6gp5at8ttl6h4aard" // TODO await deployContract(terra, wallet, './artifacts/incentives.wasm', { "owner": councilContractAddress, "address_provider_address": addressProviderContractAddress })
  console.log("Incentives Contract Address: " + incentivesContractAddress);

  /************************************* Upload cw20 Token Contract *************************************/
  console.log("Uploading cw20 token contract");
  const cw20TokenCodeId = 6074 // TODO await uploadContract(terra, wallet, './artifacts/cw20_token.wasm');
  console.log(`Uploaded cw20 token contract, code: ${cw20TokenCodeId}`);

  /************************************* Instantiate Mars Token Contract *************************************/
  console.log("Deploying Mars token...");
  const marsTokenContractAddress = "terra18vkz2qmaghzvvxpzl2szk0000rmtrzxxekvrp0" // TODO
  // const marsTokenContractAddress = await instantiateContract(terra, wallet, cw20TokenCodeId, {
  //   "name": "Mars token",
  //   symbol: "Mars",
  //   decimals: 6,
  //   initial_balances: isTestnet ? [{ "address": wallet.key.accAddress, "amount": "1000000000000" }] : [],
  //   mint: { "minter": councilContractAddress },
  // });
  console.log("Mars Token Contract Address: " + marsTokenContractAddress);

  const balanceResponse = await queryContract(terra, marsTokenContractAddress, { "balance": { "address": wallet.key.accAddress } })
  console.log(`Balance of adress ${wallet.key.accAddress}: ${balanceResponse.balance / 1e6} Mars`)

  /************************************* Instantiate xMars Token Contract *************************************/
  console.log("Deploying xMars token...");
  const xMarsTokenContractAddress = "terra1uherj35jkvlvsgguxywk0r8sx3j5wvcm23ht7u" // TODO
  // const xMarsTokenContractAddress = await instantiateContract(terra, wallet, cw20TokenCodeId, {
  //   "name": "xMars token",
  //   symbol: "xMars",
  //   decimals: 6,
  //   initial_balances: [],
  //   mint: { "minter": stakingContractAddress },
  // });
  console.log("xMars Token Contract Address: " + xMarsTokenContractAddress);

  /************************************* Upload ma_token Token Contract *************************************/
  console.log("Uploading ma_token contract");
  const maTokenCodeId = 6075 // TODO await uploadContract(terra, wallet, './artifacts/ma_token.wasm')
  console.log(`Uploaded ma_token contract code: ${maTokenCodeId}`);

  /************************************* Deploy Red Bank Contract *************************************/
  console.log("Deploying Red Bank...");
  redBankInitMsg.config.address_provider_address = addressProviderContractAddress
  redBankInitMsg.config.ma_token_code_id = maTokenCodeId
  const redBankContractAddress = await deployContract(terra, wallet, './artifacts/red_bank.wasm', redBankInitMsg)
  console.log(`Red Bank Contract Address: ${redBankContractAddress}`);

  /**************************************** Update Config in Address Provider Contract *****************************************/
  console.log('Setting addresses in address provider')
  await executeContract(terra, wallet, addressProviderContractAddress, {
    "update_config": {
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
  })
  addressProviderQueryResponse = await queryContract(terra, addressProviderContractAddress, { "config": {} })
  console.log("Address Provider config successfully setup: ", addressProviderQueryResponse)

  /************************************* Setup Initial Liquidity Pools **************************************/
  await setupRedBank(terra, wallet, redBankContractAddress, { initialRedBankAssets });
  // Once initial assets initialized, set the owner of Red Bank to be Council rather than EOA
  redBankInitMsg.owner = councilContractAddress
  await executeContract(terra, wallet, redBankContractAddress, { "update_config": redBankInitMsg })
}

main().catch(console.log);
