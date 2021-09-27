# Mars
## Scripts

### Build

```
./scripts/build_artifacts.sh
./scripts/build_schema.sh
```

### Linting

```
rustup component add rustfmt
cargo fmt

rustup install nightly
rustup component add clippy
cargo +nightly clippy --tests --all-features -- -D warnings
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
node --loader ts-node/esm <script>.ts
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

```
# build the smart contracts
./scripts/build_artifacts.sh

cd scripts
npm install

# set the deploying wallet
echo "TEST_MAIN=<MNEMONIC_OF_YOUR_DEPLOYING_WALLET>" >> .env

# set the network, defaults to LocalTerra if unset
echo "NETWORK=bombay" >> .env

# ensure the deploy_config.ts has a cw20_code_id specified for above network

node --loader ts-node/esm deploy.ts
```

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
node --loader ts-node/esm tests/<test>.ts
```

Required environment variables (can be set in `scripts/.env`):

```sh
CW_PLUS_ARTIFACTS_PATH # path to cw-plus artifacts
TERRASWAP_ARTIFACTS_PATH # path to terraswap artifacts
BLOCK_TIME_MILLISECONDS # targetted block time in ms, which is set in `LocalTerra/config/config.toml`
```

### Generating a whitelist.json

1. Create a .env file in the top level of the scripts directory if doesn't already exist
2. Add the env variable NETWORK=[network_to_generate_from_e.g._NETWORK=bombay]
3. Add the env variable REDBANK_ADDRESS=[your_deployed_red_bank_contract_address]
4. Run `node --loader ts-node/esm whitelist.ts`
5. Check the whitelists folder for [NETWORK].json output
