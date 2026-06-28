use crate::BINS;
use anchor_lang::prelude::*;

/// Number of recent surprises kept on-chain (ring buffer).
pub const SURPRISE_CAP: usize = 64;

/// The protocol's belief: a 256-bin discrete posterior over the token's true
/// price, stored as fixed-point fractions that sum to `SCALE`.
#[account]
pub struct Posterior {
    pub mint: Pubkey,
    pub dlmm_pool: Pubkey,
    pub updates: u64,
    pub last_entropy_fp: i128,
    pub last_map_idx: u16,
    pub bump: u8,
    pub p_fp: [i128; BINS],
}

impl Posterior {
    // 8 disc + 32 + 32 + 8 + 16 + 2 + 1 + 16*BINS
    pub const LEN: usize = 8 + 32 + 32 + 8 + 16 + 2 + 1 + 16 * BINS;
}

/// One logged surprise: a moment the posterior's entropy increased.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default)]
pub struct SurpriseRecord {
    pub slot: u64,
    pub prior_map_idx: u16,
    pub post_map_idx: u16,
    pub delta_h_fp: i128,
    pub trade_price_fp: i128,
    pub trade_size: u64,
}

/// Append-only (ring) log of surprises, one per mint.
#[account]
pub struct SurpriseLog {
    pub mint: Pubkey,
    pub count: u64, // total surprises ever recorded
    pub head: u16,  // next write index into `records`
    pub bump: u8,
    pub records: [SurpriseRecord; SURPRISE_CAP],
}

impl SurpriseLog {
    // 8 disc + 32 + 8 + 2 + 1 + SURPRISE_CAP * 52 (size of one record)
    pub const LEN: usize = 8 + 32 + 8 + 2 + 1 + SURPRISE_CAP * 52;
}
