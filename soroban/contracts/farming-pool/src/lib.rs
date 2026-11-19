#![no_std]

#[cfg(test)]
mod mock_reentrant_token;
mod types;

use soroban_sdk::{contract, contractimpl, token, Address, Env};
pub use types::{PoolError, PoolInfo};
use types::{
    BankedCreditTotals, BoostConfig, CreditRateEvent, CreditRateHistoryPage,
    DataKey, Position, UserStake,
};

// Persistent-storage TTL constants (~30 days / ~60 days at 5 s/ledger).
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;
const USER_TTL_THRESHOLD: u32 = 518_400;
const USER_TTL_EXTEND_TO: u32 = 1_036_800;

const MAX_GLOBAL_MULTIPLIER: u32 = 1_000;
const MAX_CREDIT_RATE: i128 = 100_000_000;
const MAX_STAKE_AMOUNT: i128 = 10i128.pow(18);
const MAX_LOCK_PERIOD: u32 = 63_072_000;
const SCHEMA_VERSION: u32 = 1;

// ── Storage helpers ──────────────────────────────────────────────────────────

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn bump_user(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, USER_TTL_THRESHOLD, USER_TTL_EXTEND_TO);
}

fn require_initialized(env: &Env) -> Result<(), PoolError> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(PoolError::NotInitialized);
    }
    Ok(())
}

fn require_staking_not_paused(env: &Env) -> Result<(), PoolError> {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::PausedStaking)
        .unwrap_or(false);
    if paused {
        return Err(PoolError::Paused);
    }
    let globally_paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    if globally_paused {
        return Err(PoolError::Paused);
    }
    Ok(())
}

fn require_withdrawals_not_paused(env: &Env) -> Result<(), PoolError> {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::PausedWithdrawals)
        .unwrap_or(false);
    if paused {
        return Err(PoolError::Paused);
    }
    let globally_paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    if globally_paused {
        return Err(PoolError::Paused);
    }
    Ok(())
}

fn get_admin(env: &Env) -> Result<Address, PoolError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(PoolError::NotInitialized)
}

fn get_stake_token(env: &Env) -> Result<Address, PoolError> {
    env.storage()
        .instance()
        .get(&DataKey::StakeToken)
        .ok_or(PoolError::NotInitialized)
}

fn read_credit_rate(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::CreditRate)
        .unwrap_or(1)
}

fn read_global_multiplier(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::GlobalMultiplier)
        .unwrap_or(1)
}

fn read_min_lock_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::MinLockPeriod)
        .unwrap_or(0)
}

fn get_user_stake(env: &Env, user: &Address) -> Option<UserStake> {
    env.storage()
        .persistent()
        .get(&DataKey::UserStake(user.clone()))
}

fn set_user_stake(env: &Env, user: &Address, stake: &UserStake) {
    let key = DataKey::UserStake(user.clone());
    env.storage().persistent().set(&key, stake);
    bump_user(env, &key);
}

fn remove_user_stake(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::UserStake(user.clone()));
}

fn get_position(env: &Env, user: &Address) -> Option<Position> {
    env.storage()
        .persistent()
        .get(&DataKey::UserPosition(user.clone()))
}

fn set_position(env: &Env, user: &Address, position: &Position) {
    let key = DataKey::UserPosition(user.clone());
    env.storage().persistent().set(&key, position);
    bump_user(env, &key);
}

fn remove_position(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::UserPosition(user.clone()));
}

fn read_total_staked(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalStaked)
        .unwrap_or(0)
}

fn add_total_staked(env: &Env, amount: i128) {
    let total = read_total_staked(env);
    env.storage()
        .instance()
        .set(&DataKey::TotalStaked, &total.checked_add(amount).expect("total stake overflow"));
}

fn subtract_total_staked(env: &Env, amount: i128) {
    let total = read_total_staked(env);
    env.storage()
        .instance()
        .set(&DataKey::TotalStaked, &total.checked_sub(amount).expect("total stake underflow"));
}

fn add_total_locked(env: &Env, amount: i128) {
    let total: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalLocked)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::TotalLocked, &total.checked_add(amount).expect("total locked overflow"));
}

fn subtract_total_locked(env: &Env, amount: i128) {
    let total: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalLocked)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::TotalLocked, &total.checked_sub(amount).expect("total locked underflow"));
}

fn read_total_deposits(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalDeposits)
        .unwrap_or(0)
}

