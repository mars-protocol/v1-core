# Mars
## Scripts

### Build

```
./scripts/build_artifacts.sh
./scripts/build_schema.sh
```

### Linting

```
cargo fmt
cargo clippy --tests --all-features -- -D warnings
```

### TypeScript and JavaScript scripts

Must be run from the `scripts` directory.

Setup:

```
cd scripts
npm install
```

TypeScript scripts must be executed with `ts-node` using:

```
node --loader ts-node/esm script.ts
```

An alias can be added to the shell profile:

```
# bash
echo 'alias ts-node="node --loader ts-node/esm"' >> ~/.bashrc

# zsh
echo 'alias ts-node="node --loader ts-node/esm"' >> ~/.zshrc
```

Some scripts require LocalTerra to be running:

```
git clone https://github.com/terra-money/LocalTerra.git
cd LocalTerra
docker compose up
```

Adjust the `timeout_*` config items in `LocalTerra/config/config.toml` to `250ms` to make the test run faster:

```
sed -E -I .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/250ms/' config/config.toml
```

### Deploy

1. Build the smart contracts by running `./scripts/build_artifacts.sh`
2. Run `npm install` to install dependencies
3. Create a .env file in the top level of of the directory if doesn't already exist
4. Add the env variable TEST_MAIN=[your_deploying_wallets_mnemonic_key]
5. Run `node scripts/deploy.js` to deploy and instantiate the smart contracts

### Testing
#### Unit tests

```
# inside a package to run specific package tests
cargo unit-test

# in the root directory to run all tests
cargo test
```

#### Integration tests

```
cd scripts
ts-node tests/insurance_fund.ts
```

Env variables:
- `DEBUG`: when set to 1, more verbose logs are printed.
- `CACHE`: use a cache source to store and reuse references to local terra. (Only `redis` is supported)

### Generating a whitelist.json

1. Create a .env file in the top level of of the directory if doesn't already exist
2. Add the env variable NETWORK=[network_to_generate_from_e.g._NETWORK=testnet]
3. Add the env variable LP_ADDRESS=[your_deployed_lp_contract_address]
4. Run `node scripts/whitelist.js`
5. Check the artifacts folder for [NETWORK].json output
