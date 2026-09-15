use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("D83V15P2gLAH12ESvRav9oxceCBckPM6ThQMY8C1YQFr");

// ─── Maximum approvers per vault ───────────────────────────────────────────
pub const MAX_APPROVERS: usize = 5;

#[program]
pub mod flowdren {
    use super::*;

    // ── Vault management ───────────────────────────────────────────────────

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.usdc_mint = ctx.accounts.usdc_mint.key();
        vault.usdc_token_account = ctx.accounts.usdc_token_account.key();
        vault.total_deposited = 0;
        vault.total_allocated = 0;
        vault.total_withdrawn = 0;
        vault.bump = ctx.bumps.vault;
        vault.token_account_bump = ctx.bumps.usdc_token_account;
        // Approver / threshold defaults – threshold of 0 means all payouts
        // execute immediately regardless of amount until the authority
        // explicitly sets a payout_threshold > 0.
        vault.approvers = [Pubkey::default(); MAX_APPROVERS];
        vault.approver_count = 0;
        vault.approval_threshold = 0;
        vault.payout_threshold = u64::MAX; // no approvals needed until configured
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, FlowdrenError::InvalidAmount);

        token::transfer(ctx.accounts.transfer_ctx(), amount)?;

        let vault = &mut ctx.accounts.vault;
        vault.total_deposited = vault
            .total_deposited
            .checked_add(amount)
            .ok_or(FlowdrenError::MathOverflow)?;

        Ok(())
    }

    // ── Approver / threshold configuration (authority-only) ────────────────

    /// Add a new approver pubkey to the vault (max MAX_APPROVERS).
    pub fn add_approver(ctx: Context<ManageApprovers>, new_approver: Pubkey) -> Result<()> {
        let vault = &mut ctx.accounts.vault;

        require!(
            vault.approver_count < MAX_APPROVERS as u8,
            FlowdrenError::MaxApproversReached
        );

        // Prevent duplicates
        for i in 0..vault.approver_count as usize {
            require!(
                vault.approvers[i] != new_approver,
                FlowdrenError::AlreadyAnApprover
            );
        }

        let idx = vault.approver_count as usize;
        vault.approvers[idx] = new_approver;
        vault.approver_count = vault.approver_count.checked_add(1).unwrap();
        Ok(())
    }

    /// Set the number of approvals required before a pending payout executes.
    /// `threshold` must be ≤ current approver_count (and > 0 when approvers exist).
    pub fn set_approval_threshold(ctx: Context<ManageApprovers>, threshold: u8) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        require!(
            threshold <= vault.approver_count,
            FlowdrenError::ThresholdTooHigh
        );
        vault.approval_threshold = threshold;
        Ok(())
    }

    /// Set the minimum amount (exclusive) that triggers the approval flow.
    /// Payouts ≤ payout_threshold execute immediately; payouts > payout_threshold
    /// require `approval_threshold` approvals.
    pub fn set_payout_threshold(ctx: Context<ManageApprovers>, threshold: u64) -> Result<()> {
        ctx.accounts.vault.payout_threshold = threshold;
        Ok(())
    }

    // ── Streams ────────────────────────────────────────────────────────────

    pub fn create_stream(
        ctx: Context<CreateStream>,
        rate_per_second: u64,
        start_timestamp: i64,
        end_timestamp: Option<i64>,
    ) -> Result<()> {
        require!(rate_per_second > 0, FlowdrenError::InvalidAmount);

        let vault = &ctx.accounts.vault;
        let clock = Clock::get()?;

        require!(
            start_timestamp >= clock.unix_timestamp,
            FlowdrenError::InvalidTimestamp
        );

        let total_amount = if let Some(end) = end_timestamp {
            require!(end > start_timestamp, FlowdrenError::InvalidTimestamp);
            let duration = end
                .checked_sub(start_timestamp)
                .ok_or(FlowdrenError::MathOverflow)?;
            rate_per_second
                .checked_mul(duration as u64)
                .ok_or(FlowdrenError::MathOverflow)?
        } else {
            let unallocated = vault
                .total_deposited
                .checked_sub(vault.total_allocated)
                .ok_or(FlowdrenError::MathOverflow)?;
            require!(unallocated > 0, FlowdrenError::InsufficientBalance);
            unallocated
        };

        let unallocated = vault
            .total_deposited
            .checked_sub(vault.total_allocated)
            .ok_or(FlowdrenError::MathOverflow)?;

        require!(
            unallocated >= total_amount,
            FlowdrenError::InsufficientBalance
        );

        let stream = &mut ctx.accounts.stream;
        stream.vault = vault.key();
        stream.recipient = ctx.accounts.recipient.key();
        stream.rate_per_second = rate_per_second;
        stream.start_timestamp = start_timestamp;
        stream.end_timestamp = end_timestamp;
        stream.total_withdrawn = 0;
        stream.paused = false;
        stream.bump = ctx.bumps.stream;

        let vault = &mut ctx.accounts.vault;
        vault.total_allocated = vault
            .total_allocated
            .checked_add(total_amount)
            .ok_or(FlowdrenError::MathOverflow)?;

        Ok(())
    }

    pub fn withdraw_from_stream(ctx: Context<WithdrawFromStream>) -> Result<()> {
        let clock = Clock::get()?;
        let stream = &ctx.accounts.stream;

        require!(!stream.paused, FlowdrenError::StreamPaused);
        require!(
            clock.unix_timestamp >= stream.start_timestamp,
            FlowdrenError::StreamNotStarted
        );

        let elapsed = clock
            .unix_timestamp
            .checked_sub(stream.start_timestamp)
            .ok_or(FlowdrenError::MathOverflow)?;

        let vested = stream
            .rate_per_second
            .checked_mul(elapsed as u64)
            .ok_or(FlowdrenError::MathOverflow)?;

        let total_vested = if let Some(end) = stream.end_timestamp {
            let total_duration = end
                .checked_sub(stream.start_timestamp)
                .ok_or(FlowdrenError::MathOverflow)?;
            let total_amount = stream
                .rate_per_second
                .checked_mul(total_duration as u64)
                .ok_or(FlowdrenError::MathOverflow)?;
            vested.min(total_amount)
        } else {
            vested
        };

        let withdrawable = total_vested
            .checked_sub(stream.total_withdrawn)
            .ok_or(FlowdrenError::MathOverflow)?;

        require!(withdrawable > 0, FlowdrenError::NothingToWithdraw);

        let vault_bump = ctx.accounts.vault.bump;
        let vault_authority = ctx.accounts.vault.authority;
        let bump_seed = [vault_bump];
        let signer_seeds: &[&[u8]] = &[b"vault", vault_authority.as_ref(), &bump_seed];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.usdc_token_account.to_account_info(),
                    to: ctx.accounts.recipient_usdc_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[signer_seeds],
            ),
            withdrawable,
        )?;

        let stream = &mut ctx.accounts.stream;
        stream.total_withdrawn = stream
            .total_withdrawn
            .checked_add(withdrawable)
            .ok_or(FlowdrenError::MathOverflow)?;

        let vault = &mut ctx.accounts.vault;
        vault.total_withdrawn = vault
            .total_withdrawn
            .checked_add(withdrawable)
            .ok_or(FlowdrenError::MathOverflow)?;

        Ok(())
    }

    pub fn pause_stream(ctx: Context<PauseStream>, paused: bool) -> Result<()> {
        let stream = &mut ctx.accounts.stream;
        stream.paused = paused;
        Ok(())
    }

    pub fn cancel_stream(ctx: Context<CancelStream>) -> Result<()> {
        let stream = &mut ctx.accounts.stream;
        let vault = &mut ctx.accounts.vault;

        let clock = Clock::get()?;

        let elapsed = if clock.unix_timestamp > stream.start_timestamp {
            clock
                .unix_timestamp
                .checked_sub(stream.start_timestamp)
                .ok_or(FlowdrenError::MathOverflow)?
        } else {
            0
        };

        let vested = stream
            .rate_per_second
            .checked_mul(elapsed as u64)
            .ok_or(FlowdrenError::MathOverflow)?;

        let total_vested = if let Some(end) = stream.end_timestamp {
            let total_duration = end
                .checked_sub(stream.start_timestamp)
                .ok_or(FlowdrenError::MathOverflow)?;
            let total_amount = stream
                .rate_per_second
                .checked_mul(total_duration as u64)
                .ok_or(FlowdrenError::MathOverflow)?;
            vested.min(total_amount)
        } else {
            vested
        };

        let already_withdrawn = stream.total_withdrawn;
        let remaining_to_vest = if let Some(end) = stream.end_timestamp {
            let total_duration = end
                .checked_sub(stream.start_timestamp)
                .ok_or(FlowdrenError::MathOverflow)?;
            let total_amount = stream
                .rate_per_second
                .checked_mul(total_duration as u64)
                .ok_or(FlowdrenError::MathOverflow)?;
            total_amount
                .checked_sub(total_vested)
                .ok_or(FlowdrenError::MathOverflow)?
        } else {
            total_vested
                .checked_sub(already_withdrawn)
                .ok_or(FlowdrenError::MathOverflow)?
        };

        vault.total_allocated = vault
            .total_allocated
            .checked_sub(remaining_to_vest)
            .ok_or(FlowdrenError::MathOverflow)?;

        stream.paused = true;

        Ok(())
    }

    // ── One-time payouts ───────────────────────────────────────────────────

    /// Create a one-time invoice-style payout.
    ///
    /// - If `amount <= vault.payout_threshold` (or no approval threshold is
    ///   configured), the transfer executes immediately.
    /// - Otherwise a `PendingPayout` account is initialised and the funds are
    ///   reserved (added to `total_allocated`) until the required approvals
    ///   are collected.
    ///
    /// `payout_id`   – caller-chosen monotonic u64 used to derive the PDA so
    ///                 multiple pending payouts can coexist for the same vault.
    /// `execute_after` – optional earliest Unix timestamp for execution (0 =
    ///                   execute as soon as approvals are met).
    pub fn create_one_time_payout(
        ctx: Context<CreateOneTimePayout>,
        payout_id: u64,
        amount: u64,
        execute_after: i64,
    ) -> Result<()> {
        require!(amount > 0, FlowdrenError::InvalidAmount);

        let vault = &ctx.accounts.vault;

        // Check funds availability
        let unallocated = vault
            .total_deposited
            .checked_sub(vault.total_allocated)
            .ok_or(FlowdrenError::MathOverflow)?;
        require!(unallocated >= amount, FlowdrenError::InsufficientBalance);

        // Determine whether immediate execution applies.
        // Immediate if:  amount <= payout_threshold  OR  approval_threshold == 0
        let needs_approval =
            amount > vault.payout_threshold && vault.approval_threshold > 0;

        if !needs_approval {
            // ── Immediate path ──────────────────────────────────────────────
            let vault_bump = vault.bump;
            let vault_authority = vault.authority;
            let bump_seed = [vault_bump];
            let signer_seeds: &[&[u8]] = &[b"vault", vault_authority.as_ref(), &bump_seed];

            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.usdc_token_account.to_account_info(),
                        to: ctx.accounts.recipient_usdc_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    &[signer_seeds],
                ),
                amount,
            )?;

            let vault = &mut ctx.accounts.vault;
            vault.total_withdrawn = vault
                .total_withdrawn
                .checked_add(amount)
                .ok_or(FlowdrenError::MathOverflow)?;

            // Mark the PendingPayout (which was init-ed by the account
            // constraint) as already executed so it cannot be re-used.
            let payout = &mut ctx.accounts.pending_payout;
            payout.vault = ctx.accounts.vault.key();
            payout.recipient = ctx.accounts.recipient.key();
            payout.amount = amount;
            payout.payout_id = payout_id;
            payout.execute_after = execute_after;
            payout.approved_mask = 0xFF; // all bits set ⇒ trivially approved
            payout.executed = true;
            payout.bump = ctx.bumps.pending_payout;
        } else {
            // ── Deferred / approval path ────────────────────────────────────
            // Reserve the funds by increasing total_allocated.
            let vault = &mut ctx.accounts.vault;
            vault.total_allocated = vault
                .total_allocated
                .checked_add(amount)
                .ok_or(FlowdrenError::MathOverflow)?;

            let payout = &mut ctx.accounts.pending_payout;
            payout.vault = vault.key();
            payout.recipient = ctx.accounts.recipient.key();
            payout.amount = amount;
            payout.payout_id = payout_id;
            payout.execute_after = execute_after;
            payout.approved_mask = 0;
            payout.executed = false;
            payout.bump = ctx.bumps.pending_payout;
        }

        Ok(())
    }

    /// Called by an approver to register their approval for a pending payout.
    /// Automatically executes the transfer if the threshold is reached.
    pub fn approve_payout(ctx: Context<ApprovePayout>) -> Result<()> {
        let approver_key = ctx.accounts.approver.key();

        // Find the approver's index in the vault approvers list.
        let vault = &ctx.accounts.vault;
        let approver_idx = vault
            .approvers[..vault.approver_count as usize]
            .iter()
            .position(|k| *k == approver_key)
            .ok_or(FlowdrenError::NotAnApprover)?;

        let payout = &ctx.accounts.pending_payout;
        require!(!payout.executed, FlowdrenError::PayoutAlreadyExecuted);

        // Check not already approved by this approver
        let bit = 1u8 << approver_idx;
        require!(
            payout.approved_mask & bit == 0,
            FlowdrenError::AlreadyApproved
        );

        // Set the approval bit
        let payout = &mut ctx.accounts.pending_payout;
        payout.approved_mask |= bit;

        // Count approvals
        let approval_count = payout.approved_mask.count_ones() as u8;
        let threshold = ctx.accounts.vault.approval_threshold;

        if approval_count >= threshold {
            // ── Auto-execute ────────────────────────────────────────────────
            let clock = Clock::get()?;
            require!(
                clock.unix_timestamp >= payout.execute_after,
                FlowdrenError::PayoutNotReady
            );

            let amount = payout.amount;

            let vault_bump = ctx.accounts.vault.bump;
            let vault_authority = ctx.accounts.vault.authority;
            let bump_seed = [vault_bump];
            let signer_seeds: &[&[u8]] = &[b"vault", vault_authority.as_ref(), &bump_seed];

            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.usdc_token_account.to_account_info(),
                        to: ctx.accounts.recipient_usdc_token_account.to_account_info(),
                        authority: ctx.accounts.vault.to_account_info(),
                    },
                    &[signer_seeds],
                ),
                amount,
            )?;

            let payout = &mut ctx.accounts.pending_payout;
            payout.executed = true;

            let vault = &mut ctx.accounts.vault;
            vault.total_allocated = vault
                .total_allocated
                .checked_sub(amount)
                .ok_or(FlowdrenError::MathOverflow)?;
            vault.total_withdrawn = vault
                .total_withdrawn
                .checked_add(amount)
                .ok_or(FlowdrenError::MathOverflow)?;
        }

        Ok(())
    }

    /// Permissionless execution once threshold has been reached.
    /// Useful when the final `approve_payout` caller doesn't want to pay for
    /// the token transfer in the same transaction, or when `execute_after` is
    /// in the future.
    pub fn execute_pending_payout(ctx: Context<ExecutePendingPayout>) -> Result<()> {
        let payout = &ctx.accounts.pending_payout;
        require!(!payout.executed, FlowdrenError::PayoutAlreadyExecuted);

        let vault = &ctx.accounts.vault;
        let approval_count = payout.approved_mask.count_ones() as u8;
        require!(
            approval_count >= vault.approval_threshold,
            FlowdrenError::InsufficientApprovals
        );

        let clock = Clock::get()?;
        require!(
            clock.unix_timestamp >= payout.execute_after,
            FlowdrenError::PayoutNotReady
        );

        let amount = payout.amount;

        let vault_bump = vault.bump;
        let vault_authority = vault.authority;
        let bump_seed = [vault_bump];
        let signer_seeds: &[&[u8]] = &[b"vault", vault_authority.as_ref(), &bump_seed];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.usdc_token_account.to_account_info(),
                    to: ctx.accounts.recipient_usdc_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[signer_seeds],
            ),
            amount,
        )?;

        let payout = &mut ctx.accounts.pending_payout;
        payout.executed = true;

        let vault = &mut ctx.accounts.vault;
        vault.total_allocated = vault
            .total_allocated
            .checked_sub(amount)
            .ok_or(FlowdrenError::MathOverflow)?;
        vault.total_withdrawn = vault
            .total_withdrawn
            .checked_add(amount)
            .ok_or(FlowdrenError::MathOverflow)?;

        Ok(())
    }
}

