#![no_std]

mod types;
#[cfg(test)]
mod mock_reentrant_token;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};
pub use types::PoolError;
use types::{BoostConfig, DataKey, Position, UserStake};

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5s/ledger).
const USER_TTL_THRESHOLD: u32 = 518_400;
const USER_TTL_EXTEND_TO: u32 = 1_036_800;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(USER_TTL_THRESHOLD, USER_TTL_EXTEND_TO);
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

fn require_not_paused(env: &Env) -> Result<(), PoolError> {
    if pool_is_paused(env) {
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

fn read_global_multiplier(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::GlobalMultiplier)
        .unwrap_or(1)
}

fn read_credit_rate(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::CreditRate)
        .unwrap_or(1)
}

fn get_stake_token(env: &Env) -> Result<Address, PoolError> {
    env.storage()
        .instance()
        .get(&DataKey::StakeToken)
        .ok_or(PoolError::NotInitialized)
}

fn read_min_lock_period(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::MinLockPeriod)
        .unwrap_or(0)
}

fn pool_is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

fn get_user_boost(env: &Env, user: &Address) -> Option<u32> {
    let key = DataKey::UserBoost(user.clone());
    let value: Option<u32> = env.storage().persistent().get(&key);
    if value.is_some() {
        bump_user(env, &key);
    }
    value
}

