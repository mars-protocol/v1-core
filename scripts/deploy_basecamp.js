import 'dotenv/config.js';
import {deployBasecampContract, recover} from "./helpers.mjs";
import {LCDClient, LocalTerra} from "@terra-money/terra.js";

async function main() {
  let terra;
  let wallet;

  if (process.env.NETWORK === "testnet") {
    terra = new LCDClient({
      URL: 'https://tequila-lcd.terra.dev',
      chainID: 'tequila-0004'
    })

    wallet = await recover(terra, process.env.TEST_MAIN);
    console.log(wallet.key.accAddress);
  } else {
    terra = new LocalTerra();
    wallet = terra.wallets.test1;
  }

  let basecampConfig;

  if (process.env.NETWORK === "testnet") {
    basecampConfig = {
      "cw20_code_id": undefined,
      "cooldown_duration": 300,
      "unstake_window": 300,
      "proposal_voting_period": 1000,
      "proposal_effective_delay": 150,
      "proposal_expiration_period": 3000,
      "proposal_required_deposit": "100000000",
      "proposal_required_quorum": "10",
      "proposal_required_threshold": "5"
    };
  } else {
    basecampConfig = {
      "cw20_code_id": undefined,
      "cooldown_duration": 1,
      "unstake_window": 30,
      "proposal_voting_period": 1000,
      "proposal_effective_delay": 150,
      "proposal_expiration_period": 3000,
      "proposal_required_deposit": "100000000",
      "proposal_required_quorum": "10",
      "proposal_required_threshold": "5"
    };
  }

  await deployBasecampContract(terra, wallet, basecampConfig);
}

main().catch(console.log);