// ─── Account contexts ────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = authority,
        space = Vault::SPACE,
        seeds = [b"vault", authority.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init,
        payer = authority,
        token::mint = usdc_mint,
        token::authority = vault,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        constraint = authority_usdc_token_account.owner == authority.key() @ FlowdrenError::InvalidTokenOwner,
        constraint = authority_usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint
    )]
    pub authority_usdc_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

impl<'info> Deposit<'info> {
    fn transfer_ctx(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let accounts = Transfer {
            from: self.authority_usdc_token_account.to_account_info(),
            to: self.usdc_token_account.to_account_info(),
            authority: self.authority.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), accounts)
    }
}

/// Authority-only operations that mutate approver/threshold config.
#[derive(Accounts)]
pub struct ManageApprovers<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority
    )]
    pub vault: Account<'info, Vault>,
}

#[derive(Accounts)]
pub struct CreateStream<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub recipient: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init,
        payer = authority,
        space = Stream::SPACE,
        seeds = [b"stream", vault.key().as_ref(), recipient.key().as_ref()],
        bump
    )]
    pub stream: Account<'info, Stream>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawFromStream<'info> {
    #[account(mut)]
    pub recipient: Signer<'info>,
    #[account(
        mut,
        seeds = [b"stream", vault.key().as_ref(), recipient.key().as_ref()],
        bump = stream.bump,
        has_one = vault,
        has_one = recipient
    )]
    pub stream: Account<'info, Stream>,
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        constraint = recipient_usdc_token_account.owner == recipient.key() @ FlowdrenError::InvalidTokenOwner,
        constraint = recipient_usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint
    )]
    pub recipient_usdc_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct PauseStream<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"stream", vault.key().as_ref(), recipient.key().as_ref()],
        bump = stream.bump,
        has_one = vault,
        has_one = recipient
    )]
    pub stream: Account<'info, Stream>,
    #[account(
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority
    )]
    pub vault: Account<'info, Vault>,
    pub recipient: SystemAccount<'info>,
}