fn add_total_deposits(env: &Env, amount: i128) {
    let total = read_total_deposits(env);
    env.storage()
        .instance()
        .set(&DataKey::TotalDeposits, &total.checked_add(amount).expect("total deposits overflow"));
}

fn add_total_withdrawals(env: &Env, amount: i128) {
    let total: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalWithdrawals)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::TotalWithdrawals, &total.checked_add(amount).expect("total withdrawals overflow"));
}

fn is_user_staked(env: &Env, user: &Address) -> bool {
    get_position(env, user).is_some() || get_user_stake(env, user).is_some()
}

fn increment_active_stake_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::ActiveStakeCount)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::ActiveStakeCount, &(count + 1));
}

fn decrement_active_stake_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::ActiveStakeCount)
        .unwrap_or(0);
    if count > 0 {
        env.storage()
            .instance()
            .set(&DataKey::ActiveStakeCount, &(count - 1));
    }
}

fn increment_staked_user_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::StakedUserCount)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::StakedUserCount, &(count + 1));
}

fn decrement_staked_user_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::StakedUserCount)
        .unwrap_or(0);
    if count > 0 {
        env.storage()
            .instance()
            .set(&DataKey::StakedUserCount, &(count - 1));
    }
}

fn increment_lock_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::LockCount)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::LockCount, &(count + 1));
}

fn increment_unstake_count(env: &Env) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::UnstakeCount)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&DataKey::UnstakeCount, &(count + 1));
}

fn get_min_stake_amount_raw(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::MinStakeAmount)
        .unwrap_or(0)
}

fn whitelist_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistEnabled)
        .unwrap_or(false)
}

fn is_user_whitelisted(env: &Env, user: &Address) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Whitelisted(user.clone()))
        .unwrap_or(false)
}

// ── Credit rate history (#604) ───────────────────────────────────────────────
//
// Every `set_credit_rate` call appends a `CreditRateEvent` to persistent
// storage, keyed by a monotonically incrementing `CreditRateEventCount`.
// `get_credit_rate_history` pages over these records so off-chain tools can
// verify any historical accrual rate. This provides the audit trail and
// transparency described in #604.

fn record_credit_rate_change(env: &Env, old_rate: i128, new_rate: i128) {
    let count: u32 = env
        .storage()
        .instance()
        .get(&DataKey::CreditRateEventCount)
        .unwrap_or(0);
    let event = CreditRateEvent {
        old_rate,
        new_rate,
        ledger: env.ledger().sequence(),
    };
    env.storage()
        .persistent()
        .set(&DataKey::CreditRateHistory(count), &event);
    bump_user(env, &DataKey::CreditRateHistory(count));
    env.storage()
        .instance()
        .set(&DataKey::CreditRateEventCount, &(count + 1));
}

fn increment_credit_rate_change_count(env: &Env) {
    // Count is managed inside record_credit_rate_change; this is kept for
    // compatibility with callers that increment separately.
}

// ── Checkpoint helper ────────────────────────────────────────────────────────

fn compute_credits(amount: i128, credit_rate: i128, elapsed: u32, multiplier: u32) -> i128 {
    let elapsed_i = elapsed as i128;
    let mult_i = multiplier as i128;
    amount
        .saturating_mul(credit_rate)
        .saturating_mul(elapsed_i)
        .saturating_mul(mult_i)
}

fn checkpoint(env: &Env, user: &Address, stake: &mut UserStake) {
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(stake.start_ledger);
    if elapsed > 0 {
        let accrued = compute_credits(stake.amount, stake.credit_rate, elapsed, stake.multiplier);
        stake.credits_banked = stake.credits_banked.saturating_add(accrued);
    }
    stake.start_ledger = current;
    stake.credit_rate = read_credit_rate(env);
    stake.multiplier = read_global_multiplier(env);
}

