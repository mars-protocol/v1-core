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

beneficiary=terra1fmcjjt6yc9wqup2r06urnrd928jhrde6gcld6n

terracli tx send $multi $beneficiary 5000000uluna \
  --gas=200000 \
  --fees=100000uluna \
  --chain-id=localterra \
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
  --broadcast-mode=block

terracli query account $beneficiary