#[derive(Accounts)]
pub struct CancelStream<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        close = authority,
        seeds = [b"stream", vault.key().as_ref(), recipient.key().as_ref()],
        bump = stream.bump,
        has_one = vault,
        has_one = recipient
    )]
    pub stream: Account<'info, Stream>,
    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority
    )]
    pub vault: Account<'info, Vault>,
    pub recipient: SystemAccount<'info>,
}

/// Create a one-time payout.  The `pending_payout` account is always
/// initialised so the PDA is occupied (preventing replay), regardless of
/// whether the transfer happens immediately or is deferred.
#[derive(Accounts)]
#[instruction(payout_id: u64)]
pub struct CreateOneTimePayout<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: recipient receives the tokens; validated via recipient_usdc_token_account
    pub recipient: AccountInfo<'info>,
    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        has_one = authority,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init,
        payer = authority,
        space = PendingPayout::SPACE,
        seeds = [b"pending-payout", vault.key().as_ref(), &payout_id.to_le_bytes()],
        bump
    )]
    pub pending_payout: Account<'info, PendingPayout>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = recipient_usdc_token_account.owner == recipient.key() @ FlowdrenError::InvalidTokenOwner,
        constraint = recipient_usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint
    )]
    pub recipient_usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Register an approver's signature on a pending payout.