fn get_user_stake(env: &Env, user: &Address) -> Option<UserStake> {
    let key = DataKey::UserStake(user.clone());
    let value: Option<UserStake> = env.storage().persistent().get(&key);
    if value.is_some() {
        bump_user(env, &key);
    }
    value
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

fn set_banked_credits(env: &Env, user: &Address, credits: i128) {
    let key = DataKey::BankedCredits(user.clone());
    env.storage().persistent().set(&key, &credits);
    bump_user(env, &key);
}

fn get_position(env: &Env, user: &Address) -> Option<Position> {
    let key = DataKey::UserPosition(user.clone());
    let value: Option<Position> = env.storage().persistent().get(&key);
    if value.is_some() {
        bump_user(env, &key);
    }
    value
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

fn compute_total_stake(amount: i128, allocation_pct: u32, multiplier: u32) -> Result<i128, PoolError> {
    let boosted = amount
        .checked_mul(allocation_pct as i128)
        .ok_or(PoolError::CreditOverflow)?
        / 100;
    let principal = amount
        .checked_sub(boosted)
        .ok_or(PoolError::CreditOverflow)?;
    let virtual_stake = boosted
        .checked_mul(multiplier as i128)
        .ok_or(PoolError::CreditOverflow)?;
    principal
        .checked_add(virtual_stake)
        .ok_or(PoolError::CreditOverflow)
}

fn compute_credits(
    amount: i128,
    allocation_pct: u32,
    multiplier: u32,
    credit_rate: i128,
    ledgers_elapsed: u32,
) -> Result<i128, PoolError> {
    let total_stake = compute_total_stake(amount, allocation_pct, multiplier)?;
    total_stake
        .checked_mul(credit_rate)
        .and_then(|v| v.checked_mul(ledgers_elapsed as i128))
        .ok_or(PoolError::CreditOverflow)
}

fn checkpoint(env: &Env, user: &Address, stake: &mut UserStake) -> Result<(), PoolError> {
    let allocation_pct = get_user_boost(env, user).unwrap_or(0);
    let multiplier = read_global_multiplier(env);
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(stake.start_ledger);
    let credits = compute_credits(
        stake.amount,
        allocation_pct,
        multiplier,
        stake.credit_rate,
        elapsed,
    )?;
    stake.credits_banked = stake
        .credits_banked
        .checked_add(credits)
        .ok_or(PoolError::CreditOverflow)?;
    stake.start_ledger = current;
    stake.credit_rate = read_credit_rate(env);
    Ok(())
}

fn checkpoint_position(env: &Env, position: &mut Position) -> Result<(), PoolError> {
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(position.checkpoint_ledger);
    let credits = position
        .amount
        .checked_mul(position.credit_rate)
        .and_then(|v| v.checked_mul(elapsed as i128))
        .ok_or(PoolError::CreditOverflow)?;
    position.total_credits = position
        .total_credits
        .checked_add(credits)
        .ok_or(PoolError::CreditOverflow)?;
    position.checkpoint_ledger = current;
    position.credit_rate = read_credit_rate(env);
    Ok(())
}

#[contract]
pub struct FarmingPool;

#[contractimpl]
impl FarmingPool {
    pub fn initialize(
        env: Env,
        admin: Address,
        stake_token: Address,
        global_multiplier: u32,
        credit_rate: i128,
        min_lock_period: u32,
    ) -> Result<(), PoolError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(PoolError::AlreadyInitialized);
        }
        if global_multiplier < 1 {
            return Err(PoolError::InvalidMultiplier);
        }
        if credit_rate <= 0 {
            return Err(PoolError::InvalidCreditRate);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::StakeToken, &stake_token);
        env.storage()
            .instance()
            .set(&DataKey::GlobalMultiplier, &global_multiplier);
        env.storage()
            .instance()
            .set(&DataKey::CreditRate, &credit_rate);
        env.storage()
            .instance()
            .set(&DataKey::MinLockPeriod, &min_lock_period);
        bump_instance(&env);
        Ok(())
    }

    pub fn admin(env: Env) -> Address {
        bump_instance(&env);
        get_admin(&env).unwrap()
    }

    pub fn transfer_admin(env: Env, new_admin: Address) {
        let current = get_admin(&env).unwrap();
        current.require_auth();
        bump_instance(&env);

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );
    }

    pub fn lock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        bump_instance(&env);

        let current = env.ledger().sequence();
        let mut position = if let Some(mut existing) = get_position(&env, &user) {
            checkpoint_position(&env, &mut existing)?;
            existing.amount += amount;
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

        // Checks-effects-interactions: persist state *before* the external
        // token transfer below. `stake_token` is an admin-supplied address,
        // not necessarily a trusted Stellar Asset Contract, and its
        // `transfer` is a synchronous cross-contract call that could
        // otherwise observe (or, on a future host that permits it, mutate)
        // this position while it's still only a local variable. If the
        // transfer fails, the whole invocation reverts and this write is
        // rolled back with it — Soroban's per-invocation atomicity, not
        // manual sequencing, is what keeps this safe on failure. See #69.
        set_position(&env, &user, &position);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("locked")),
            (user, amount),
        );
        Ok(())
    }

    pub fn unlock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;

        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        bump_instance(&env);

        let mut position = get_position(&env, &user).ok_or(PoolError::NoActiveStake)?;
        if amount > position.amount {
            return Err(PoolError::InsufficientBalance);
        }

        let current = env.ledger().sequence();
        if current < position.unlock_ledger {
            return Err(PoolError::LockPeriodNotElapsed);
        }

        // Try to checkpoint credits; if the credit computation overflows,
        // fall back to the previously banked credits so the unlock still works.
        if checkpoint_position(&env, &mut position).is_err() {
            // proceed with existing total_credits (already banked)
        }
        let total_credits = position.total_credits;
        position.amount -= amount;

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &user,
            &amount,
        );

        if position.amount == 0 {
            remove_position(&env, &user);
        } else {
            set_position(&env, &user, &position);
        }

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("unlocked")),
            (user, amount, total_credits),
        );
        Ok(())
    }

    pub fn calculate_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let Some(position) = get_position(&env, &user) else {
            return Ok(0);
        };

        let elapsed = env
            .ledger()
            .sequence()
            .saturating_sub(position.checkpoint_ledger);
        let accruing = position
            .amount
            .checked_mul(position.credit_rate)
            .and_then(|v| v.checked_mul(elapsed as i128))
            .ok_or(PoolError::CreditOverflow)?;
        position
            .total_credits
            .checked_add(accruing)
            .ok_or(PoolError::CreditOverflow)
    }

    pub fn get_user_position(env: Env, user: Address) -> Result<Option<Position>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_position(&env, &user))
    }

    pub fn pause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &true);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("paused")), ());
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("unpaused")), ());
        Ok(())
    }

    pub fn is_paused(env: Env) -> Result<bool, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(pool_is_paused(&env))
    }

    pub fn emergency_withdraw(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        if !pool_is_paused(&env) {
            return Err(PoolError::NotPaused);
        }
        bump_instance(&env);

        let mut total_returned = 0i128;
        let mut banked_credits = 0i128;
        let stake_token = get_stake_token(&env)?;
        let token = token::TokenClient::new(&env, &stake_token);

        if let Some(position) = get_position(&env, &user) {
            token.transfer(&env.current_contract_address(), &user, &position.amount);
            total_returned += position.amount;
            banked_credits += position.total_credits;
            remove_position(&env, &user);
        }

        if let Some(stake) = get_user_stake(&env, &user) {
            token.transfer(&env.current_contract_address(), &user, &stake.amount);
            total_returned += stake.amount;
            banked_credits += stake.credits_banked;
            remove_user_stake(&env, &user);
        }

        if total_returned == 0 {
            return Err(PoolError::NoActiveStake);
        }

        if banked_credits > 0 {
            set_banked_credits(&env, &user, banked_credits);
        }

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("emrg_exit")),
            (admin, user, total_returned),
        );
        Ok(total_returned)
    }

    pub fn get_banked_credits(env: Env, user: Address) -> i128 {
        bump_instance(&env);
        let key = DataKey::BankedCredits(user);
        let value: Option<i128> = env.storage().persistent().get(&key);
        if value.is_some() {
            bump_user(&env, &key);
        }
        value.unwrap_or(0)
    }

    pub fn stake(env: Env, from: Address, amount: i128) -> Result<(), PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;
        if amount <= 0 {
            return Err(PoolError::InvalidAmount);
        }
        bump_instance(&env);

        let current = env.ledger().sequence();
        let mut new_stake = if let Some(mut existing) = get_user_stake(&env, &from) {
            checkpoint(&env, &from, &mut existing)?;
            existing.amount += amount;
            existing
        } else {
            UserStake {
                amount,
                start_ledger: current,
                credits_banked: 0,
                credit_rate: read_credit_rate(&env),
            }
        };

        new_stake.credit_rate = read_credit_rate(&env);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );

        set_user_stake(&env, &from, &new_stake);
        Ok(())
    }

    pub fn unstake(env: Env, from: Address) -> Result<i128, PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;
        bump_instance(&env);

        let mut stake = get_user_stake(&env, &from).ok_or(PoolError::NoActiveStake)?;
        // Try to checkpoint credits; if the credit computation overflows,
        // fall back to returning whatever credits were already banked.
        if checkpoint(&env, &from, &mut stake).is_err() {
            // proceed with previously banked credits only
        }
        let total_credits = stake.credits_banked;

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &from,
            &stake.amount,
        );

        remove_user_stake(&env, &from);
        Ok(total_credits)
    }

    pub fn set_boost(env: Env, user: Address, allocation_pct: u32) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;
        if !(1..=100).contains(&allocation_pct) {
            return Err(PoolError::InvalidAllocation);
        }
        bump_instance(&env);

        if let Some(mut stake) = get_user_stake(&env, &user) {
            checkpoint(&env, &user, &mut stake)?;
            set_user_stake(&env, &user, &stake);
        }

        let key = DataKey::UserBoost(user.clone());
        env.storage().persistent().set(&key, &allocation_pct);
        bump_user(&env, &key);

        let multiplier = read_global_multiplier(&env);
        env.events().publish(
            (symbol_short!("boost"), symbol_short!("applied")),
            (user, allocation_pct, multiplier),
        );
        Ok(())
    }

    pub fn get_boost_config(env: Env, user: Address) -> Result<Option<BoostConfig>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(
            get_user_boost(&env, &user).map(|allocation_pct| BoostConfig {
                multiplier: read_global_multiplier(&env),
                allocation_pct,
            }),
        )
    }

    pub fn set_global_multiplier(env: Env, multiplier: u32) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if multiplier < 1 {
            return Err(PoolError::InvalidMultiplier);
        }
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::GlobalMultiplier, &multiplier);
        env.events().publish(
            (symbol_short!("boost"), symbol_short!("mult_set")),
            multiplier,
        );
        Ok(())
    }

    pub fn set_credit_rate(env: Env, new_rate: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if new_rate <= 0 {
            return Err(PoolError::InvalidCreditRate);
        }
        bump_instance(&env);

        let old_rate = read_credit_rate(&env);
        env.storage()
            .instance()
            .set(&DataKey::CreditRate, &new_rate);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("rate_set")),
            (old_rate, new_rate),
        );
        Ok(())
    }

    pub fn set_min_lock_period(env: Env, new_period: u32) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let old_period = read_min_lock_period(&env);
        env.storage()
            .instance()
            .set(&DataKey::MinLockPeriod, &new_period);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("lock_set")),
            (old_period, new_period),
        );
        Ok(())
    }

    pub fn credit_rate(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_credit_rate(&env))
    }

    pub fn get_credit_rate(env: Env) -> Result<i128, PoolError> {
        Self::credit_rate(env)
    }

    pub fn min_lock_period(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_min_lock_period(&env))
    }

    pub fn get_min_lock_period(env: Env) -> Result<u32, PoolError> {
        Self::min_lock_period(env)
    }

    pub fn get_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let Some(stake) = get_user_stake(&env, &user) else {
            return Ok(0);
        };

        let allocation_pct = get_user_boost(&env, &user).unwrap_or(0);
        let multiplier = read_global_multiplier(&env);
        let elapsed = env.ledger().sequence().saturating_sub(stake.start_ledger);
        let accruing = compute_credits(
            stake.amount,
            allocation_pct,
            multiplier,
            stake.credit_rate,
            elapsed,
        )?;
        stake
            .credits_banked
            .checked_add(accruing)
            .ok_or(PoolError::CreditOverflow)
    }

    pub fn get_stake(env: Env, user: Address) -> Result<Option<UserStake>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_user_stake(&env, &user))
    }
}

mod test;
