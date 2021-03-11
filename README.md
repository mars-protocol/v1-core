# Mars

### Build
1. Run `./scripts/build_artifacts.sh`

### Deploy Dev
1. Run `cd scripts && yarn install`
2. Ensure you have LocalTerra running
3.  Change the filepath in `deploy_local.mjs` to point to the contract's `.wasm` file
4.  Modify the functions you wish to call and their messages at the bottom of `deploy_local.mjs`
  * Note: If you want to interact with a contract but do not wish to upload or instantiate a new one,
    you can find the contract's address in Terra Station and pass it in manually
5.  Run `node deploy_local.mjs` from the `scripts` directory

### Testing
1. Run `cd scripts && node integration_tests.js`