#[derive(Accounts)]
pub struct ApprovePayout<'info> {
    #[account(mut)]
    pub approver: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"pending-payout", vault.key().as_ref(), &pending_payout.payout_id.to_le_bytes()],
        bump = pending_payout.bump,
        has_one = vault,
        has_one = recipient
    )]
    pub pending_payout: Account<'info, PendingPayout>,
    /// CHECK: identity check enforced by pending_payout.has_one = recipient
    pub recipient: AccountInfo<'info>,
    #[account(
        mut,
        constraint = recipient_usdc_token_account.owner == recipient.key() @ FlowdrenError::InvalidTokenOwner,
        constraint = recipient_usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint
    )]
    pub recipient_usdc_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

/// Permissionless execution once approvals threshold has been reached.
#[derive(Accounts)]
pub struct ExecutePendingPayout<'info> {
    /// Any payer can trigger execution.
    #[account(mut)]
    pub executor: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump,
        has_one = usdc_token_account
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"pending-payout", vault.key().as_ref(), &pending_payout.payout_id.to_le_bytes()],
        bump = pending_payout.bump,
        has_one = vault,
        has_one = recipient
    )]
    pub pending_payout: Account<'info, PendingPayout>,
    /// CHECK: identity check enforced by pending_payout.has_one = recipient
    pub recipient: AccountInfo<'info>,
    #[account(
        mut,
        constraint = recipient_usdc_token_account.owner == recipient.key() @ FlowdrenError::InvalidTokenOwner,
        constraint = recipient_usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint
    )]
    pub recipient_usdc_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"vault-usdc", vault.key().as_ref()],
        bump = vault.token_account_bump,
        constraint = usdc_token_account.mint == vault.usdc_mint @ FlowdrenError::InvalidMint,
        constraint = usdc_token_account.owner == vault.key() @ FlowdrenError::InvalidTokenOwner
    )]
    pub usdc_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

