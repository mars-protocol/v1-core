# docker compose -f $LOCAL_TERRA_PATH/docker-compose.yml up -d

for test in tests/liquidations*.ts; do
  # oracle test must be run with slower block times
  if [[ $test =~ oracle.ts ]]; then
    continue
  fi

  echo Running $test
  node --loader ts-node/esm $test

  exit_code=$?
  if [ $exit_code -ne 0 ]; then
    echo Error $test
    return $exit_code
  fi
done

# docker compose -f $LOCAL_TERRA_PATH/docker-compose.yml down
