import 'dotenv/config.js';
import { recover, queryContract, executeContract, deployBasecampContract, deployStakingContract, deployInsuranceFundContract } from "./helpers.mjs";
import { LCDClient, LocalTerra } from "@terra-money/terra.js";

async function main() {
  let terra;
  let wallet;

  if (process.env.NETWORK === "testnet") {
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

  let basecampConfig;
  let stakingConfig;

  if (process.env.NETWORK === "testnet") {
    basecampConfig = {
      "cw20_code_id": undefined,
      "config": {
        "proposal_voting_period": 40,
        "proposal_effective_delay": 20,
        "proposal_expiration_period": 300,
        "proposal_required_deposit": "100000000",
        "proposal_required_quorum": "0.1",
        "proposal_required_threshold": "0.05"
      }
    };

    stakingConfig = {
      "cw20_code_id": undefined,
      "config": {
        "mars_token_address": undefined,
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }
  } else {
    basecampConfig = {
      "cw20_code_id": undefined,
      "config": {
        "proposal_voting_period": 1000,
        "proposal_effective_delay": 150,
        "proposal_expiration_period": 3000,
        "proposal_required_deposit": "100000000",
        "proposal_required_quorum": "0.1",
        "proposal_required_threshold": "0.05"
      }
    };

    stakingConfig = {
      "cw20_code_id": undefined,
      "config": {
        "mars_token_address": undefined,
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }
  }

  /*************************************** Deploy Basecamp Contract *****************************************/
  const { basecampContractAddress, cw20CodeId } = await deployBasecampContract(terra, wallet, basecampConfig);
  let basecampQueryResponse = await queryContract(terra, basecampContractAddress, { "config": {} })

  /**************************************** Deploy Staking Contract *****************************************/
  stakingConfig.config.mars_token_address = basecampQueryResponse.mars_token_address
  const stakingContractAddress = await deployStakingContract(terra, wallet, stakingConfig);
  const stakingQueryResponse = await queryContract(terra, stakingContractAddress, { "config": {} })
  
  /************************************* Deploy Insurance Fund Contract *************************************/
  const insuranceFundContractAddress = await deployInsuranceFundContract(terra, wallet)
  await executeContract(terra, wallet, insuranceFundContractAddress, { "update_config": { "owner": basecampContractAddress } })
  const insuranceFundQueryResponse = await queryContract(terra, insuranceFundContractAddress, { "config": {} })
  console.log("Insurance fund config successfully updated to have owner of: ", insuranceFundQueryResponse.owner)

  /**************************************** Setup Basecamp Contract *****************************************/
  console.log('Setting staking contract addresses in basecamp...')
  await executeContract(terra, wallet, basecampContractAddress, {
    "set_contract_addresses": {
      xmars_token_address: stakingQueryResponse.xmars_token_address,
      staking_contract_address: stakingContractAddress,
      insurance_fund_contract_address: insuranceFundContractAddress
    }
  })
  basecampQueryResponse = await queryContract(terra, basecampContractAddress, { "config": {} })
  console.log("Basecamp config successfully setup: ", basecampQueryResponse)

  /************************************* Mint Mars to Contract Owner ****************************************/
  if (process.env.NETWORK !== "mainnet") {
    console.log(`Minting MARS tokens to ${wallet.key.accAddress}`)
    await executeContract(terra, wallet, basecampContractAddress, { "mint_mars": { "recipient": wallet.key.accAddress, "amount": "1000000000000" } })
    const balanceResponse = await queryContract(terra, basecampQueryResponse.mars_token_address, { "balance": { "address": wallet.key.accAddress } })
    console.log(`Balance of wallet ${wallet.key.accAddress}: ${balanceResponse.balance / 1e6} MARS`)
  }
}

main().catch(console.log);
