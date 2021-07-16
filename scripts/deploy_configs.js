export const testnet = {
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
            "terraswap_factory_address": "terra18qpjm4zkvqnpjpw0zn0tdr8gdzvt8au35v45xf",
            "terraswap_max_spread": "0.05",
            "cooldown_duration": 10,
            "unstake_window": 300,
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

export const local = {
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