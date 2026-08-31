import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { expect } from "chai";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAssociatedTokenAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";

import { Flowdren } from "../target/types/flowdren";

describe("flowdren", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Flowdren as Program<Flowdren>;
  const authority = provider.wallet as anchor.Wallet;

  let usdcMint: anchor.web3.PublicKey;
  let authorityUsdcTokenAccount: anchor.web3.PublicKey;
  let vault: anchor.web3.PublicKey;
  let vaultUsdcTokenAccount: anchor.web3.PublicKey;

  before(async () => {
    usdcMint = await createMint(
      provider.connection,
      authority.payer,
      authority.publicKey,
      null,
      6,
    );

    authorityUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      authority.publicKey,
    );

    await mintTo(
      provider.connection,
      authority.payer,
      usdcMint,
      authorityUsdcTokenAccount,
      authority.publicKey,
      1_000_000_000,
    );

    [vault] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), authority.publicKey.toBuffer()],
      program.programId,
    );

    [vaultUsdcTokenAccount] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault-usdc"), vault.toBuffer()],
      program.programId,
    );
  });

  it("initializes a company vault", async () => {
    await program.methods
      .initializeVault()
      .accounts({
        authority: authority.publicKey,
        usdcMint,
        vault,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const account = await program.account.vault.fetch(vault);

    expect(account.authority.toBase58()).to.equal(authority.publicKey.toBase58());
    expect(account.usdcMint.toBase58()).to.equal(usdcMint.toBase58());
    expect(account.usdcTokenAccount.toBase58()).to.equal(vaultUsdcTokenAccount.toBase58());
    expect(account.totalDeposited.toNumber()).to.equal(0);
    expect(account.totalAllocated.toNumber()).to.equal(0);
    expect(account.totalWithdrawn.toNumber()).to.equal(0);

    const tokenAccount = await getAccount(provider.connection, vaultUsdcTokenAccount);
    expect(tokenAccount.owner.toBase58()).to.equal(vault.toBase58());
    expect(tokenAccount.mint.toBase58()).to.equal(usdcMint.toBase58());
  });

  it("deposits USDC into the vault token account", async () => {
    const amount = new anchor.BN(250_000_000);

    await program.methods
      .deposit(amount)
      .accounts({
        authority: authority.publicKey,
        vault,
        authorityUsdcTokenAccount,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vault);
    const vaultTokenAccount = await getAccount(provider.connection, vaultUsdcTokenAccount);
    const authorityTokenAccount = await getAccount(provider.connection, authorityUsdcTokenAccount);

    expect(vaultAccount.totalDeposited.toNumber()).to.equal(amount.toNumber());
    expect(vaultTokenAccount.amount).to.equal(BigInt(amount.toNumber()));
    expect(authorityTokenAccount.amount).to.equal(750_000_000n);
  });

  it("creates a stream with end timestamp", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey,
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stream"), vault.toBuffer(), recipient.publicKey.toBuffer()],
      program.programId,
    );

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now + 10);
    const endTimestamp = new anchor.BN(now + 20);
    const ratePerSecond = new anchor.BN(1_000_000);

    await program.methods
      .createStream(ratePerSecond, startTimestamp, endTimestamp)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        vault,
        stream,
        usdcTokenAccount: vaultUsdcTokenAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const streamAccount = await program.account.stream.fetch(stream);
    expect(streamAccount.vault.toBase58()).to.equal(vault.toBase58());
    expect(streamAccount.recipient.toBase58()).to.equal(recipient.publicKey.toBase58());
    expect(streamAccount.ratePerSecond.toNumber()).to.equal(ratePerSecond.toNumber());
    expect(streamAccount.startTimestamp.toNumber()).to.equal(startTimestamp.toNumber());
    expect(streamAccount.endTimestamp!.toNumber()).to.equal(endTimestamp.toNumber());
    expect(streamAccount.totalWithdrawn.toNumber()).to.equal(0);
    expect(streamAccount.paused).to.equal(false);
  });

  it("withdraws from stream mid-stream", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey,
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stream"), vault.toBuffer(), recipient.publicKey.toBuffer()],
      program.programId,
    );

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now);
    const endTimestamp = new anchor.BN(now + 20);
    const ratePerSecond = new anchor.BN(1_000_000);

    await program.methods
      .createStream(ratePerSecond, startTimestamp, endTimestamp)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        vault,
        stream,
        usdcTokenAccount: vaultUsdcTokenAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    await new Promise(resolve => setTimeout(resolve, 5000));

    await program.methods
      .withdrawFromStream()
      .accounts({
        recipient: recipient.publicKey,
        stream,
        vault,
        authority: authority.publicKey,
        recipientUsdcTokenAccount,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([recipient])
      .rpc();

    const streamAccount = await program.account.stream.fetch(stream);
    const recipientTokenAccount = await getAccount(provider.connection, recipientUsdcTokenAccount);

    expect(streamAccount.totalWithdrawn.toNumber()).to.be.greaterThan(0);
    expect(recipientTokenAccount.amount).to.be.greaterThan(0n);
  });

  it("withdraws after stream end caps at total", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey,
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stream"), vault.toBuffer(), recipient.publicKey.toBuffer()],
      program.programId,
    );

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now);
    const endTimestamp = new anchor.BN(now + 5);
    const ratePerSecond = new anchor.BN(1_000_000);

    await program.methods
      .createStream(ratePerSecond, startTimestamp, endTimestamp)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        vault,
        stream,
        usdcTokenAccount: vaultUsdcTokenAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    await new Promise(resolve => setTimeout(resolve, 6000));

    await program.methods
      .withdrawFromStream()
      .accounts({
        recipient: recipient.publicKey,
        stream,
        vault,
        authority: authority.publicKey,
        recipientUsdcTokenAccount,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([recipient])
      .rpc();

    const streamAccount = await program.account.stream.fetch(stream);
    const totalStreamAmount = ratePerSecond.mul(endTimestamp.sub(startTimestamp));

    expect(streamAccount.totalWithdrawn.toNumber()).to.be.lte(totalStreamAmount.toNumber());
  });

  it("cancels stream and refunds correctly", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey,
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stream"), vault.toBuffer(), recipient.publicKey.toBuffer()],
      program.programId,
    );

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now);
    const endTimestamp = new anchor.BN(now + 20);
    const ratePerSecond = new anchor.BN(1_000_000);

    await program.methods
      .createStream(ratePerSecond, startTimestamp, endTimestamp)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        vault,
        stream,
        usdcTokenAccount: vaultUsdcTokenAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    await new Promise(resolve => setTimeout(resolve, 3000));

    const vaultBefore = await program.account.vault.fetch(vault);

    await program.methods
      .cancelStream()
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        stream,
        vault,
      })
      .rpc();

    const vaultAfter = await program.account.vault.fetch(vault);

    expect(vaultAfter.totalAllocated.toNumber()).to.be.lessThan(vaultBefore.totalAllocated.toNumber());
  });

  it("pauses and unpauses a stream", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey,
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("stream"), vault.toBuffer(), recipient.publicKey.toBuffer()],
      program.programId,
    );

    const now = Math.floor(Date.now() / 1000);
    const startTimestamp = new anchor.BN(now + 10);
    const endTimestamp = new anchor.BN(now + 20);
    const ratePerSecond = new anchor.BN(1_000_000);

    await program.methods
      .createStream(ratePerSecond, startTimestamp, endTimestamp)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        vault,
        stream,
        usdcTokenAccount: vaultUsdcTokenAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    await program.methods
      .pauseStream(true)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        stream,
        vault,
      })
      .rpc();

    let streamAccount = await program.account.stream.fetch(stream);
    expect(streamAccount.paused).to.equal(true);

    await program.methods
      .pauseStream(false)
      .accounts({
        authority: authority.publicKey,
        recipient: recipient.publicKey,
        stream,
        vault,
      })
      .rpc();

    streamAccount = await program.account.stream.fetch(stream);
    expect(streamAccount.paused).to.equal(false);
  });
});