fn checkpoint_position(env: &Env, _user: &Address, position: &mut Position) {
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(position.checkpoint_ledger);
    if elapsed > 0 {
        let accrued = compute_credits(position.amount, position.credit_rate, elapsed, 1);
        position.total_credits = position.total_credits.saturating_add(accrued);
    }
    position.checkpoint_ledger = current;
    position.credit_rate = read_credit_rate(env);
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct FarmingPool;

#[contractimpl]
impl FarmingPool {
    /// Initialize the pool with its core parameters.
    pub fn initialize(
        env: Env,
        admin: Address,
        stake_token: Address,
        global_multiplier: u32,
        credit_rate: i128,
        min_lock_period: u32,
        min_stake_amount: i128,
    ) -> Result<(), PoolError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(PoolError::AlreadyInitialized);
        }
        if !(1..=MAX_GLOBAL_MULTIPLIER).contains(&global_multiplier) {
            return Err(PoolError::InvalidGlobalMultiplier);
        }
        if credit_rate <= 0 || credit_rate > MAX_CREDIT_RATE {
            return Err(PoolError::InvalidCreditRate);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::StakeToken, &stake_token);
        env.storage().instance().set(&DataKey::GlobalMultiplier, &global_multiplier);
        env.storage().instance().set(&DataKey::CreditRate, &credit_rate);
        env.storage().instance().set(&DataKey::MinLockPeriod, &min_lock_period);
        env.storage().instance().set(&DataKey::MinStakeAmount, &min_stake_amount);
        env.storage().instance().set(&DataKey::TotalStaked, &0i128);
        env.storage().instance().set(&DataKey::TotalLocked, &0i128);
        env.storage().instance().set(&DataKey::TotalDeposits, &0i128);
        env.storage().instance().set(&DataKey::TotalWithdrawals, &0i128);
        env.storage().instance().set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        bump_instance(&env);
        Ok(())
    }

    /// Return the pool admin address.
    pub fn admin(env: Env) -> Result<Address, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        get_admin(&env)
    }

    /// Return the per-ledger credit accrual rate.
    pub fn credit_rate(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_credit_rate(&env))
    }

    /// Return the global reward multiplier.
    pub fn global_multiplier(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_global_multiplier(&env))
    }

    /// Return the minimum lock period in ledgers.
    pub fn min_lock_period(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_min_lock_period(&env))
    }

    /// Return the minimum stake amount.
    pub fn get_min_stake_amount(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_min_stake_amount_raw(&env))
    }

    /// Return whether the pool is currently paused.
    pub fn is_paused(env: Env) -> Result<bool, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false))
    }

    /// Return the total staked across all positions.
    pub fn total_staked(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_staked(&env))
    }

    /// Return cumulative deposits.
    pub fn total_deposits(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_deposits(&env))
    }

    /// Return cumulative withdrawals.
    pub fn total_withdrawals(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalWithdrawals)
            .unwrap_or(0))
    }

    /// Return the total locked in time-locked positions.
    pub fn total_locked(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0))
    }

    /// Return the user's current flexible stake, or None.
    pub fn get_stake(env: Env, user: Address) -> Option<UserStake> {
        get_user_stake(&env, &user)
    }

    /// Return the user's current locked position, or None.
    pub fn get_user_position(env: Env, user: Address) -> Option<Position> {
        get_position(&env, &user)
    }

    /// Stake tokens into the flexible staking pool.
    pub fn stake(env: Env, from: Address, amount: i128) -> Result<(), PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_staking_not_paused(&env)?;

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        if amount > MAX_STAKE_AMOUNT {
            return Err(PoolError::ExceedsMaxStake);
        }
        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &from) {
            return Err(PoolError::NotWhitelisted);
        }

        let existing_amount = get_user_stake(&env, &from).map_or(0i128, |s| s.amount);
        let total_amount = existing_amount
            .checked_add(amount)
            .ok_or(PoolError::ExceedsMaxStake)?;
        let min_stake = get_min_stake_amount_raw(&env);
        if total_amount < min_stake {
            return Err(PoolError::BelowMinimumStake);
        }
        if total_amount > MAX_STAKE_AMOUNT {
            return Err(PoolError::ExceedsMaxStake);
        }

        let stake_token = get_stake_token(&env)?;

        // Validate balance before touching state.
        let caller_balance = token::TokenClient::new(&env, &stake_token).balance(&from);
        if caller_balance < amount {
            return Err(PoolError::InsufficientBalance);
        }

        // Reject before any write if the pool-wide deposit total would overflow. See #452.
        read_total_deposits(&env)
            .checked_add(amount)
            .ok_or(PoolError::ExceedsMaxStake)?;

        // Reject before any write if the pool-wide total staked would overflow. See #460, #461.
        read_total_staked(&env)
            .checked_add(amount)
            .ok_or(PoolError::ExceedsMaxStake)?;

        let mut existing_stake = env
            .storage()
            .persistent()
            .get::<DataKey, UserStake>(&DataKey::UserStake(from.clone()));

        bump_instance(&env);

        let is_first_stake = existing_stake.is_none();
        let was_staked = is_user_staked(&env, &from);
        let current = env.ledger().sequence();

        let mut new_stake = if let Some(mut existing) = existing_stake.take() {
            bump_user(&env, &DataKey::UserStake(from.clone()));
            checkpoint(&env, &from, &mut existing);
            existing.amount = total_amount;
            existing
        } else {
            UserStake {
                amount,
                start_ledger: current,
                credits_banked: 0,
                credit_rate: read_credit_rate(&env),
                multiplier: read_global_multiplier(&env),
            }
        };
        new_stake.credit_rate = read_credit_rate(&env);

        // Checks-effects-interactions: write state before external transfer. See #69.
        set_user_stake(&env, &from, &new_stake);
        if is_first_stake {
            increment_active_stake_count(&env);
        }
        if !was_staked && is_user_staked(&env, &from) {
            increment_staked_user_count(&env);
        }
        add_total_staked(&env, amount);
        add_total_deposits(&env, amount);

        token::TokenClient::new(&env, &stake_token).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );

        Ok(())
    }

    /// Withdraw flexible stake.
    pub fn unstake(env: Env, from: Address, amount: i128) -> Result<i128, PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_withdrawals_not_paused(&env)?;
        bump_instance(&env);

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }

        let was_staked = is_user_staked(&env, &from);
        let mut stake = get_user_stake(&env, &from).ok_or(PoolError::NoActiveStake)?;
        if amount > stake.amount {
            return Err(PoolError::InsufficientBalance);
        }

        checkpoint(&env, &from, &mut stake);
        let total_credits = stake.credits_banked;
        stake.amount -= amount;

        if stake.amount == 0 {
            remove_user_stake(&env, &from);
            decrement_active_stake_count(&env);
            if was_staked && !is_user_staked(&env, &from) {
                decrement_staked_user_count(&env);
            }
        } else {
            set_user_stake(&env, &from, &stake);
        }

        increment_unstake_count(&env);
        subtract_total_staked(&env, amount);
        add_total_withdrawals(&env, amount);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &from,
            &amount,
        );

        Ok(total_credits)
    }

    /// Lock assets for the minimum lock period.
    pub fn lock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_staking_not_paused(&env)?;

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        if amount > MAX_STAKE_AMOUNT {
            return Err(PoolError::ExceedsMaxStake);
        }
        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &user) {
            return Err(PoolError::NotWhitelisted);
        }

        let existing_amount = get_position(&env, &user).map_or(0i128, |p| p.amount);
        let total_amount = existing_amount + amount;
        let min_stake = get_min_stake_amount_raw(&env);
        if total_amount < min_stake {
            return Err(PoolError::BelowMinimumStake);
        }
        if total_amount > MAX_STAKE_AMOUNT {
            return Err(PoolError::ExceedsMaxStake);
        }

        bump_instance(&env);
        let was_staked = is_user_staked(&env, &user);
        let current = env.ledger().sequence();

        let mut position = if let Some(mut existing) = get_position(&env, &user) {
            checkpoint_position(&env, &user, &mut existing);
            existing.amount += amount;
            existing.lock_ledger = current;
            let fresh_unlock = current.saturating_add(read_min_lock_period(&env));
            existing.unlock_ledger = existing.unlock_ledger.max(fresh_unlock);
            existing
        } else {
            Position {
                amount,
                lock_ledger: current,
                unlock_ledger: current.saturating_add(read_min_lock_period(&env)),
                checkpoint_ledger: current,
                total_credits: 0,
                credit_rate: read_credit_rate(&env),
            }
        };
        position.credit_rate = read_credit_rate(&env);

        set_position(&env, &user, &position);
        if !was_staked && is_user_staked(&env, &user) {
            increment_staked_user_count(&env);
        }
        increment_lock_count(&env);
        add_total_staked(&env, amount);
        add_total_locked(&env, amount);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        Ok(())
    }

    /// Unlock assets after the lock period has elapsed.
    pub fn unlock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_withdrawals_not_paused(&env)?;

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }

        bump_instance(&env);
        let was_staked = is_user_staked(&env, &user);
        let mut position = get_position(&env, &user).ok_or(PoolError::NoActivePosition)?;
        if amount > position.amount {
            return Err(PoolError::InsufficientBalance);
        }

        let current = env.ledger().sequence();
        if current < position.unlock_ledger {
            return Err(PoolError::LockPeriodNotElapsed);
        }

        checkpoint_position(&env, &user, &mut position);
        position.amount -= amount;

        if position.amount == 0 {
            remove_position(&env, &user);
        } else {
            set_position(&env, &user, &position);
        }

        if was_staked && !is_user_staked(&env, &user) {
            decrement_staked_user_count(&env);
        }
        subtract_total_staked(&env, amount);
        subtract_total_locked(&env, amount);
        add_total_withdrawals(&env, amount);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &user,
            &amount,
        );

        Ok(())
    }

    /// Update the credit rate. Admin-only.
    ///
    /// Every rate change is appended to the on-chain `CreditRateHistory` log
    /// (keyed by `DataKey::CreditRateHistory(index)`) and queryable via
    /// `get_credit_rate_history`. This provides a full audit trail of past
    /// rates and enables off-chain verification of historical accrual. See #604.
    pub fn set_credit_rate(env: Env, new_rate: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        if new_rate <= 0 || new_rate > MAX_CREDIT_RATE {
            return Err(PoolError::InvalidCreditRate);
        }
        bump_instance(&env);
        let old_rate = read_credit_rate(&env);
        env.storage().instance().set(&DataKey::CreditRate, &new_rate);
        record_credit_rate_change(&env, old_rate, new_rate);
        Ok(())
    }

    /// Update the global multiplier. Admin-only.
    pub fn set_global_multiplier(env: Env, new_multiplier: u32) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        if !(1..=MAX_GLOBAL_MULTIPLIER).contains(&new_multiplier) {
            return Err(PoolError::InvalidGlobalMultiplier);
        }
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::GlobalMultiplier, &new_multiplier);
        Ok(())
    }

    /// Update the minimum stake amount. Admin-only.
    pub fn set_min_stake_amount(env: Env, amount: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        if amount <= 0 {
            return Err(PoolError::InvalidMinStakeAmount);
        }
        bump_instance(&env);
        env.storage().instance().set(&DataKey::MinStakeAmount, &amount);
        Ok(())
    }

    /// Pause the pool.
    pub fn pause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    /// Unpause the pool.
    pub fn unpause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    /// Return paginated credit rate change history. See #604.
    ///
    /// The full history is stored on-chain as `DataKey::CreditRateHistory(n)`
    /// entries, appended by every `set_credit_rate` call. This provides the
    /// audit trail and transparency described in #604.
    pub fn get_credit_rate_history(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<CreditRateHistoryPage, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let total: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CreditRateEventCount)
            .unwrap_or(0);
        let mut events = soroban_sdk::Vec::new(&env);
        let mut i = offset;
        let mut count = 0u32;
        while i < total && count < limit {
            if let Some(event) = env
                .storage()
                .persistent()
                .get::<_, CreditRateEvent>(&DataKey::CreditRateHistory(i))
            {
                events.push_back(event);
                count += 1;
            }
            i += 1;
        }
        Ok(CreditRateHistoryPage { events, total })
    }

    /// Return the user's accrued credits.
    pub fn get_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let mut total: i128 = 0;
        if let Some(mut stake) = get_user_stake(&env, &user) {
            checkpoint(&env, &user, &mut stake);
            total = total.saturating_add(stake.credits_banked);
        }
        if let Some(mut position) = get_position(&env, &user) {
            checkpoint_position(&env, &user, &mut position);
            total = total.saturating_add(position.total_credits);
        }
        Ok(total)
    }

    /// Return aggregate pool parameters in a single call.
    pub fn get_pool_info(env: Env) -> Result<PoolInfo, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(PoolInfo {
            total_staked: read_total_staked(&env),
            total_banked_credits: 0,
            total_distributed_credits: 0,
            credit_rate: read_credit_rate(&env),
            global_multiplier: read_global_multiplier(&env),
            min_lock_period: read_min_lock_period(&env),
            min_stake_amount: get_min_stake_amount_raw(&env),
            is_paused: env
                .storage()
                .instance()
                .get(&DataKey::Paused)
                .unwrap_or(false),
        })
    }
}

#[cfg(any(test, feature = "testutils"))]
mod wasm_import {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32v1-none/release/farming_pool.wasm"
    );
}
#[cfg(any(test, feature = "testutils"))]
pub use wasm_import::WASM;
