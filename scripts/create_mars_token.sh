# Script to deploy a cw20 token from a multisig account, mint tokens, and migrate the contract.
#
# This script is designed to work with Terra Columbus-4.
#
# Dependencies:
#   - rust
#   - cosmwasm-plus v0.2.0
#   - terracli 58602320d2907814cfccdf43e9679468bb4bd8d3
#   - LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5
#   - docker
#   - jq
#   - Add test accounts to terracli
#
# Test accounts:
#
# terracli keys add test1 --recover
# notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius
#
# terracli keys add test2 --recover
# quality vacuum heart guard buzz spike sight swarm shove special gym robust assume sudden deposit grid alcohol choice devote leader tilt noodle tide penalty
#
# terracli keys add test3 --recover
# symbol force gallery make bulk round subway violin worry mixture penalty kingdom boring survey tool fringe patrol sausage hard admit remember broken alien absorb
#
# terracli keys add test4 --recover
# bounce success option birth apple portion aunt rural episode solution hockey pencil lend session cause hedgehog slender journey system canvas decorate razor catch empty


# HELPERS

function sign {
  terracli tx sign $2 \
    --multisig $multi \
    --from $1 \
    --output-document ${1}sig.json \
    --chain-id $chain_id
}

function multisign-broadcast {
  sign test1 unsignedTx.json
  sign test2 unsignedTx.json

  terracli tx multisign unsignedTx.json multi test1sig.json test2sig.json \
    --output-document signedTx.json \
    --chain-id $chain_id

  terracli tx broadcast signedTx.json \
    --chain-id $chain_id \
    --broadcast-mode block \
    --output json
}


# CONSTS

chain_id=localterra

# Default terracli args. Compatible with zsh (for bash, replace parentheses with double quote marks)
defaults=(--chain-id $chain_id --fees=100000uluna --broadcast-mode block --output json -y)

# Token info
token_name=Mars
token_symbol=MARS
token_decimals=6
token_minter=$(terracli keys show multi --output json | jq -r .address)
token_cap=1000000000000000

token_info_template='{
  "name": "",
  "symbol": "",
  "decimals": 0,
  "initial_balances": [],
  "mint": {
    "minter": "",
    "cap": ""
  }
}'

token_info=$(
  echo $token_info_template \
    | jq '.name |= $name | .symbol |= $symbol | .decimals |= $decimals | .mint.minter |= $minter | .mint.cap |= $cap' \
      --arg name $token_name \
      --arg symbol $token_symbol \
      --argjson decimals $token_decimals \
      --arg minter $token_minter \
      --arg cap $token_cap
)

cosmwasm_plus_path=../../cosmwasm-plus


# MULTISIG

# Create multisig
terracli keys add multi \
  --multisig=test1,test2,test3 \
  --multisig-threshold=2

multi=$(terracli keys show multi --output json | jq -r .address)

# Send some Luna to the multisig address
terracli tx send test1 $multi 10000000uluna \
  --gas auto \
  $defaults


# CW20 CONTRACT

# Compile the contract
(
  cd $cosmwasm_plus_path/contracts/cw20-base
  RUSTFLAGS='-C link-arg=-s' cargo wasm

  # cd $cosmwasm_plus_path
  # docker run --rm -v "$(pwd)":/code \
  #   --mount type=volume,source="$(basename "$(pwd)")_cache",target=/code/target \
  #   --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  #   cosmwasm/workspace-optimizer:0.11.4
)

# Upload the contract
res=$(
  terracli tx wasm store $cosmwasm_plus_path/target/wasm32-unknown-unknown/release/cw20_base.wasm \
    --from test1 \
    --gas auto \
    $defaults
)
code_id=$(echo $res | jq -r ".logs[0].events[-1].attributes[-1].value")
echo Code ID = $code_id

# Instantiate the token contract
terracli tx wasm instantiate $code_id $token_info \
  --migratable \
  --from $multi \
  --gas 200000 \
  $defaults \
  --generate-only > unsignedTx.json

res=$(multisign-broadcast)
tx_hash=$(echo $res | jq -r .txhash)
echo Tx hash = $tx_hash

res=$(terracli query tx --trust-node $tx_hash --output json)
contract_addr=$(echo $res | jq -r ".logs[0].events[0].attributes[-1].value")
echo Contract address = $contract_addr

terracli query wasm contract-store $contract_addr '{"token_info": {}}'

# Mint some tokens
beneficiary=$(terracli keys show test4 --output json | jq -r .address)
mint_amount=100000000

mint_tx=$(
  jq -n '{"mint": {"recipient": $address, "amount": $amount}}' \
    --arg address $beneficiary \
    --arg amount $mint_amount
)
terracli tx wasm execute $contract_addr $mint_tx \
  --from $multi \
  --gas 2000000 \
  $defaults \
  --generate-only > unsignedTx.json

multisign-broadcast

# Check the total supply of the token is correct
res=$(terracli query wasm contract-store $contract_addr '{"token_info": {}}')
if [ $(echo $res | jq -r .total_supply) != $mint_amount ]; then
  echo ERROR
fi

# Check the token balance of the beneficiary is correct
balance_query=$(
  jq -n '{"balance": {"address": $address}}' \
    --arg address $beneficiary
)
res=$(terracli query wasm contract-store $contract_addr $balance_query)
if [ $(echo $res | jq -r .balance) != $mint_amount ]; then
  echo ERROR
fi


# Migrate
terracli tx wasm migrate $contract_addr $code_id '{"migrate": {}}' \
  --from $multi \
  --gas 200000 \
  $defaults \
  --generate-only > unsignedTx.json

multisign-broadcast
