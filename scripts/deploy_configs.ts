interface CouncilInitMsg {
  config: {
    address_provider_address?: string,
    proposal_voting_period: number,
    proposal_effective_delay: number,
    proposal_expiration_period: number,
    proposal_required_deposit: string,
    proposal_required_quorum: string,
    proposal_required_threshold: string,
  }
}

interface StakingInitMsg {
  config: {
    owner?: string,
    address_provider_address?: string,
    terraswap_factory_address?: string,
    terraswap_max_spread: string,
    cooldown_duration: number,
    unstake_window: number,
  }
}

interface InsuranceFundInitMsg {
  owner?: string,
  terraswap_factory_address?: string,
  terraswap_max_spread: string,
}

interface RedBankInitMsg {
  config: {
    owner?: string,
    address_provider_address?: string,
    insurance_fund_fee_share: string,
    treasury_fee_share: string,
    ma_token_code_id?: number,
    close_factor: string,
  }
}

interface Asset {
  denom?: string,
  symbol?: string,
  contract_addr?: string,
  initial_borrow_rate: string,
  min_borrow_rate: string,
  max_borrow_rate: string,
  max_loan_to_value: string,
  reserve_factor: string,
  maintenance_margin: string,
  liquidation_bonus: string,
  kp: string,
  optimal_utilization_rate: string,
  kp_augmentation_threshold: string,
  kp_multiplier: string,
}

export interface Config {
  councilInitMsg: CouncilInitMsg,
  stakingInitMsg: StakingInitMsg,
  insuranceFundInitMsg: InsuranceFundInitMsg,
  redBankInitMsg: RedBankInitMsg,
  initialAssets: Asset[],
}

export const testnet: Config = {
  councilInitMsg: {
    "config": {
      "address_provider_address": undefined,

      "proposal_voting_period": 20, // 20 blocks = ~2.5 minutes (for internal testing) // 57600 blocks = ~5 days
      "proposal_effective_delay": 0, // 0 blocks = able to execute proposal immediately (for internal testing) // 11520 blocks = ~24 hours
      "proposal_expiration_period": 115200, // 115200 blocks = ~10 days
      "proposal_required_deposit": "100000000",
      "proposal_required_quorum": "0.1",
      "proposal_required_threshold": "0.05"
    }
  },
  stakingInitMsg: {
    "config": {
      "owner": undefined,
      "address_provider_address": undefined,
      "terraswap_factory_address": "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf",
      "terraswap_max_spread": "0.05",
      "cooldown_duration": 12, // 12 blocks = ~1.5 minutes (for internal testing) // 115200 blocks = ~10 days
      "unstake_window": 40, // 40 blocks = ~5 minutes (for internal testing) // 23040 blocks = ~2 days
    }
  },
  insuranceFundInitMsg: {
    "owner": undefined,
    "terraswap_factory_address": "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf",
    "terraswap_max_spread": "0.05",
  },
  redBankInitMsg: {
    "config": {
      "owner": undefined,
      "address_provider_address": undefined,
      "insurance_fund_fee_share": "0.1",
      "treasury_fee_share": "0.2",
      "ma_token_code_id": undefined,
      "close_factor": "0.5"
    }
  },
  initialAssets: [
    // find contract addresses of CW20's here: https://github.com/terra-project/assets/blob/master/cw20/tokens.json
    {
      denom: "uusd",
      initial_borrow_rate: "0.1",
      min_borrow_rate: "0.01",
      max_borrow_rate: "0.8",
      max_loan_to_value: "0.75",
      reserve_factor: "0.3",
      maintenance_margin: "0.85",
      liquidation_bonus: "0.15",
      kp: "4",
      optimal_utilization_rate: "0.9",
      kp_augmentation_threshold: "0.2",
      kp_multiplier: "1.75"
    },
    {
      denom: "uluna",
      initial_borrow_rate: "0.1",
      min_borrow_rate: "0.03",
      max_borrow_rate: "0.6",
      max_loan_to_value: "0.5",
      reserve_factor: "0.3",
      maintenance_margin: "0.7",
      liquidation_bonus: "0.15",
      kp: "2",
      optimal_utilization_rate: "0.7",
      kp_augmentation_threshold: "0.2",
      kp_multiplier: "2.5"
    },
    {
      symbol: "ANC",
      contract_addr: "terra1747mad58h0w4y589y3sk84r5efqdev9q4r02pc",
      initial_borrow_rate: "0.1",
      min_borrow_rate: "0.03",
      max_borrow_rate: "0.6",
      max_loan_to_value: "0.5",
      reserve_factor: "0.3",
      maintenance_margin: "0.7",
      liquidation_bonus: "0.15",
      kp: "2",
      optimal_utilization_rate: "0.5",
      kp_augmentation_threshold: "0.2",
      kp_multiplier: "2.5"
    },
    {
      symbol: "MIR",
      contract_addr: "terra10llyp6v3j3her8u3ce66ragytu45kcmd9asj3u",
      initial_borrow_rate: "0.1",
      min_borrow_rate: "0.03",
      max_borrow_rate: "0.6",
      max_loan_to_value: "0.5",
      reserve_factor: "0.3",
      maintenance_margin: "0.7",
      liquidation_bonus: "0.15",
      kp: "2",
      optimal_utilization_rate: "0.5",
      kp_augmentation_threshold: "0.2",
      kp_multiplier: "2.5"
    },
  ]
}

export const local: Config = {
  councilInitMsg: {
    "config": {
      "address_provider_address": undefined,

      "proposal_voting_period": 1000,
      "proposal_effective_delay": 150,
      "proposal_expiration_period": 3000,
      "proposal_required_deposit": "100000000",
      "proposal_required_quorum": "0.1",
      "proposal_required_threshold": "0.05"
    }
  },
  stakingInitMsg: {
    "config": {
      "owner": undefined,
      "address_provider_address": undefined,
      "terraswap_factory_address": undefined,
      "terraswap_max_spread": "0.05",
      "cooldown_duration": 10,
      "unstake_window": 300,
    }
  },
  insuranceFundInitMsg: {
    "owner": undefined,
    "terraswap_factory_address": undefined,
    "terraswap_max_spread": "0.05",
  },
  redBankInitMsg: {
    "config": {
      "owner": undefined,
      "address_provider_address": undefined,
      "insurance_fund_fee_share": "0.1",
      "treasury_fee_share": "0.2",
      "ma_token_code_id": undefined,
      "close_factor": "0.5"
    }
  },
  initialAssets: []
}
