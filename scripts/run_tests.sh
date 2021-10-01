#!/bin/bash

# Instructions:
# LOCAL_TERRA_REPO_PATH must be provided

set -e

docker compose -f $LOCAL_TERRA_REPO_PATH/docker-compose.yml down

sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/200ms/' $LOCAL_TERRA_REPO_PATH/config/config.toml

docker compose -f $LOCAL_TERRA_REPO_PATH/docker-compose.yml up -d

# oracle tests must be run with slower block times
tests=$(ls tests/*.ts | grep -v oracle)

for test in $tests; do
  echo Running $test
  node --loader ts-node/esm $test
done

docker compose -f $LOCAL_TERRA_REPO_PATH/docker-compose.yml down

# oracle tests
sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/1500ms/' $LOCAL_TERRA_REPO_PATH/config/config.toml

docker compose -f $LOCAL_TERRA_REPO_PATH/docker-compose.yml up -d

echo Running tests/oracle.ts
node --loader ts-node/esm tests/oracle.ts

docker compose -f $LOCAL_TERRA_REPO_PATH/docker-compose.yml down

sed -E -i .bak '/timeout_(propose|prevote|precommit|commit)/s/[0-9]+m?s/200ms/' $LOCAL_TERRA_REPO_PATH/config/config.toml
