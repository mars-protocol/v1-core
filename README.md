# Mars

### Build
1. Run `./scripts/build_artifacts.sh`

### Deploy Dev
1. Build the smart contracts by running `./scripts/build_artifacts.sh`
2. Run `cd scripts && yarn install` to install dependencies
3. Ensure you have LocalTerra running
4.  Run `node scripts/deploy_local.mjs` to deploy and instantiate the smart contracts

### Testing
1. Run `cd scripts && node integration_tests.js`

