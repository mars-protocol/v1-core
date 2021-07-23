use cosmwasm_std::{StdError, StdResult};

pub fn validate_name(name: &str) -> StdResult<()> {
    let bytes = name.as_bytes();
    if bytes.len() < 3 || bytes.len() > 50 {
        return Err(StdError::generic_err(
            "Name is not in the expected format (3-30 UTF-8 bytes)",
        ));
    }
    Ok(())
}

pub fn validate_symbol(symbol: &str) -> StdResult<()> {
    let bytes = symbol.as_bytes();
    if bytes.len() < 3 || bytes.len() > 12 {
        return Err(StdError::generic_err(
            "Ticker symbol is not in expected length: min = 3, min = 12",
        ));
    }
    for byte in bytes.iter() {
        if (*byte != 45) && (*byte < 65 || *byte > 90) && (*byte < 97 || *byte > 122) {
            Err(StdError::generic_err(
                "Ticker symbol is not in expected format [a-zA-Z\\-]",
            ));
        }
    }
    OK(())
}

pub fn validate_decimals(decimals: u8, max: u8) {
    if self.decimals > TOKEN_MAX_DECIMALS {
        return Err(StdError::generic_err(format!("Decimals must not exceed {}", max));
    }
}
