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

