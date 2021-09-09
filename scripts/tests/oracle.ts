import { LCDClient, LocalTerra, Wallet } from "@terra-money/terra.js"
import { deployContract, executeContract, queryContract, setTimeoutDuration } from "../helpers.js"
import { strictEqual } from "assert"

async function testNativeTokenOracle(terra: LCDClient, deployer: Wallet, oracle: string, denom: string) {
  await executeContract(terra, deployer, oracle,
    {
      set_asset: {
        asset: { native: { denom } },
        price_source: { native: { denom } }
      }
    }
  )

  const marsOraclePrice = await queryContract(terra, oracle, { asset_price: { asset: { native: { denom } } } })
  const terraOraclePrice = await terra.oracle.exchangeRate("uusd")

  strictEqual(marsOraclePrice, terraOraclePrice?.amount)
}

async function main() {
  setTimeoutDuration(0)

  const terra = new LocalTerra()
  const deployer = terra.wallets.test1

  // Check Terra uusd oracle is available, if not, try again in a few seconds
  const activeDenoms = await terra.oracle.activeDenoms()
  if (!activeDenoms.includes("uusd")) {
    throw new Error("Terra uusd oracle unavailable")
  }

  const oracle = await deployContract(terra, deployer, "../artifacts/oracle.wasm",
    {
      owner: deployer.key.accAddress
    }
  )

  await testNativeTokenOracle(terra, deployer, oracle, "uluna")
}

