//! Onsnes — a Token-2022 transfer hook that maintains a Bayesian posterior over
//! the token's true price and updates it on every transfer.
//!
//! On each transfer token-2022 invokes `transfer_hook`. The program reads the
//! swap's executed price from a Meteora DLMM pool, multiplies its 256-bin prior
//! by a Gaussian likelihood centred on each candidate price, renormalises, and
//! recomputes entropy and the MAP estimate. When entropy increases past a
//! threshold it appends a "surprise" to an on-chain ring log. It never charges
//! a surcharge — the hook returns having only updated belief.
//!
//! No admin, no parameters that can change. Upgrade authority should be set to
//! `None` and mint authority to `null` at deployment (see README).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_error::ProgramError;
use anchor_lang::system_program;
use anchor_spl::token_interface::{Mint, TokenAccount};
use spl_tlv_account_resolution::{account::ExtraAccountMeta, seeds::Seed, state::ExtraAccountMetaList};
use spl_transfer_hook_interface::instruction::{ExecuteInstruction, TransferHookInstruction};

pub mod dlmm;
pub mod errors;
pub mod math;
pub mod state;

use errors::OnsnesError;
use math::*;
use state::*;

// PLACEHOLDER program id — replace by running `anchor keys sync` after the
// first `anchor build` (it overwrites this and Anchor.toml from the generated
// program keypair in target/deploy/onsnes-keypair.json).
declare_id!("11111111111111111111111111111111");

// ---- protocol constants (baked into the binary, never changed) ----
/// Number of discrete price hypotheses. The `lean` build drops to 64 bins (with
/// a table-lookup Gaussian) so the per-transfer update fits a tighter compute
/// budget; the default build keeps the full 256-bin resolution.
#[cfg(feature = "lean")]
pub const BINS: usize = 64;
#[cfg(not(feature = "lean"))]
pub const BINS: usize = 256;

/// Maximum entropy = log2(BINS): 6 bits (lean) / 8 bits (default).
#[cfg(feature = "lean")]
pub const H_MAX_FP: i128 = 6_000_000_000_000;
#[cfg(not(feature = "lean"))]
pub const H_MAX_FP: i128 = 8_000_000_000_000;
/// Confidence floor: 1 bit of irreducible humility.
pub const H_FLOOR_FP: i128 = 1_000_000_000_000;
/// Gaussian likelihood width, in price units (~0.03).
pub const SIGMA_FP: i128 = 30_000_000_000;
/// Entropy increase (in bits, fixed-point) that counts as a surprise (0.02 bits).
pub const SURPRISE_THRESH_FP: i128 = 20_000_000_000;
/// Posterior support: price range floor (0.001) and ceiling (0.003).
pub const PRICE_LO_FP: i128 = 1_000_000_000;
pub const PRICE_HI_FP: i128 = 3_000_000_000;

#[program]
pub mod onsnes {
    use super::*;

    /// Create the posterior PDA (uniform prior, 8 bits of entropy) and the
    /// surprise-log PDA. `dlmm_pool` is the pool the hook will read prices from.
    pub fn initialize(ctx: Context<Initialize>, dlmm_pool: Pubkey) -> Result<()> {
        let post = &mut ctx.accounts.posterior;
        post.mint = ctx.accounts.mint.key();
        post.dlmm_pool = dlmm_pool;
        post.updates = 0;
        post.last_entropy_fp = H_MAX_FP;
        post.last_map_idx = (BINS / 2) as u16;
        post.bump = ctx.bumps.posterior;
        let uniform = SCALE / (BINS as i128);
        for v in post.p_fp.iter_mut() {
            *v = uniform;
        }

        let log = &mut ctx.accounts.surprise_log;
        log.mint = ctx.accounts.mint.key();
        log.count = 0;
        log.head = 0;
        log.bump = ctx.bumps.surprise_log;

        msg!("onsnes: initialised uniform prior, H={} bits (fp)", H_MAX_FP);
        Ok(())
    }

