use anchor_lang::prelude::*;
use anchor_lang::prelude::{Account, Clock};
use anchor_lang::solana_program::{program_option::COption};
use anchor_spl::token::{self, Mint, TokenAccount, Token};
use std::convert::Into;
use std::convert::TryInto;
use spl_math::uint::U192;
declare_id!("8SbebuABofE1WbiRU1cy3h4H26ji9r7y8Nta7akgkct3");


pub const YEAR_IN_SECONDS: u64 = 31536000;
pub const HERSH_TOKEN_KEY: &str = "2Dnj6Txs4p8E3J5wgA28vmzJMcaPhBs3R4vgwUU6yvLa";
pub const DEPOSIT_REQUIREMENT: u64 = 10_000_000_000_000;
pub const MIN_DURATION: u64 = 1;

const PRECISION: u128 = u64::MAX as u128;
pub(crate) fn update_yield(
    pool: &mut Account<StakePool>,
    staker: Option<&mut Box<Account<Staker>>>,
    staked_amount: u64,
) -> Result<()> {
    let last_time_yield_applicable = last_time_yield_applicable(pool.staking_period_end_time);

    let _yield = yield_per_stake(pool, staked_amount, last_time_yield_applicable);
    pool.unclaimed_yield = _yield.into();

    pool.last_update_time = last_time_yield_applicable;

    if let Some(s) = staker {
        let a = user_earned_amount(pool, s);

        s.unclaimed_yield = a;
        s.claimed_yield = pool.last_calculated_yield;
    }

    Ok(())
}
// compares the current time with the staking period end time and returns the minimum of them.
pub fn last_time_yield_applicable(staking_period_end_time: u64) -> u64 {
    let now = Clock::get().unwrap();
    std::cmp::min(now.unix_timestamp.try_into().unwrap(), staking_period_end_time )
}

pub(crate) fn yield_per_stake(
    pool: &Account<StakePool>,
    amount_staked: u64,
    last_time_yield_applicable: u64,
) -> u128 {
    if amount_staked == 0 {
        return pool.unclaimed_yield.into();
    }
    let time_period = U192::from(last_time_yield_applicable)
        .checked_sub(pool.last_update_time.into())
        .unwrap();
    let earned_yield = pool.unclaimed_yield.checked_add(time_period
                .checked_mul(pool.yield_rate.into())
                .unwrap()
                .checked_mul(PRECISION.into())
                .unwrap()
                .checked_div(YEAR_IN_SECONDS.into())
                .unwrap()
                .checked_div(amount_staked.into())
                .unwrap()
                .try_into()
                .unwrap())
            .unwrap();

    earned_yield.into()
    
}
pub(crate) fn user_earned_amount(
    pool: &Account<StakePool>,
    user: &Account<Staker>,
) -> u64 {
    let earned_amount: u64 = (user.staked_amount as u128)
        .checked_mul(
            (pool.unclaimed_yield as u128)
                .checked_sub(user.claimed_yield as u128)
                .unwrap(),
        )
        .unwrap()
        .checked_div(PRECISION)
        .unwrap()
        .checked_add(user.unclaimed_yield as u128)
        .unwrap()
        .try_into()
        .unwrap(); 
    earned_amount
}
#[program]
pub mod stake_hersh {
    use super::*;
    pub fn init_stake_pool(ctx: Context<InitPool>, pool_bump: u8, reward_duration: u64) -> Result<()>{
        if reward_duration < MIN_DURATION {
            return Err(error!(StakeErr::DurationTooShort));
        }
        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            token::Transfer{
                from: ctx.accounts.hersh_depositor.to_account_info(),
                to: ctx.accounts.hersh_vault.to_account_info(),
                authority: ctx.accounts.hersh_deposit_authority.to_account_info(),
            },
        );
        token::transfer(cpi_ctx, DEPOSIT_REQUIREMENT)?;
        let pool = &mut ctx.accounts.stake_pool;
        pool.user_amount = 0;
        pool.staking_mint = ctx.accounts.staking_mint.key();
        pool.staking_vault = ctx.accounts.staking_vault.key();
        pool.yield_token_vault = ctx.accounts.yield_token_vault.key();
        pool.yield_mint = ctx.accounts.yield_mint.key();
        pool.staking_period_end_time = 0;
        pool.yield_rate = 0;
        pool.last_calculated_yield = 0;
        pool.last_update_time = 0;
        pool.pool_bump = pool_bump;
        pool.is_initialized = true;
        pool.unclaimed_yield = 0;

