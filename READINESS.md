# Flowdren readiness review — 17 November 2026

Scope: current local repository, including the pre-existing untracked api/ directory. This is a source review and attempted verification, not a security audit or proof of deployment.

## Current state

Flowdren is an Anchor treasury/payroll prototype. Implemented instructions cover vault creation, token deposits, payment streams, withdrawals, pause/cancel, immediate payouts, approver configuration, and approval-gated payouts. There are 17 TypeScript integration test cases. Their presence does not establish that they pass.

There is no frontend, wallet connection flow, deployment automation, CI, or submission README in this checkout. Anchor is configured for localnet only. No live deployment was verified. The API contains helper modules but lacks its declared src/main.rs entry point and HTTP routing. Its instructions module imports spl_token without a declared direct dependency.

## Blocking correctness findings

1. **Payout accounting can overcommit funds.** create_stream and create_one_time_payout use total_deposited - total_allocated. Immediate payouts increase total_withdrawn but do not reduce that availability; deferred execution releases allocation without reducing deposits. Example: deposit 100, pay 60 immediately, then reserve a 100-token stream: the reservation can pass with only 40 tokens left. Define consistent accounting for remaining reservations versus historical withdrawals and test mixed operations.
2. **Open-ended streams have no stored funding cap.** Creation reserves the then-unallocated balance, but subsequent vesting grows without that bound. Such a stream can consume later deposits or funds intended for other streams. Store and enforce a per-stream budget, or omit open-ended streams from the MVP.
3. **Cancellation destroys unpaid recipient entitlement.** CancelStream closes the stream account, while cancel_stream transfers no vested-but-unwithdrawn amount and retains that vested allocation for bounded streams. Settle owed funds or preserve a claimable terminal account. The current test only checks that allocation decreases.
4. **Scheduling is inconsistent.** Immediate payouts ignore a future execute_after. For approval-gated payouts, reaching the threshold before execute_after errors and rolls back the final approval. Separate approval recording from execution readiness and test both paths.
5. **Pause semantics need a decision.** Pause only blocks withdrawal; vesting continues because start time is unchanged. Document this as a withdrawal freeze or implement actual accrual pause.

Additional gaps: stream PDA permits only one active stream per vault/recipient; approval settings are mutable for existing payouts; account fetchers skip discriminators without validating account owner/type. Decide intended policy and add negative authorization, invalid-account, boundary-time, and accounting tests.

## Verification

- TypeScript check (`node_modules\\.bin\\tsc.cmd --noEmit`): failed because @types/chai and @types/mocha are missing.
- API check (`cargo check --manifest-path api/Cargo.toml`): failed because api is beneath the workspace but is neither a member nor excluded. Missing main.rs remains an additional source-level blocker.
- `anchor test`: attempted; no result obtained during review. Do not treat existing target/ or test-ledger/ directories as evidence of passing tests.

## Focused plan for November 27

- Nov 17–19: resolve accounting/cancellation/scheduling, choose bounded streams for MVP, add regression tests, restore reproducible build/test tooling.
- Nov 20–23: wallet-signed web app: connect, initialize vault, deposit test tokens, create bounded stream, recipient withdrawal, approval queue and execution. Direct Anchor client calls can avoid making the incomplete API a demo dependency.
- Nov 24–25: devnet deployment and test-mint setup; demonstrate the complete journey with separate authority, recipient, and approver wallets. Verify balances and transaction links.
- Nov 26–27: clean-install verification, README, architecture explanation, screenshots, hosted demo, short demo video, and submission narrative. Leave time for failures.

This is an aggressive MVP plan, not a promise of production readiness. Production use requires deeper invariant/security review, operational monitoring and deployment/key-management procedures beyond this demo scope.

Acceptance gate: a new reviewer can set up the repo, run passing checks, open the demo, and complete the funded-vault-to-recipient journey with verifiable transactions. No real customer funds for the demo.

## Hackathon candidate

Official Dev3pack page confirms a worldwide hybrid student Solana hackathon on **November 27–29, 2026** (November 27 is the start, not the advertised end). Application links are available at https://hack.dev3pack.xyz/ . FAQ: https://hack.dev3pack.xyz/faq . Checked November 17.

Student/team eligibility, acceptance of pre-existing work, exact submission cutoff/timezone, and required artifacts remain unverified: retrieved FAQ text exposed questions but not answers. Confirm these before treating Flowdren as eligible. No application or submission was made.
