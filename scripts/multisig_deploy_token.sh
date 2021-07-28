# LocalTerra 1c3f42a60116b4c17cb5d002aa194eae9b8811b5

# notice oak worry limit wrap speak medal online prefer cluster roof addict wrist behave treat actual wasp year salad speed social layer crew genius
terracli keys add test1 --recover

# quality vacuum heart guard buzz spike sight swarm shove special gym robust assume sudden deposit grid alcohol choice devote leader tilt noodle tide penalty
terracli keys add test2 --recover

# symbol force gallery make bulk round subway violin worry mixture penalty kingdom boring survey tool fringe patrol sausage hard admit remember broken alien absorb
terracli keys add test3 --recover

terracli keys add multi \
  --multisig=test1,test2,test3 \
  --multisig-threshold=2

terracli keys show multi

multi=$(terracli keys show multi --output json | jq -r .address)

terracli tx send test1 $multi 10000000uluna \
  --chain-id=localterra \
  --gas=auto \
  --fees=100000uluna \
  --broadcast-mode=block

(
  cd ../../cosmwasm-plus/contracts/cw20-base

  # docker run --rm -v "$(pwd)":/code \
  #   --mount type=volume,source="$(basename "$(pwd)")_cache",target=/code/target \
  #   --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  #   cosmwasm/rust-optimizer:0.11.4

  RUSTFLAGS='-C link-arg=-s' cargo wasm
)

res=$(
  terracli tx wasm store ../../cosmwasm-plus/target/wasm32-unknown-unknown/release/cw20_base.wasm \
    --from test1 \
    --chain-id localterra \
    --gas auto \
    --fees=100000uluna \
    --broadcast-mode=block \
    --output json \
    -y
)

code_id=$(echo $res | jq -r ".logs[0].events[-1].attributes[-1].value")

beneficiary=terra1fmcjjt6yc9wqup2r06urnrd928jhrde6gcld6n
amount=1000000000000000

token_info_template='{"name": "Mars", "symbol": "MARS", "decimals": 6, "initial_balances": [{"address": "", "amount": ""}]}'

token_info=$(
  echo $token_info_template | jq -c \
    --arg address $beneficiary \
    --arg amount $amount \
    '.initial_balances[0].address |= $address | .initial_balances[0].amount |= $amount'
)


###### single

res=$(
  terracli tx wasm instantiate $code_id $token_info \
    --migratable \
    --from test1 \
    --chain-id localterra \
    --gas auto \
    --fees=100000uluna \
    --broadcast-mode=block \
    --output json \
    -y
)

tx_hash=$(echo $res | jq -r .txhash)

res=$(terracli query tx --trust-node $tx_hash --output json)

contract_addr=$(echo $res | jq -r ".logs[0].events[0].attributes[-1].value")

res=$(
  terracli query wasm contract-store $contract_addr \
    $(jq -n -c --arg address $beneficiary '{"balance": {"address": $address}}')
)

tx_hash=$(echo $res | jq -r .txhash)

res=$(terracli query tx --trust-node $tx_hash --output json)

contract_addr=$(echo $res | jq -r ".logs[0].events[0].attributes[-1].value")

terracli query wasm contract-store $contract_addr '{"token_info": {}}'

res=$(
  terracli query wasm contract-store $contract_addr \
    $(jq -n -c --arg address $beneficiary '{"balance": {"address": $address}}')
)

if [ $(echo $res | jq -r .balance) = $amount ]; then
  echo yes
else
  echo no
fi

terracli tx wasm migrate $contract_addr $code_id '{"migrate": {}}' \
  --from test1 \
  --chain-id localterra \
  --gas 200000 \
  --fees=100000uluna \
  --broadcast-mode=block \
  --output json \
  -y



##### multi

terracli tx wasm instantiate $code_id $token_info \
  --migratable \
  --from $multi \
  --chain-id localterra \
  --gas 200000 \
  --fees=100000uluna \
  --broadcast-mode=block \
  --output json \
  -y \
  --generate-only > unsignedTx.json

terracli tx sign unsignedTx.json \
  --multisig=$multi \
  --from=test1 \
  --output-document=test1sig.json \
  --chain-id=localterra

terracli tx sign unsignedTx.json \
  --multisig=$multi \
  --from=test2 \
  --output-document=test2sig.json \
  --chain-id=localterra

terracli tx multisign unsignedTx.json multi test1sig.json test2sig.json \
  --output-document=signedTx.json \
  --chain-id=localterra

res=$(
  terracli tx broadcast signedTx.json \
    --chain-id=localterra \
    --broadcast-mode=block \
    --output json
)

tx_hash=$(echo $res | jq -r .txhash)

res=$(terracli query tx --trust-node $tx_hash --output json)

contract_addr=$(echo $res | jq -r ".logs[0].events[0].attributes[-1].value")

terracli query wasm contract-store $contract_addr '{"token_info": {}}'

res=$(
  terracli query wasm contract-store $contract_addr \
    $(jq -n -c --arg address $beneficiary '{"balance": {"address": $address}}')
)

if [ $(echo $res | jq -r .balance) = $amount ]; then
  echo yes
else
  echo no
fi

terracli tx wasm migrate $contract_addr $code_id '{"migrate": {}}' \
  --from $multi \
  --chain-id localterra \
  --gas 200000 \
  --fees=100000uluna \
  --broadcast-mode=block \
  --output json \
  -y \
  --generate-only > unsignedTx.json

terracli tx sign unsignedTx.json \
  --multisig=$multi \
  --from=test1 \
  --output-document=test1sig.json \
  --chain-id=localterra

terracli tx sign unsignedTx.json \
  --multisig=$multi \
  --from=test2 \
  --output-document=test2sig.json \
  --chain-id=localterra

terracli tx multisign unsignedTx.json multi test1sig.json test2sig.json \
  --output-document=signedTx.json \
  --chain-id=localterra

terracli tx broadcast signedTx.json \
  --chain-id=localterra \
  --broadcast-mode=block \
  --output json