        Ok(())
    }

    pub fn new_staker(ctx:Context<NewStaker>, nonce: u8) -> Result<()> {
        let new_staker = &mut ctx.accounts.staker;
        new_staker.staked_amount = 0;
        new_staker.owner = *ctx.accounts.owner.key;
        new_staker.stake_pool = *ctx.accounts.stake_pool.to_account_info().key;
        new_staker.claimed_yield = 0;
        new_staker.unclaimed_yield = 0;
        new_staker.nonce = nonce;
        Ok(())
    }


    pub fn stake(ctx: Context<Stake>, amount: u64) -> Result<()> {
        if amount == 0{
           return Err(error!(StakeErr::AmountZero));
        }
        let total_staked = ctx.accounts.staking_vault.amount;

        let stake_pool = &mut ctx.accounts.stake_pool;
        let staker_opt = Some(&mut ctx.accounts.staker);
        update_yield(stake_pool, staker_opt, total_staked);

        ctx.accounts.staker.staked_amount = ctx.accounts.staker.staked_amount.checked_add(amount).unwrap();

        {
            let cpi_ctx = CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::Transfer {
                    from: ctx.accounts.stake_from_account.to_account_info(),
                    to: ctx.accounts.staking_vault.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(), //todo use user account as signer
                },
            );
            token::transfer(cpi_ctx, amount)?;
        }

        msg!("Pool: {:?}", **ctx.accounts.stake_pool);
        msg!("User: {:?}", **ctx.accounts.staker);
        Ok(())
    }

    
    pub fn claim_yield(ctx: Context<ClaimYield>) -> Result<()> {
    
        let total_staked = ctx.accounts.staking_vault.amount;
        let user_opt = Some(&mut ctx.accounts.staker);

        let stake_pool = &mut ctx.accounts.stake_pool;
        update_yield(stake_pool, user_opt, total_staked).unwrap();

        let seeds = &[
            ctx.accounts.stake_pool.to_account_info().key.as_ref(),
            &[ctx.accounts.stake_pool.pool_bump],
        ];
        let pool_signer =&[&seeds[..]];
        if ctx.accounts.staker.unclaimed_yield > 0 {
            let mut reward_amount = ctx.accounts.staker.unclaimed_yield;
            let vault_balance = ctx.accounts.staker_yield_account.amount;
            ctx.accounts.staker.unclaimed_yield = 0;
            if reward_amount > vault_balance.into() {
                reward_amount = vault_balance;
            }
            if reward_amount > 0 {
                let cpi_ctx = CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    token::Transfer {
                        from: ctx.accounts.yield_token_vault.to_account_info(),
                        to: ctx.accounts.staker_yield_account.to_account_info(),
                        authority: ctx.accounts.pool_signer.to_account_info(), 
                    },
                    pool_signer,
                );
                token::transfer(cpi_ctx, reward_amount)?;
            }

        }
        Ok(())
    }

    pub fn withdraw(ctx: Context<Stake>, spt_amount: u64) -> Result<()> {
        if spt_amount == 0 {
            return err!(StakeErr::AmountZero);
        }

        let stake_pool = &mut ctx.accounts.stake_pool;
        let staked_amount = ctx.accounts.staking_vault.amount;

        if ctx.accounts.staker.staked_amount < spt_amount {
            return Err(error!(StakeErr::InsufficientFundUnstake));
        }

        let staker_opt = Some(&mut ctx.accounts.staker);
        update_yield(stake_pool, staker_opt, staked_amount).unwrap();
        ctx.accounts.staker.staked_amount = ctx
            .accounts
            .staker
            .staked_amount
            .checked_sub(spt_amount)
            .unwrap();

        // Transfer tokens from the pool vault to user vault.
        {
            let seeds = &[stake_pool.to_account_info().key.as_ref(), &[stake_pool.pool_bump]];
            let pool_signer = &[&seeds[..]];

            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::Transfer {
                    from: ctx.accounts.staking_vault.to_account_info(),
                    to: ctx.accounts.stake_from_account.to_account_info(),
                    authority: ctx.accounts.pool_signer.to_account_info(),
                },
                pool_signer,
            );
            token::transfer(cpi_ctx, spt_amount)?;
        }
        msg!("Pool: {:?}", **ctx.accounts.stake_pool);
        msg!("User: {:?}", **ctx.accounts.staker);
        Ok(())
    }

}