// ─── Account data structures ──────────────────────────────────────────────────

#[account]
pub struct Vault {
    pub authority: Pubkey,           // 32
    pub usdc_mint: Pubkey,           // 32
    pub usdc_token_account: Pubkey,  // 32
    pub total_deposited: u64,        // 8
    pub total_allocated: u64,        // 8
    pub total_withdrawn: u64,        // 8
    pub bump: u8,                    // 1
    pub token_account_bump: u8,      // 1
    // Approver / threshold extension
    pub approvers: [Pubkey; MAX_APPROVERS], // 5 * 32 = 160
    pub approver_count: u8,          // 1
    pub approval_threshold: u8,      // 1
    pub payout_threshold: u64,       // 8
}

impl Vault {
    // discriminator(8) + authority(32) + usdc_mint(32) + usdc_token_account(32)
    // + total_deposited(8) + total_allocated(8) + total_withdrawn(8)
    // + bump(1) + token_account_bump(1)
    // + approvers(5*32=160) + approver_count(1) + approval_threshold(1) + payout_threshold(8)
    // = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 160 + 1 + 1 + 8 = 300
    pub const SPACE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 160 + 1 + 1 + 8;
}

#[account]
pub struct Stream {
    pub vault: Pubkey,               // 32
    pub recipient: Pubkey,           // 32
    pub rate_per_second: u64,        // 8
    pub start_timestamp: i64,        // 8
    pub end_timestamp: Option<i64>,  // 9
    pub total_withdrawn: u64,        // 8
    pub paused: bool,                // 1
    pub bump: u8,                    // 1
}

