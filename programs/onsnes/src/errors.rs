use anchor_lang::prelude::*;

#[error_code]
pub enum OnsnesError {
    #[msg("dlmm pool account data is malformed or too short")]
    PoolAccountMalformed,
    #[msg("provided pool does not match the posterior's bound pool")]
    WrongPool,
}
