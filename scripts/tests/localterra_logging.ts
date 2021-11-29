import {
  LocalTerra,
  MsgExecuteContract,
  MsgInstantiateContract,
  Wallet
} from "@terra-money/terra.js"
import {migrate, performTransaction, uploadContract} from "../helpers.js"

export class LocalTerraWithLogging extends LocalTerra {
  gasConsumptions: Array<{msg: string, gas_used: number}>

  constructor() {
    super()
    this.gasConsumptions = []
  }

  async deployContract(wallet: Wallet, filepath: string, initMsg: object) {
    const codeId = await this.uploadContract(wallet, filepath)
    return await this.instantiateContract(wallet, codeId, initMsg)
  }

  async uploadContract(wallet: Wallet, filepath: string) {
    return await uploadContract(this, wallet, filepath)
  }

  async instantiateContract(wallet: Wallet, codeId: number, msg: object, admin?: string) {
    if (admin == undefined) {
      admin = wallet.key.accAddress
    }

    const instantiateMsg = new MsgInstantiateContract(wallet.key.accAddress, admin, codeId, msg, undefined);
    let result = await performTransaction(this, wallet, instantiateMsg)

    // save gas consumption during contract instantiation
    const msgStr = JSON.stringify(msg)
    this.gasConsumptions.push({msg: msgStr, gas_used: result.gas_used})

    const attributes = result.logs[0].events[0].attributes
    return attributes[attributes.length - 1].value // contract address
  }

  async executeContract(wallet: Wallet, contractAddress: string, msg: object, coins?: string) {
    const executeMsg = new MsgExecuteContract(wallet.key.accAddress, contractAddress, msg, coins)
    const result = await performTransaction(this, wallet, executeMsg)

    // save gas consumption during contract execution
    const msgStr = JSON.stringify(msg)
    this.gasConsumptions.push({msg: msgStr, gas_used: result.gas_used})

    return result
  }

  async queryContract(contractAddress: string, query: object): Promise<any> {
    return await this.wasm.contractQuery(contractAddress, query)
  }

  async migrate(wallet: Wallet, contractAddress: string, newCodeId: number) {
    return await migrate(this, wallet, contractAddress, newCodeId)
  }

  showGasConsumption() {
    this.gasConsumptions.forEach(function ({msg, gas_used}) {
      console.log("gas used: ", gas_used, ", msg: ", msg)
    })
  }
}
