use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("Fg6PaFpoGXkYsidMpWxTWqgph1DfhhbkTJSa9kM2sXpj");

#[program]
pub mod flowdren {
    use super::*;

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
            // For streams without end timestamp, allocate all available balance
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
            // For streams without end timestamp, refund everything not yet withdrawn
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
}

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

#[account]
pub struct Vault {
    pub authority: Pubkey,
    pub usdc_mint: Pubkey,
    pub usdc_token_account: Pubkey,
    pub total_deposited: u64,
    pub total_allocated: u64,
    pub total_withdrawn: u64,
    pub bump: u8,
    pub token_account_bump: u8,
}

impl Vault {
    pub const SPACE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1;
}

#[account]
pub struct Stream {
    pub vault: Pubkey,
    pub recipient: Pubkey,
    pub rate_per_second: u64,
    pub start_timestamp: i64,
    pub end_timestamp: Option<i64>,
    pub total_withdrawn: u64,
    pub paused: bool,
    pub bump: u8,
}

impl Stream {
    pub const SPACE: usize = 8 + 32 + 32 + 8 + 8 + 9 + 8 + 1 + 1;
}

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
}

