import {LCDClient, MnemonicKey, MsgSend, Wallet} from "@terra-money/terra.js";

function initialize(mnemonic) {
  const mk = new MnemonicKey({mnemonic: mnemonic});
  const wallet = terra.wallet(mk);

  let accountAddress = wallet.key.accAddress
  let publicKey = wallet.key.accPubKey

  console.log(`Account Address: ${accountAddress}`)
  console.log(`Public Key: ${publicKey}`)

  return wallet
}

function recover(mnemonic) {
  const mk = new MnemonicKey({mnemonic: mnemonic});
  return terra.wallet(mk);
}

const terra = new LCDClient({
  URL: 'https://tequila-lcd.terra.dev',
  chainID: 'tequila-0004'
});

// let wallet = initialize("insert mnemonic here");
let wallet = recover(process.env.TEST_MAIN);
console.log(wallet.key.accAddress);
