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
5. Run `node scripts/deploy.js` to deploy and instantiate the smart contracts

### Linting
1. Format: `cargo fmt`.
2. Lint: `cargo clippy --tests --all-features -- -D warnings`

### Testing
#### Unit tests
- Run `cargo unit-test` inside a package to run specific package tests
- Run `cargo test` on root directory to run all tests

#### Integration tests
Run `node scripts/liquidity_pool_integration_tests.js`

Env variables:
- `DEBUG`: when set to 1, more verbose logs are printed.
- `CACHE`: use a cache source to store and reuse references to local terra. (Only `redis` is supported)

### Generating a whitelist.json

1. Create a .env file in the top level of of the directory if doesn't already exist
2. Add the env variable NETWORK=[network_to_generate_from_e.g._NETWORK=testnet]
3. Add the env variable REDBANK_ADDRESS=[your_deployed_red_bank_contract_address]
4. Run `node scripts/whitelist.js`
5. Check the artifacts folder for [NETWORK].json output