    /// Build the ExtraAccountMetaList token-2022 reads to know which extra
    /// accounts to pass into the hook: the posterior PDA, the surprise log PDA,
    /// and the bound DLMM pool. Call after `initialize`.
    pub fn initialize_extra_account_meta_list(
        ctx: Context<InitializeExtraAccountMetaList>,
    ) -> Result<()> {
        let dlmm_pool = ctx.accounts.posterior.dlmm_pool;

        // Transfer-hook account indices: 0 source, 1 mint, 2 destination,
        // 3 owner, 4 this meta list. Our extras begin at index 5.
        let extra_metas = vec![
            // index 5: posterior PDA, seeds ["posterior", mint]
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::Literal { bytes: b"posterior".to_vec() },
                    Seed::AccountKey { index: 1 },
                ],
                false, // is_signer
                true,  // is_writable
            )?,
            // index 6: surprise log PDA, seeds ["surprises", mint]
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::Literal { bytes: b"surprises".to_vec() },
                    Seed::AccountKey { index: 1 },
                ],
                false,
                true,
            )?,
            // index 7: the DLMM pool (read-only, fixed pubkey)
            ExtraAccountMeta::new_with_pubkey(&dlmm_pool, false, false)?,
        ];

        let account_size = ExtraAccountMetaList::size_of(extra_metas.len())? as u64;
        let lamports = Rent::get()?.minimum_balance(account_size as usize);

        let mint = ctx.accounts.mint.key();
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"extra-account-metas",
            mint.as_ref(),
            &[ctx.bumps.extra_account_meta_list],
        ]];

        system_program::create_account(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::CreateAccount {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.extra_account_meta_list.to_account_info(),
                },
            )
            .with_signer(signer_seeds),
            lamports,
            account_size,
            ctx.program_id,
        )?;

        ExtraAccountMetaList::init::<ExecuteInstruction>(
            &mut ctx.accounts.extra_account_meta_list.try_borrow_mut_data()?,
            &extra_metas,
        )?;

        msg!("onsnes: extra account meta list initialised");
        Ok(())
    }

    /// The transfer hook. token-2022 calls this on every transfer of the mint.
    pub fn transfer_hook(ctx: Context<TransferHook>, amount: u64) -> Result<()> {
        let post = &mut ctx.accounts.posterior;

        // 1. read the swap's executed price from the DLMM pool active bin
        let trade_price_fp =
            dlmm::read_active_bin_price(&ctx.accounts.dlmm_pool, PRICE_LO_FP, PRICE_HI_FP)?;

        // 2. snapshot prior entropy + MAP
        let prior_h_fp = post.last_entropy_fp;
        let prior_map = post.last_map_idx;

        // 3. multiply prior by the Gaussian likelihood for each candidate price.
        //    lean build: O(BINS) table lookups, no on-chain exp.
        //    default build: a true fixed-point Gaussian per bin.
        #[cfg(feature = "lean")]
        let obs = price_to_bin(trade_price_fp, BINS, PRICE_LO_FP, PRICE_HI_FP);
        for i in 0..BINS {
            #[cfg(feature = "lean")]
            let like = math::lut::weight(if i >= obs { i - obs } else { obs - i });
            #[cfg(not(feature = "lean"))]
            let like = {
                let mu = bin_to_price_fp(i, BINS, PRICE_LO_FP, PRICE_HI_FP);
                gaussian_fp(trade_price_fp, mu, SIGMA_FP)
            };
            post.p_fp[i] = mul_fp(post.p_fp[i], like);
        }

        // 4. renormalise so the posterior sums to SCALE
        renormalise(&mut post.p_fp);

        // 5. new entropy (clamped to [floor, max]) and MAP
        let mut new_h = entropy_fp(&post.p_fp);
        if new_h < H_FLOOR_FP {
            new_h = H_FLOOR_FP;
        }
        if new_h > H_MAX_FP {
            new_h = H_MAX_FP;
        }
        let new_map = argmax(&post.p_fp);

        post.last_entropy_fp = new_h;
        post.last_map_idx = new_map;
        post.updates = post.updates.saturating_add(1);

        // 6. if entropy increased past the threshold, record a surprise
        let delta_h = new_h - prior_h_fp;
        if delta_h > SURPRISE_THRESH_FP {
            let log = &mut ctx.accounts.surprise_log;
            let slot = Clock::get()?.slot;
            let idx = (log.head as usize) % SURPRISE_CAP;
            log.records[idx] = SurpriseRecord {
                slot,
                prior_map_idx: prior_map,
                post_map_idx: new_map,
                delta_h_fp: delta_h,
                trade_price_fp,
                trade_size: amount,
            };
            log.head = ((log.head as usize + 1) % SURPRISE_CAP) as u16;
            log.count = log.count.saturating_add(1);
        }

        msg!(
            "onsnes: H={} dH={} MAP={} updates={}",
            new_h,
            delta_h,
            new_map,
            post.updates
        );
        Ok(())
    }

    /// Fallback router so token-2022's raw `Execute` instruction reaches
    /// `transfer_hook` above. token-2022 does not use Anchor's 8-byte
    /// discriminator, so we unpack the transfer-hook interface here.
    pub fn fallback<'info>(
        program_id: &Pubkey,
        accounts: &'info [AccountInfo<'info>],
        data: &[u8],
    ) -> Result<()> {
        let instruction =
            TransferHookInstruction::unpack(data).map_err(|_| ProgramError::InvalidInstructionData)?;
        match instruction {
            TransferHookInstruction::Execute { amount } => {
                let amount_bytes = amount.to_le_bytes();
                __private::__global::transfer_hook(program_id, accounts, &amount_bytes)
            }
            _ => Err(ProgramError::InvalidInstructionData.into()),
        }
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(
        init,
        payer = payer,
        space = Posterior::LEN,
        seeds = [b"posterior", mint.key().as_ref()],
        bump
    )]
    pub posterior: Account<'info, Posterior>,
    #[account(
        init,
        payer = payer,
        space = SurpriseLog::LEN,
        seeds = [b"surprises", mint.key().as_ref()],
        bump
    )]
    pub surprise_log: Account<'info, SurpriseLog>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeExtraAccountMetaList<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,
    /// CHECK: PDA created in the handler; seeds = ["extra-account-metas", mint].
    #[account(
        mut,
        seeds = [b"extra-account-metas", mint.key().as_ref()],
        bump
    )]
    pub extra_account_meta_list: AccountInfo<'info>,
    #[account(
        seeds = [b"posterior", mint.key().as_ref()],
        bump = posterior.bump
    )]
    pub posterior: Account<'info, Posterior>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TransferHook<'info> {
    #[account(token::mint = mint, token::authority = owner)]
    pub source_token: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(token::mint = mint)]
    pub destination_token: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: the transfer authority / source owner.
    pub owner: UncheckedAccount<'info>,
    /// CHECK: validated PDA, seeds = ["extra-account-metas", mint].
    #[account(seeds = [b"extra-account-metas", mint.key().as_ref()], bump)]
    pub extra_account_meta_list: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [b"posterior", mint.key().as_ref()],
        bump = posterior.bump
    )]
    pub posterior: Account<'info, Posterior>,
    #[account(
        mut,
        seeds = [b"surprises", mint.key().as_ref()],
        bump = surprise_log.bump
    )]
    pub surprise_log: Account<'info, SurpriseLog>,
    /// CHECK: read-only; must match the pool bound at initialise time.
    #[account(address = posterior.dlmm_pool @ OnsnesError::WrongPool)]
    pub dlmm_pool: UncheckedAccount<'info>,
}