impl Stream {
    pub const SPACE: usize = 8 + 32 + 32 + 8 + 8 + 9 + 8 + 1 + 1;
}

/// A pending one-time payout waiting for approvals before funds move.
#[account]
pub struct PendingPayout {
    pub vault: Pubkey,        // 32 – parent vault
    pub recipient: Pubkey,    // 32 – who receives the funds
    pub amount: u64,          // 8  – token amount (USDC lamports)
    pub payout_id: u64,       // 8  – caller-assigned id (used in PDA seed)
    pub execute_after: i64,   // 8  – earliest execution timestamp (0 = any time)
    pub approved_mask: u8,    // 1  – bitmask of approver indices that have approved
    pub executed: bool,       // 1  – true once funds have been transferred
    pub bump: u8,             // 1  – PDA bump
}

impl PendingPayout {
    // discriminator(8) + vault(32) + recipient(32) + amount(8) + payout_id(8)
    // + execute_after(8) + approved_mask(1) + executed(1) + bump(1)
    pub const SPACE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 1;
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[error_code]
pub enum FlowdrenError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Token account owner is invalid")]
    InvalidTokenOwner,
    #[msg("Token mint is invalid")]
    InvalidMint,
    #[msg("Invalid timestamp")]
    InvalidTimestamp,
    #[msg("Insufficient balance")]
    InsufficientBalance,
    #[msg("Stream is paused")]
    StreamPaused,
    #[msg("Stream has not started")]
    StreamNotStarted,
    #[msg("Nothing to withdraw")]
    NothingToWithdraw,
    // Payout / approver errors
    #[msg("This payout has already been executed")]
    PayoutAlreadyExecuted,
    #[msg("Insufficient approvals to execute payout")]
    InsufficientApprovals,
    #[msg("Signer is not a registered approver for this vault")]
    NotAnApprover,
    #[msg("Approver has already approved this payout")]
    AlreadyApproved,
    #[msg("Payout execute_after time has not been reached")]
    PayoutNotReady,
    #[msg("Vault already has the maximum number of approvers")]
    MaxApproversReached,
    #[msg("Approval threshold cannot exceed the number of approvers")]
    ThresholdTooHigh,
    #[msg("Pubkey is already registered as an approver")]
    AlreadyAnApprover,
}
