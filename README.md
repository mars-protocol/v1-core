# Mars

### Deploying
1. Ensure you have LocalTerra running
2.  Change the filepath in `deploy.js` to point to the contract's `.wasm` file
3.  Modify the functions you wish to call and their messages at the bottom of `deploy.js`
  * Note: If you want to interact with a contract but do not wish to upload or instantiate a new one,
    you can find the contract's address in Terra Station and pass it in manually
4.  Run `node deploy.js` from the `scripts` directory

### Testnet

In order to initialize and recover wallets on the testnet:
1. Call the appropriate function in `testnet.js`
   * If initializing, create and pass in a mnemonic. If recovering, load the mnemonic for `.env` by typing `process.env.<variable_name>`
2. When initializing, make sure to save the mnemonic in the `.env` file. This allows us to recover and reuse the same wallet
3. Run `testnet.js` using the command: `node -r dotenv/config testnet.js` from the scripts directory
4. To add funds to your wallet, visit `https://faucet.terra.money/` and insert your public testnet address which can be
   found using `wallet.key.accAddress`


