# Onsnes

A Solana token whose price quote is shaped by a piece of math: a **Token-2022
transfer hook** that maintains a 256-bin **Bayesian posterior** over the token's
true price and updates it — by Bayes' theorem, in `i128` fixed-point — on every
transfer. When its entropy rises it logs a "surprise" on-chain. It charges no
surcharge; the hook only updates belief.

This repo contains two things:

| Part | Path | What it is |
|------|------|-----------|
| **Frontend** | `index.html`, `Onsnes.png` | Self-contained static site (no build step). Live at GitHub Pages. |
| **Program** | `programs/onsnes/`, `Anchor.toml`, `Cargo.toml`, `tests/` | The Anchor/Rust transfer-hook program + an end-to-end test. |

---

## ⚠️ Read this first (honesty about state)

- **Not compiled in this repo's authoring environment.** The Solana/Anchor
  toolchain isn't available where these files were written, so the program has
  **not been `anchor build`-verified here**. Treat it as a complete, idiomatic
  **reference implementation** you build and test yourself (steps below).
- **Compute budget.** The default build runs a 256-bin Gaussian update *and* a
  256-bin entropy sum per transfer — heavy, and it will not fit the default 200k
  compute units. Two mitigations ship here: (1) the **`lean` build** (64 bins +
  a table-lookup Gaussian, no on-chain `exp`) — `anchor build -- --features lean`;
  and (2) you should still raise the swap transaction's compute budget. Even
  lean, profile before mainnet.
- **DLMM layout.** `programs/onsnes/src/dlmm.rs` reads `active_id` at account
  offset **76** and `bin_step` at **80**, derived from the published Meteora
  `LbPair` layout (MeteoraAg/dlmm-sdk IDL — `StaticParameters` and
  `VariableParameters` are 32 bytes each; see the offset table in the file).
  Meteora has revised this struct over time, so **verify against the deployed
  pool** before mainnet, and add a `10^(decimals_x - decimals_y)` factor +
  realistic `PRICE_LO/HI` for your token.
- **The fixed-point `exp`/`log2`** in `math.rs` are small polynomial
  approximations — good enough to drive the posterior, not IEEE-accurate.
- **Deploying is a real on-chain action** with cost and irreversibility. You run
  it; nothing here deploys anything for you.

---

## Prerequisites

Build on **Linux / macOS / WSL** (native Windows is not supported for Solana BPF):

- [Rust](https://rustup.rs/)
- [Solana CLI](https://docs.solanalabs.com/cli/install) (`solana --version`)
- [Anchor](https://www.anchor-lang.com/docs/installation) `0.30.1`
  (`avm install 0.30.1 && avm use 0.30.1`)
- Node.js 18+ and `yarn`

## Build & test the program

```bash
yarn install            # JS deps for the tests
anchor keys sync        # generate the program keypair + write its id into
                        # lib.rs (declare_id!) and Anchor.toml — replaces the
                        # 1111…1111 placeholder
anchor build            # compile the BPF program + generate the IDL/types
anchor build -- --features lean   # CU-optimised variant: 64 bins + LUT gaussian
anchor test             # spins up a local validator and runs tests/onsnes.ts
```

`anchor test` creates a Token-2022 mint with the hook, a mock pool, initialises
the posterior, then does one transfer and asserts the posterior updated and
entropy dropped below 8 bits.

## Deploy

```bash
# choose a cluster
solana config set --url devnet        # or mainnet-beta

anchor build
anchor deploy                         # prints the program id

# After verifying, make it ownerless (matches the site's narrative):
solana program set-upgrade-authority <PROGRAM_ID> --final
```

## Wire it to a live token

1. Create a **Token-2022 mint** with the **TransferHook** extension pointing at
   your deployed program id (see `tests/onsnes.ts` for the exact instructions,
   or use the `spl-token` CLI / SDK).
2. Call `initialize(dlmm_pool)` — creates the posterior (uniform prior, 8 bits)
   and the surprise log.
3. Call `initialize_extra_account_meta_list()` — lets token-2022 resolve the
   hook's extra accounts automatically.
4. (Optional) set mint authority to `null` once supply is minted.
5. In `index.html`, set `CONFIG.ca` (search for `ca: ''`) to your mint address.
   The frontend's live module then pulls real prices from dexscreener and runs
   the same Bayesian update client-side to drive the charts.

## Run the frontend locally

It's a single static file — no build:

```bash
npx serve -l 8080 .
# open http://localhost:8080
```

## Program layout

```
programs/onsnes/src/
  lib.rs      instructions, accounts, transfer-hook entrypoint + fallback router
  state.rs    Posterior (256-bin) and SurpriseLog (ring buffer) accounts
  math.rs     i128 fixed-point: gaussian, exp, log2, entropy, renormalise, pow
  dlmm.rs     reconstructs executed price from the DLMM pool active bin
  errors.rs   program errors
```

## License

MIT
