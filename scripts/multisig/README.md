# Multisig

## Instructions for multisig key holders

As a multisig key holder, you will be required to sign transactions.
The instructions below explain how to do this.

### Dependencies

- go (https://golang.org/dl/)
- terracli (https://github.com/terra-money/core)

### Install terracli

```sh
git clone https://github.com/terra-money/core.git
cd core
make
terracli
```

### Setup key in terracli

```sh
# Setup a new private key
terracli keys add <name>

# Or recover an existing private key
terracli keys add <name> --recover
# Then enter your 24 word mnemonic

# Check the key was added
terracli keys show <name>
```

### Create a signature for a transaction

You will be sent:
- An unsigned transaction `.json` file
- A signing command that will look similar to this:

```sh
# Set `from` to your address that is a key to the multisig: terra1multisigaddress
from=terra1...

terracli tx sign unsigned_tx.json \
  --multisig=terra1multisigaddress \
  --from=$from \
  ...
```

You need to:
1. Open a terminal
2. Change directory to the location of the unsigned transaction `.json` file, e.g. `cd path/to/directory`
3. Replace `terra1...` in the signing command with your address, e.g. `from=terra1youraddress`
4. Run the modified signing command
5. Return the signature `.json` file that is generated

## Instructions for creators of multisig transactions

As a multisig transaction creator, you will be required to create unsigned transactions.
The instructions below explain how to do this.

### Dependencies

- go
- terracli

### Add a multisig key to terracli

```sh
terracli keys add <multisig_name> \
  --multisig <terra1...>,<terra1...>[,<terra1...>] \
  --multisig-threshold <k>
```

### Create an unsigned transaction

1. Add a multisig to terracli
2. Populate a `.env` file with the required environment variables from `create_unsigned_tx.ts`
3. Run `ts-node create_unsigned_tx.ts`
4. Distribute `unsigned_tx.json` to multisig key holders

### Broadcast a transaction

1. Collect signatures from multisig key holders
2. Populate a `.env` file with the required environment variables from `broadcast_tx.ts`
3. Run `ts-node broadcast_tx.ts`
