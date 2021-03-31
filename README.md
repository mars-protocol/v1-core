# Mars

### Build

1. Run `./scripts/build_artifacts.sh`

### Deploy Dev

1. Build the smart contracts by running `./scripts/build_artifacts.sh`
2. Run `yarn install` to install dependencies
3. Ensure you have LocalTerra running
4. Modify your terra instance and wallet as desired in `deploy_local.js`
5. Run `node scripts/deploy_local.js` to deploy and instantiate the smart contracts

### Deploy Testnet

1. Build the smart contracts by running `./scripts/build_artifacts.sh`
2. Run `yarn install` to install dependencies
3. Create a .env file in the top level of of the directory if doesn't already exist
4. Add the env variable TEST_MAIN=[your_deploying_wallets_mnemonic_key]
5. Run `node scripts/deploy_testnet.js` to deploy and instantiate the smart contracts

### Testing

1. Run `node scripts/integration_tests.js`

### Generating a whitelist.json

1. Create a .env file in the top level of of the directory if doesn't already exist
2. If generating whitelist for testnet, also add the env variable NETWORK=testnet, localTerra doesn't require any NETOWORK env variable
3. Add the env variable LP_ADDRESS=[your_deployed_lp_contract_address]
4. Run `node scripts/whitelist.js`
5. Check the artifacts folder for whitelist.json output
