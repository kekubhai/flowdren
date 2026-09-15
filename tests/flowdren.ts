import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import { expect } from "chai";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAssociatedTokenAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";

import { Flowdren } from "../target/types/flowdren";

// ─── Helpers ──────────────────────────────────────────────────────────────────

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Returns the current on-chain block time (avoids local-clock drift issues). */
async function chainTime(
  connection: anchor.web3.Connection
): Promise<number> {
  const slot = await connection.getSlot();
  const blockTime = await connection.getBlockTime(slot);
  if (blockTime === null)
    throw new Error(`could not fetch block time for slot ${slot}`);
  return blockTime;
}

/** Derive a PendingPayout PDA for the given vault + payout_id. */
function pendingPayoutPda(
  vault: anchor.web3.PublicKey,
  payoutId: BN,
  programId: anchor.web3.PublicKey
): anchor.web3.PublicKey {
  const idBytes = Buffer.alloc(8);
  idBytes.writeBigUInt64LE(BigInt(payoutId.toString()));
  const [pda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("pending-payout"), vault.toBuffer(), idBytes],
    programId
  );
  return pda;
}

// ─── Test suite ───────────────────────────────────────────────────────────────

describe("flowdren", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Flowdren as Program<Flowdren>;
  const authority = provider.wallet as anchor.Wallet;

  let usdcMint: anchor.web3.PublicKey;
  let authorityUsdcTokenAccount: anchor.web3.PublicKey;
  let vault: anchor.web3.PublicKey;
  let vaultUsdcTokenAccount: anchor.web3.PublicKey;

  // ── Shared setup ────────────────────────────────────────────────────────────

  before(async () => {
    usdcMint = await createMint(
      provider.connection,
      authority.payer,
      authority.publicKey,
      null,
      6
    );

    authorityUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      authority.publicKey
    );

    await mintTo(
      provider.connection,
      authority.payer,
      usdcMint,
      authorityUsdcTokenAccount,
      authority.publicKey,
      2_000_000_000 // 2,000 USDC
    );

    [vault] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), authority.publicKey.toBuffer()],
      program.programId
    );

    [vaultUsdcTokenAccount] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault-usdc"), vault.toBuffer()],
      program.programId
    );
  });

  // ── Original stream tests ────────────────────────────────────────────────────

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

    expect(account.authority.toBase58()).to.equal(
      authority.publicKey.toBase58()
    );
    expect(account.usdcMint.toBase58()).to.equal(usdcMint.toBase58());
    expect(account.usdcTokenAccount.toBase58()).to.equal(
      vaultUsdcTokenAccount.toBase58()
    );
    expect(account.totalDeposited.toNumber()).to.equal(0);
    expect(account.totalAllocated.toNumber()).to.equal(0);
    expect(account.totalWithdrawn.toNumber()).to.equal(0);
    // New fields initialised to sensible defaults
    expect(account.approverCount).to.equal(0);
    expect(account.approvalThreshold).to.equal(0);
    // payout_threshold defaults to u64::MAX → all payouts execute immediately
    expect(account.payoutThreshold.toString()).to.equal(
      "18446744073709551615"
    );

    const tokenAccount = await getAccount(
      provider.connection,
      vaultUsdcTokenAccount
    );
    expect(tokenAccount.owner.toBase58()).to.equal(vault.toBase58());
    expect(tokenAccount.mint.toBase58()).to.equal(usdcMint.toBase58());
  });

  it("deposits USDC into the vault token account", async () => {
    const amount = new BN(500_000_000); // 500 USDC

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
    const vaultToken = await getAccount(
      provider.connection,
      vaultUsdcTokenAccount
    );

    expect(vaultAccount.totalDeposited.toNumber()).to.equal(amount.toNumber());
    expect(vaultToken.amount).to.equal(BigInt(amount.toNumber()));
  });

  it("creates a stream with end timestamp", async () => {
    const recipient = anchor.web3.Keypair.generate();
    await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("stream"),
        vault.toBuffer(),
        recipient.publicKey.toBuffer(),
      ],
      program.programId
    );

    const now = await chainTime(provider.connection);
    const startTimestamp = new BN(now + 10);
    const endTimestamp = new BN(now + 20);
    const ratePerSecond = new BN(1_000_000);

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
    expect(streamAccount.recipient.toBase58()).to.equal(
      recipient.publicKey.toBase58()
    );
    expect(streamAccount.ratePerSecond.toNumber()).to.equal(
      ratePerSecond.toNumber()
    );
    expect(streamAccount.startTimestamp.toNumber()).to.equal(
      startTimestamp.toNumber()
    );
    expect(streamAccount.endTimestamp!.toNumber()).to.equal(
      endTimestamp.toNumber()
    );
    expect(streamAccount.totalWithdrawn.toNumber()).to.equal(0);
    expect(streamAccount.paused).to.equal(false);
  });

  it("withdraws from stream mid-stream", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("stream"),
        vault.toBuffer(),
        recipient.publicKey.toBuffer(),
      ],
      program.programId
    );

    const now = await chainTime(provider.connection);
    const startTimestamp = new BN(now + 2);
    const endTimestamp = new BN(now + 22);
    const ratePerSecond = new BN(1_000_000);

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

    await sleep(6000);

    await program.methods
      .withdrawFromStream()
      .accounts({
        recipient: recipient.publicKey,
        stream,
        vault,
        recipientUsdcTokenAccount,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([recipient])
      .rpc();

    const streamAccount = await program.account.stream.fetch(stream);
    const recipientToken = await getAccount(
      provider.connection,
      recipientUsdcTokenAccount
    );

    expect(streamAccount.totalWithdrawn.toNumber()).to.be.greaterThan(0);
    expect(Number(recipientToken.amount)).to.be.greaterThan(0);
  });

  it("withdraws after stream end caps at total", async () => {
    const recipient = anchor.web3.Keypair.generate();
    const recipientUsdcTokenAccount = await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("stream"),
        vault.toBuffer(),
        recipient.publicKey.toBuffer(),
      ],
      program.programId
    );

    const now = await chainTime(provider.connection);
    const startTimestamp = new BN(now + 2);
    const endTimestamp = new BN(now + 7);
    const ratePerSecond = new BN(1_000_000);

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

    await sleep(10000);

    await program.methods
      .withdrawFromStream()
      .accounts({
        recipient: recipient.publicKey,
        stream,
        vault,
        recipientUsdcTokenAccount,
        usdcTokenAccount: vaultUsdcTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([recipient])
      .rpc();

    const streamAccount = await program.account.stream.fetch(stream);
    const totalStreamAmount = ratePerSecond.mul(
      endTimestamp.sub(startTimestamp)
    );

    expect(streamAccount.totalWithdrawn.toNumber()).to.be.lte(
      totalStreamAmount.toNumber()
    );
  });

  it("cancels stream and refunds correctly", async () => {
    const recipient = anchor.web3.Keypair.generate();
    await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("stream"),
        vault.toBuffer(),
        recipient.publicKey.toBuffer(),
      ],
      program.programId
    );

    const now = await chainTime(provider.connection);
    const startTimestamp = new BN(now + 2);
    const endTimestamp = new BN(now + 22);
    const ratePerSecond = new BN(1_000_000);

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

    await sleep(3000);

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

    expect(vaultAfter.totalAllocated.toNumber()).to.be.lessThan(
      vaultBefore.totalAllocated.toNumber()
    );
  });

  it("pauses and unpauses a stream", async () => {
    const recipient = anchor.web3.Keypair.generate();
    await createAssociatedTokenAccount(
      provider.connection,
      authority.payer,
      usdcMint,
      recipient.publicKey
    );

    const [stream] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("stream"),
        vault.toBuffer(),
        recipient.publicKey.toBuffer(),
      ],
      program.programId
    );

    const now = await chainTime(provider.connection);
    const startTimestamp = new BN(now + 10);
    const endTimestamp = new BN(now + 20);
    const ratePerSecond = new BN(1_000_000);

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

  // ── Approver configuration tests ─────────────────────────────────────────────

  describe("approver management", () => {
    const approver1 = anchor.web3.Keypair.generate();
    const approver2 = anchor.web3.Keypair.generate();

    it("adds approvers to the vault", async () => {
      await program.methods
        .addApprover(approver1.publicKey)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      await program.methods
        .addApprover(approver2.publicKey)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const v = await program.account.vault.fetch(vault);
      expect(v.approverCount).to.equal(2);
      expect(v.approvers[0].toBase58()).to.equal(
        approver1.publicKey.toBase58()
      );
      expect(v.approvers[1].toBase58()).to.equal(
        approver2.publicKey.toBase58()
      );
    });

    it("rejects duplicate approver registration", async () => {
      try {
        await program.methods
          .addApprover(approver1.publicKey)
          .accounts({ authority: authority.publicKey, vault })
          .rpc();
        expect.fail("should have thrown");
      } catch (err: any) {
        expect(err.message).to.include("AlreadyAnApprover");
      }
    });

    it("sets approval threshold", async () => {
      await program.methods
        .setApprovalThreshold(1) // 1-of-2
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const v = await program.account.vault.fetch(vault);
      expect(v.approvalThreshold).to.equal(1);
    });

    it("rejects threshold higher than approver count", async () => {
      try {
        await program.methods
          .setApprovalThreshold(5) // only 2 approvers
          .accounts({ authority: authority.publicKey, vault })
          .rpc();
        expect.fail("should have thrown");
      } catch (err: any) {
        expect(err.message).to.include("ThresholdTooHigh");
      }
    });
  });

  // ── One-time payout tests ─────────────────────────────────────────────────────

  describe("one-time payouts", () => {
    let recipient: anchor.web3.Keypair;
    let recipientUsdcTokenAccount: anchor.web3.PublicKey;
    let approver1: anchor.web3.Keypair;
    let approver2: anchor.web3.Keypair;

    // Counters so each sub-test gets a unique payout_id
    let payoutIdCounter = 100;

    /** Airdrop SOL to a keypair so it can sign transactions */
    async function fundKeypair(kp: anchor.web3.Keypair, lamports = 1e9) {
      const sig = await provider.connection.requestAirdrop(
        kp.publicKey,
        lamports
      );
      await provider.connection.confirmTransaction(sig);
    }

    before(async () => {
      // Fresh recipient keypair + token account for this suite
      recipient = anchor.web3.Keypair.generate();
      recipientUsdcTokenAccount = await createAssociatedTokenAccount(
        provider.connection,
        authority.payer,
        usdcMint,
        recipient.publicKey
      );

      // Reuse the same approver keypairs that are registered on the vault.
      // The approver management tests above registered two approvers; fetch
      // them by reading the vault state so this suite stays independent.
      const v = await program.account.vault.fetch(vault);
      // approver1 / approver2 correspond to vault.approvers[0] and [1].
      // We need the signing keypairs — generate new ones and re-register
      // fresh keypairs so this suite owns the private keys.
      approver1 = anchor.web3.Keypair.generate();
      approver2 = anchor.web3.Keypair.generate();

      await fundKeypair(approver1);
      await fundKeypair(approver2);

      // Register the new keypairs (vault already has approvers[0/1] from the
      // management suite; we add two more so we have indices 2 and 3).
      await program.methods
        .addApprover(approver1.publicKey)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();
      await program.methods
        .addApprover(approver2.publicKey)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      // Ensure the vault has enough funds (deposit more to cover tests)
      await program.methods
        .deposit(new BN(500_000_000))
        .accounts({
          authority: authority.publicKey,
          vault,
          authorityUsdcTokenAccount,
          usdcTokenAccount: vaultUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
    });

    // ── Test 1: payout below threshold executes immediately ─────────────────

    it("payout below payout_threshold executes immediately", async () => {
      // Set payout_threshold high enough so our small amount goes through
      await program.methods
        .setPayoutThreshold(new BN(100_000_000)) // 100 USDC threshold
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const payoutId = new BN(payoutIdCounter++);
      const amount = new BN(1_000_000); // 1 USDC — well below 100 USDC

      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      const recipientBefore = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );

      await program.methods
        .createOneTimePayout(payoutId, amount, new BN(0))
        .accounts({
          authority: authority.publicKey,
          recipient: recipient.publicKey,
          vault,
          pendingPayout,
          usdcTokenAccount: vaultUsdcTokenAccount,
          recipientUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      const recipientAfter = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );

      // Funds should have arrived immediately
      expect(
        Number(recipientAfter.amount) - Number(recipientBefore.amount)
      ).to.equal(amount.toNumber());

      // PendingPayout account should be marked executed
      const payoutAccount = await program.account.pendingPayout.fetch(
        pendingPayout
      );
      expect(payoutAccount.executed).to.equal(true);
      expect(payoutAccount.amount.toNumber()).to.equal(amount.toNumber());
    });

    // ── Test 2: payout above threshold requires approvals ──────────────────

    it("payout above payout_threshold requires approvals before transfer", async () => {
      // Ensure threshold requires 1 approval from our newly added approvers
      // Vault now has 4 approvers (2 from mgmt suite + 2 from before())
      await program.methods
        .setApprovalThreshold(1)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const payoutId = new BN(payoutIdCounter++);
      const amount = new BN(200_000_000); // 200 USDC — above 100 USDC threshold

      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      const recipientBefore = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );
      const vaultBefore = await program.account.vault.fetch(vault);

      // Create the payout (deferred — no immediate transfer)
      await program.methods
        .createOneTimePayout(payoutId, amount, new BN(0))
        .accounts({
          authority: authority.publicKey,
          recipient: recipient.publicKey,
          vault,
          pendingPayout,
          usdcTokenAccount: vaultUsdcTokenAccount,
          recipientUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      // Funds must NOT have moved yet
      const recipientMid = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );
      expect(Number(recipientMid.amount)).to.equal(
        Number(recipientBefore.amount),
        "no transfer should happen before approval"
      );

      // Funds should be reserved in total_allocated
      const vaultMid = await program.account.vault.fetch(vault);
      expect(vaultMid.totalAllocated.toNumber()).to.equal(
        vaultBefore.totalAllocated.toNumber() + amount.toNumber()
      );

      // PendingPayout is pending, not executed
      let payoutAccount = await program.account.pendingPayout.fetch(
        pendingPayout
      );
      expect(payoutAccount.executed).to.equal(false);
      expect(payoutAccount.approvedMask).to.equal(0);

      // approver1 approves → threshold reached → auto-executes (1-of-4 needed)
      // approver1 was the 3rd approver added (index 2 in the vault)
      await program.methods
        .approvePayout()
        .accounts({
          approver: approver1.publicKey,
          vault,
          pendingPayout,
          recipient: recipient.publicKey,
          recipientUsdcTokenAccount,
          usdcTokenAccount: vaultUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([approver1])
        .rpc();

      // Funds should now have arrived
      const recipientAfter = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );
      expect(
        Number(recipientAfter.amount) - Number(recipientBefore.amount)
      ).to.equal(amount.toNumber());

      // PendingPayout is now executed
      payoutAccount = await program.account.pendingPayout.fetch(pendingPayout);
      expect(payoutAccount.executed).to.equal(true);

      // total_allocated should be back down
      const vaultAfter = await program.account.vault.fetch(vault);
      expect(vaultAfter.totalAllocated.toNumber()).to.equal(
        vaultBefore.totalAllocated.toNumber()
      );
    });

    // ── Test 3: cannot execute with insufficient approvals ─────────────────

    it("execute_pending_payout fails with insufficient approvals", async () => {
      // Raise threshold to 2-of-4 approvers
      await program.methods
        .setApprovalThreshold(2)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const payoutId = new BN(payoutIdCounter++);
      const amount = new BN(200_000_000); // above payout_threshold

      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      // Create deferred payout
      await program.methods
        .createOneTimePayout(payoutId, amount, new BN(0))
        .accounts({
          authority: authority.publicKey,
          recipient: recipient.publicKey,
          vault,
          pendingPayout,
          usdcTokenAccount: vaultUsdcTokenAccount,
          recipientUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      // Only approver1 approves (1 of 2 required) — should NOT auto-execute
      await program.methods
        .approvePayout()
        .accounts({
          approver: approver1.publicKey,
          vault,
          pendingPayout,
          recipient: recipient.publicKey,
          recipientUsdcTokenAccount,
          usdcTokenAccount: vaultUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([approver1])
        .rpc();

      // Payout must still be pending
      const payoutAccount = await program.account.pendingPayout.fetch(
        pendingPayout
      );
      expect(payoutAccount.executed).to.equal(false);

      // Attempting manual execution without enough approvals must fail
      try {
        await program.methods
          .executePendingPayout()
          .accounts({
            executor: authority.publicKey,
            vault,
            pendingPayout,
            recipient: recipient.publicKey,
            recipientUsdcTokenAccount,
            usdcTokenAccount: vaultUsdcTokenAccount,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc();
        expect.fail("should have thrown InsufficientApprovals");
      } catch (err: any) {
        expect(err.message).to.include("InsufficientApprovals");
      }
    });

    // ── Test 4: execute_pending_payout succeeds once threshold met ──────────

    it("execute_pending_payout succeeds after all required approvals collected", async () => {
      // Reuse the payout from the previous test (still pending after 1/2 approval)
      // Find the payout_id — it's payoutIdCounter - 1 (last created)
      const payoutId = new BN(payoutIdCounter - 1);
      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      const recipientBefore = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );

      // approver2 adds their approval → 2/2 required
      await program.methods
        .approvePayout()
        .accounts({
          approver: approver2.publicKey,
          vault,
          pendingPayout,
          recipient: recipient.publicKey,
          recipientUsdcTokenAccount,
          usdcTokenAccount: vaultUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([approver2])
        .rpc();

      // Auto-execution should have kicked in; funds should have moved
      const recipientAfter = await getAccount(
        provider.connection,
        recipientUsdcTokenAccount
      );
      expect(Number(recipientAfter.amount)).to.be.greaterThan(
        Number(recipientBefore.amount)
      );

      const payoutAccount = await program.account.pendingPayout.fetch(
        pendingPayout
      );
      expect(payoutAccount.executed).to.equal(true);
    });

    // ── Test 5: duplicate approval is rejected ────────────────────────────

    it("approve_payout rejects duplicate approval from same approver", async () => {
      // Reset threshold to 2 and create a fresh payout
      await program.methods
        .setApprovalThreshold(2)
        .accounts({ authority: authority.publicKey, vault })
        .rpc();

      const payoutId = new BN(payoutIdCounter++);
      const amount = new BN(200_000_000);
      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      await program.methods
        .createOneTimePayout(payoutId, amount, new BN(0))
        .accounts({
          authority: authority.publicKey,
          recipient: recipient.publicKey,
          vault,
          pendingPayout,
          usdcTokenAccount: vaultUsdcTokenAccount,
          recipientUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      // First approval succeeds
      await program.methods
        .approvePayout()
        .accounts({
          approver: approver1.publicKey,
          vault,
          pendingPayout,
          recipient: recipient.publicKey,
          recipientUsdcTokenAccount,
          usdcTokenAccount: vaultUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([approver1])
        .rpc();

      // Second attempt by the same approver must fail
      try {
        await program.methods
          .approvePayout()
          .accounts({
            approver: approver1.publicKey,
            vault,
            pendingPayout,
            recipient: recipient.publicKey,
            recipientUsdcTokenAccount,
            usdcTokenAccount: vaultUsdcTokenAccount,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .signers([approver1])
          .rpc();
        expect.fail("should have thrown AlreadyApproved");
      } catch (err: any) {
        expect(err.message).to.include("AlreadyApproved");
      }
    });

    // ── Test 6: non-approver cannot approve ──────────────────────────────

    it("approve_payout rejects a signer who is not a registered approver", async () => {
      const payoutId = new BN(payoutIdCounter++);
      const amount = new BN(200_000_000);
      const pendingPayout = pendingPayoutPda(vault, payoutId, program.programId);

      await program.methods
        .createOneTimePayout(payoutId, amount, new BN(0))
        .accounts({
          authority: authority.publicKey,
          recipient: recipient.publicKey,
          vault,
          pendingPayout,
          usdcTokenAccount: vaultUsdcTokenAccount,
          recipientUsdcTokenAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      const stranger = anchor.web3.Keypair.generate();
      const sig = await provider.connection.requestAirdrop(
        stranger.publicKey,
        1e9
      );
      await provider.connection.confirmTransaction(sig);

      try {
        await program.methods
          .approvePayout()
          .accounts({
            approver: stranger.publicKey,
            vault,
            pendingPayout,
            recipient: recipient.publicKey,
            recipientUsdcTokenAccount,
            usdcTokenAccount: vaultUsdcTokenAccount,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .signers([stranger])
          .rpc();
        expect.fail("should have thrown NotAnApprover");
      } catch (err: any) {
        expect(err.message).to.include("NotAnApprover");
      }
    });
  });
});
