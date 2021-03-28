# Mars

### Build

1. Run `./scripts/build_artifacts.sh`

### Deploy Dev

1. Build the smart contracts by running `./scripts/build_artifacts.sh`
2. Run `yarn install` to install dependencies
3. Ensure you have LocalTerra running
4. Modify your terra instance and wallet as desired in `deploy_local.js`
5. Run `node scripts/deploy_local.js` to deploy and instantiate the smart contracts

### Testing

1. Run `node scripts/integration_tests.js`

### Generating a whitelist.json

1. Create a .env file in the top level of of the directory with LP_ADDRESS=[your_deployed_lp_contract_address]
2. Run `node scripts/whitelist.js`
3. Check the artifacts folder for whitelist.json output
