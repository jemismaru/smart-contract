use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Invalid seller address.")]
    InvalidSellerAddress,
    #[msg("Invalid payment contract address.")]
    InvalidPaymentContractAddress,
}

pub fn uint_to_string(value: u64) -> String {
    // Convert the unsigned integer to a string using Rust's built-in method
    value.to_string()
}

pub fn generate_metadata(
    listing_id: &str,
    amount: u64,
    time: i64,
    seller_address: Pubkey,
    payment_contract_address: Pubkey
) -> Result<String> {
    // Perform validation checks
    require!(seller_address != Pubkey::default(), ErrorCode::InvalidSellerAddress);
    require!(
        payment_contract_address != Pubkey::default(),
        ErrorCode::InvalidPaymentContractAddress
    );

    // Convert amount and time to string
    let amount_str = uint_to_string(amount);
    let time_str = uint_to_string(time as u64);

    // Construct the metadata string using format!
    let metadata = format!(
        "listing_id:{}, amount:{}, time:{}, seller:{}, minter:{}",
        listing_id,
        amount_str,
        time_str,
        seller_address,
        payment_contract_address
    );

    Ok(metadata)
}
