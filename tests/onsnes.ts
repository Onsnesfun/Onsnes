import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Onsnes } from "../target/types/onsnes";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  ExtensionType,
  TOKEN_2022_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMintInstruction,
  createInitializeTransferHookInstruction,
  createMintToInstruction,
  createTransferCheckedWithTransferHookInstruction,
  getAssociatedTokenAddressSync,
  getMintLen,
} from "@solana/spl-token";
import { assert } from "chai";

// End-to-end test against a local validator:
//   anchor test
//
// It builds a Token-2022 mint whose transfer hook is this program, creates a
// *mock* DLMM pool (a zeroed, program-owned 96-byte account — active_id and
// bin_step both 0, so the reconstructed price clamps to PRICE_HI), initialises
// the posterior + meta list, then does one transfer to fire the hook and checks
// that the posterior updated and entropy dropped below the 8-bit maximum.

describe("onsnes", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Onsnes as Program<Onsnes>;
  const wallet = provider.wallet as anchor.Wallet;
  const connection = provider.connection;
  const decimals = 9;

  const mintKp = Keypair.generate();
  const mint = mintKp.publicKey;
  const poolKp = Keypair.generate();

  const [posterior] = PublicKey.findProgramAddressSync(
    [Buffer.from("posterior"), mint.toBuffer()],
    program.programId
  );
  const [surpriseLog] = PublicKey.findProgramAddressSync(
    [Buffer.from("surprises"), mint.toBuffer()],
    program.programId
  );
  const [extraMetas] = PublicKey.findProgramAddressSync(
    [Buffer.from("extra-account-metas"), mint.toBuffer()],
    program.programId
  );

  it("creates a token-2022 mint with the transfer hook", async () => {
    const mintLen = getMintLen([ExtensionType.TransferHook]);
    const lamports = await connection.getMinimumBalanceForRentExemption(mintLen);
    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: wallet.publicKey,
        newAccountPubkey: mint,
        space: mintLen,
        lamports,
        programId: TOKEN_2022_PROGRAM_ID,
      }),
      createInitializeTransferHookInstruction(
        mint,
        wallet.publicKey,
        program.programId,
        TOKEN_2022_PROGRAM_ID
      ),
      createInitializeMintInstruction(
        mint,
        decimals,
        wallet.publicKey,
        null,
        TOKEN_2022_PROGRAM_ID
      )
    );
    await sendAndConfirmTransaction(connection, tx, [wallet.payer, mintKp]);
  });

  it("creates a mock dlmm pool account (zeroed, program-owned)", async () => {
    const space = 96;
    const lamports = await connection.getMinimumBalanceForRentExemption(space);
    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: wallet.publicKey,
        newAccountPubkey: poolKp.publicKey,
        space,
        lamports,
        programId: program.programId,
      })
    );
    await sendAndConfirmTransaction(connection, tx, [wallet.payer, poolKp]);
  });

  it("initialises posterior + surprise log", async () => {
    await program.methods
      .initialize(poolKp.publicKey)
      .accounts({
        payer: wallet.publicKey,
        mint,
        posterior,
        surpriseLog,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const post = await program.account.posterior.fetch(posterior);
    assert.equal(post.updates.toNumber(), 0);
    assert.equal(post.lastEntropyFp.toString(), "8000000000000");
  });

  it("initialises the extra account meta list", async () => {
    await program.methods
      .initializeExtraAccountMetaList()
      .accounts({
        payer: wallet.publicKey,
        mint,
        extraAccountMetaList: extraMetas,
        posterior,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  });

  it("mints and transfers, firing the hook + updating the posterior", async () => {
    const src = getAssociatedTokenAddressSync(
      mint,
      wallet.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID
    );
    const dstOwner = Keypair.generate();
    const dst = getAssociatedTokenAddressSync(
      mint,
      dstOwner.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID
    );

    const setup = new Transaction().add(
      createAssociatedTokenAccountInstruction(
        wallet.publicKey,
        src,
        wallet.publicKey,
        mint,
        TOKEN_2022_PROGRAM_ID
      ),
      createAssociatedTokenAccountInstruction(
        wallet.publicKey,
        dst,
        dstOwner.publicKey,
        mint,
        TOKEN_2022_PROGRAM_ID
      ),
      createMintToInstruction(
        mint,
        src,
        wallet.publicKey,
        1_000_000_000_000,
        [],
        TOKEN_2022_PROGRAM_ID
      )
    );
    await sendAndConfirmTransaction(connection, setup, [wallet.payer]);

    // resolves the hook's extra accounts (posterior, surprise log, pool) by
    // reading the on-chain ExtraAccountMetaList.
    const transferIx = await createTransferCheckedWithTransferHookInstruction(
      connection,
      src,
      mint,
      dst,
      wallet.publicKey,
      BigInt(1_000_000_000),
      decimals,
      [],
      "confirmed",
      TOKEN_2022_PROGRAM_ID
    );
    await sendAndConfirmTransaction(
      connection,
      new Transaction().add(transferIx),
      [wallet.payer]
    );

    const post = await program.account.posterior.fetch(posterior);
    assert.equal(post.updates.toNumber(), 1);
    // entropy must have fallen below the 8-bit maximum after one observation
    assert.ok(post.lastEntropyFp.lt(new anchor.BN("8000000000000")));
  });
});