#[derive(Accounts)]
pub struct InitPool<'info>{
    pool_authority: UncheckedAccount<'info>,

    #[account(mut, 
    constraint = hersh_vault.mint == HERSH_TOKEN_KEY.parse::<Pubkey>().unwrap(),
    constraint = hersh_vault.owner == pool_signer.key(),
    )]
    hersh_vault: Box<Account<'info,TokenAccount>>,
    hersh_depositor: Box<Account<'info, TokenAccount>>,
    hersh_deposit_authority: Signer<'info>,

    #[account(mut, 
        constraint = staking_vault.mint == HERSH_TOKEN_KEY.parse::<Pubkey>().unwrap(),
        constraint = staking_vault.owner == pool_signer.key())]
    staking_vault: Box<Account<'info, TokenAccount>>,
    staking_mint: Box<Account<'info, Mint>>,

    #[account(seeds = [stake_pool.to_account_info().key.as_ref()], bump )]
    pool_signer: Signer<'info>,
    
    yield_mint: Box<Account<'info, Mint>>,
    #[account(
        constraint = yield_token_vault.mint == yield_mint.key(),
        constraint = yield_token_vault.owner == pool_signer.key(),
        constraint = yield_token_vault.close_authority == COption::None,
    )]
    yield_token_vault: Box<Account<'info, TokenAccount>>,
    #[account(zero)]
    stake_pool: Box<Account<'info, StakePool>>,
    token_program: Program<'info, Token>,

}
#[derive(Accounts)]
pub struct NewStaker<'info>{
    #[account(init, payer = owner, seeds = [owner.key.as_ref(), stake_pool.to_account_info().key.as_ref()],bump)]
    staker: Box<Account<'info, Staker>>,

    #[account(mut, constraint = stake_pool.is_initialized == true)]
    stake_pool: Box<Account<'info, StakePool>>,
    #[account(mut)]
    owner: Signer<'info>,
    system_program: Program<'info, System>,

}
#[derive(Accounts)]
pub struct Stake<'info>{
    #[account(mut, has_one = staking_vault)]
    stake_pool: Box<Account<'info, StakePool>>,

    #[account(mut, constraint = staking_vault.owner == *pool_signer.key)]
    staking_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        has_one = owner,
        has_one = stake_pool,
        seeds = [
            owner.key.as_ref(),
            stake_pool.to_account_info().key.as_ref()
        ],
        bump = staker.nonce,
    )]
    staker: Box<Account<'info, Staker>>,
    owner: Signer<'info>,

    #[account(mut)]
    stake_from_account: Box<Account<'info, TokenAccount>>,

    #[account(
        seeds = [
            stake_pool.to_account_info().key.as_ref()
        ],
        bump = stake_pool.pool_bump,
    )]
    pool_signer: UncheckedAccount<'info>,

    token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClaimYield<'info>{
    #[account(mut, 
        has_one = staking_vault,
        has_one = yield_token_vault,)]
    stake_pool: Box<Account<'info,StakePool>>,

    #[account(mut)]
    staking_vault: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    yield_token_vault: Box<Account<'info, TokenAccount>>,
    
    #[account(mut)]
    staker_yield_account: Box<Account<'info,TokenAccount>>,
    #[account(mut)]
    staker: Box<Account<'info, Staker>>,
    #[account(
        seeds = [
            stake_pool.to_account_info().key.as_ref()
        ],
        bump = stake_pool.pool_bump,
    )]
    pool_signer: UncheckedAccount<'info>,
    token_program: Program<'info, Token>,

}

#[account]
#[derive(Default, Debug)]
pub struct StakePool{
    pub user_amount: u64,
    pub staking_mint: Pubkey,
    pub staking_vault: Pubkey,
    pub yield_token_vault: Pubkey,
    pub yield_mint: Pubkey,
    pub yield_rate: u64,
    pub staking_period: u64,
    pub staking_period_end_time: u64,
    pub last_calculated_yield: u64,
    pub unclaimed_yield: u128,
    pub pool_bump: u8,
    pub last_update_time: u64,
    pub is_initialized: bool,
}
#[account]
#[derive(Default, Debug)]
pub struct Staker{
    pub stake_pool: Pubkey,
    pub owner: Pubkey,
    pub claimed_yield: u64,
    pub unclaimed_yield: u64,
    pub staked_amount: u64,
    pub nonce: u8,
}
#[error_code]
pub enum StakeErr {
    #[msg("Duration too short")]
    DurationTooShort,
    #[msg("Amount should be greater than zero")]
    AmountZero,
    #[msg("Amount must be greater than 0")]
    AmountMustBeGreaterThanZero,
    #[msg("Insufficient funds to unstake")]
    InsufficientFundUnstake,
}
