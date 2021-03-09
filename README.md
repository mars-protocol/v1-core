# Mars

### Deploying
1. Ensure you have LocalTerra running
2.  Change the filepath in `deploy.js` to point to the contract's `.wasm` file
3.  Modify the functions you wish to call and their messages at the bottom of `deploy.js`
  * Note: If you want to interact with a contract but do not wish to upload or instantiate a new one,
    you can find the contract's address in Terra Station and pass it in manually
4.  Run `node deploy.js` from the `scripts` directory




