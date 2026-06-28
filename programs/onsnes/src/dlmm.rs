use crate::errors::OnsnesError;
use crate::math::{pow_fp, SCALE};
use anchor_lang::prelude::*;

/// Reconstruct the executed price from a Meteora DLMM pool account.
///
/// This assumes a specific byte layout — the active bin id as an `i32` at byte
/// offset 88, and the bin step (in basis points) as a `u16` at offset 92 — and
/// rebuilds the price as `(1 + bin_step/10_000)^active_id`. The real Meteora
/// DLMM `LbPair` layout differs; adapt this reader to the deployed pool program
/// before mainnet use. The reconstructed price is clamped to [price_lo, price_hi]
/// so it always lands inside the posterior's support.
pub fn read_active_bin_price(pool: &AccountInfo, price_lo: i128, price_hi: i128) -> Result<i128> {
    let data = pool.try_borrow_data()?;
    require!(data.len() >= 96, OnsnesError::PoolAccountMalformed);

    let active_id = i32::from_le_bytes(data[88..92].try_into().unwrap());
    let bin_step = u16::from_le_bytes(data[92..94].try_into().unwrap()) as i128;

    let base = SCALE + (bin_step * SCALE) / 10_000;
    let mut price = pow_fp(base, active_id);

    if price < price_lo {
        price = price_lo;
    }
    if price > price_hi {
        price = price_hi;
    }
    Ok(price)
}
