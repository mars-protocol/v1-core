import 'dotenv/config.js';
import {
  recover,
  queryContract,
  executeContract,
  deployBasecampContract,
  deployStakingContract,
  deployInsuranceFundContract,
  deployTreasuryContract,
  deployLiquidityPool,
  setupLiquidityPool,
} from "./helpers.mjs";
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

  // TODO, abstract these configs to a separate file...
  let basecampConfig;
  let stakingConfig;
  let insuranceFundConfig;
  let lpConfig;

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
        "terraswap_factory_address": "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf",
        "terraswap_max_spread": "0.05",
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }

    insuranceFundConfig = {
      "terraswap_factory_address": "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf",
      "terraswap_max_spread": "0.05",
    }

    lpConfig = {
      "config": {
        "treasury_contract_address": undefined,
        "insurance_fund_contract_address": undefined,
        "staking_contract_address": undefined,
        "insurance_fund_fee_share": "0.1",
        "treasury_fee_share": "0.2",
        "ma_token_code_id": undefined,
        "close_factor": "0.5"
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
        "terraswap_factory_address": undefined,
        "terraswap_max_spread": "0.05",
        "cooldown_duration": 10,
        "unstake_window": 300,
      }
    }

    insuranceFundConfig = {
      "terraswap_factory_address": undefined,
      "terraswap_max_spread": "0.05",
    }

    lpConfig = {
      "config": {
        "treasury_contract_address": undefined,
        "insurance_fund_contract_address": undefined,
        "staking_contract_address": undefined,
        "insurance_fund_fee_share": "0.1",
        "treasury_fee_share": "0.2",
        "ma_token_code_id": undefined,
        "close_factor": "0.5"
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
  const insuranceFundContractAddress = await deployInsuranceFundContract(terra, wallet, insuranceFundConfig)
  await executeContract(terra, wallet, insuranceFundContractAddress, { "update_config": { "owner": basecampContractAddress } })
  const insuranceFundQueryResponse = await queryContract(terra, insuranceFundContractAddress, { "config": {} })
  console.log("Insurance fund config successfully updated to have owner of: ", insuranceFundQueryResponse.owner)

  /**************************************** Deploy Treasury Contract ****************************************/
  const treasuryContractAddress = await deployTreasuryContract(terra, wallet)

  /************************************* Deploy Liquidity Pool Contract *************************************/
  lpConfig.config.treasury_contract_address = treasuryContractAddress
  lpConfig.config.insurance_fund_contract_address = insuranceFundContractAddress
  lpConfig.config.staking_contract_address = stakingContractAddress
  lpConfig.config.ma_token_code_id = cw20CodeId
  const lpContractAddress = await deployLiquidityPool(terra, wallet, lpConfig)
  // TODO, owner of lp contract should be set to basecamp

  /************************************* Setup Initial Liquidity Pools **************************************/

  // find contract addresses of CW20's here: https://github.com/terra-project/assets/blob/master/cw20/tokens.json
  // TODO this config should be moved into env specific variable?
  const initialAssets = [
    { denom: "uluna", borrow_slope: "0.1", loan_to_value: "0.5", reserve_factor: "0.3", liquidation_threshold: "0.525", liquidation_bonus: "0.1" },
    { denom: "uusd", borrow_slope: "0.5", loan_to_value: "0.8", reserve_factor: "0.3", liquidation_threshold: "0.825", liquidation_bonus: "0.1" },
    { symbol: "ANC", contract_addr: "terra1747mad58h0w4y589y3sk84r5efqdev9q4r02pc", borrow_slope: "0.1", loan_to_value: "0.5", reserve_factor: "0.3", liquidation_threshold: "0.525", liquidation_bonus: "0.1" },
    { symbol: "MIR", contract_addr: "terra10llyp6v3j3her8u3ce66ragytu45kcmd9asj3u", borrow_slope: "0.1", loan_to_value: "0.5", reserve_factor: "0.3", liquidation_threshold: "0.525", liquidation_bonus: "0.1" },
  ];
  await setupLiquidityPool(terra, wallet, lpContractAddress, { initialAssets });

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
