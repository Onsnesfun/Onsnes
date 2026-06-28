// Anchor runs this after `anchor deploy`. Customise to initialise the posterior
// and the extra-account-meta list for your live mint if you want a one-shot
// bootstrap. By default it just confirms the provider wiring.
import * as anchor from "@coral-xyz/anchor";

module.exports = async function (provider: anchor.AnchorProvider) {
  anchor.setProvider(provider);
  console.log("onsnes deploy migration — provider:", provider.connection.rpcEndpoint);
  // Example bootstrap (uncomment + fill in your mint + pool):
  //
  // const program = anchor.workspace.Onsnes as anchor.Program;
  // const mint = new anchor.web3.PublicKey("<your token-2022 mint>");
  // const pool = new anchor.web3.PublicKey("<your meteora dlmm pool>");
  // await program.methods.initialize(pool).accounts({ mint }).rpc();
  // await program.methods.initializeExtraAccountMetaList().accounts({ mint }).rpc();
};
