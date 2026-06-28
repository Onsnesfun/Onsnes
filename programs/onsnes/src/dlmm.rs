use crate::errors::OnsnesError;
use crate::math::{pow_fp, SCALE};
use anchor_lang::prelude::*;

/// Byte offsets of `active_id` and `bin_step` inside a Meteora DLMM `LbPair`
/// account, derived from the on-chain layout (MeteoraAg/dlmm-sdk IDL,
/// `idls/dlmm.json`, type `LbPair` — `serialization: bytemuck`, `repr(C)`):
///
/// ```text
///   8   anchor discriminator
///  +0   parameters:     StaticParameters    (32 bytes)
///  +32  v_parameters:   VariableParameters  (32 bytes)
///  +64  bump_seed:      [u8; 1]
///  +65  bin_step_seed:  [u8; 2]
///  +67  pair_type:      u8
///  +68  active_id:      i32   -> account offset 8 + 68 = 76
///  +72  bin_step:       u16   -> account offset 8 + 72 = 80
/// ```
///
/// StaticParameters and VariableParameters are each exactly 32 bytes (they carry
/// explicit `_padding` fields so they are bytemuck `Pod`), so there is no
/// implicit padding before `active_id`. Verify against the deployed pool before
/// mainnet — Meteora has revised this struct over time (e.g. `base_fee_power_factor`,
/// `collect_fee_mode` were added inside the explicit-padding budget).
pub const ACTIVE_ID_OFFSET: usize = 76;
pub const BIN_STEP_OFFSET: usize = 80;

/// Reconstruct the executed price from a Meteora DLMM `LbPair` account.
///
/// DLMM prices are geometric in the active bin id: `price = (1 + bin_step/1e4)^active_id`.
/// That raw price is the ratio of token Y per token X in base units; for a UI
/// price you would additionally scale by `10^(decimals_x - decimals_y)`. Here we
/// clamp into the posterior's support `[price_lo, price_hi]`, so the mapping into
/// belief space stays well-defined regardless of the token's absolute scale —
/// adjust `PRICE_LO_FP` / `PRICE_HI_FP` (and add a decimals factor) to match a
/// real token's range.
pub fn read_active_bin_price(pool: &AccountInfo, price_lo: i128, price_hi: i128) -> Result<i128> {
    let data = pool.try_borrow_data()?;
    require!(
        data.len() >= BIN_STEP_OFFSET + 2,
        OnsnesError::PoolAccountMalformed
    );

    let active_id = i32::from_le_bytes(
        data[ACTIVE_ID_OFFSET..ACTIVE_ID_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    let bin_step = u16::from_le_bytes(
        data[BIN_STEP_OFFSET..BIN_STEP_OFFSET + 2]
            .try_into()
            .unwrap(),
    ) as i128;

    // base = 1 + bin_step / 10_000  (fixed-point)
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
