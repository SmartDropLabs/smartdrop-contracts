#![no_std]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Vec};
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

fn whitelist_enabled(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistEnabled)
        .unwrap_or(false)
}

fn is_user_whitelisted(env: &Env, user: &Address) -> bool {
    let key = DataKey::Whitelisted(user.clone());
    let ok = env.storage().persistent().get(&key).unwrap_or(false);
    if ok {
        bump_user(env, &key);
    }
    ok
}

// ── Boost calculation ─────────────────────────────────────────────────────────

/// Compute the effective total stake for credit accrual.
///
/// Splits `amount` into a principal portion and a boosted virtual portion:
///   boosted_amount  = amount * allocation_pct / 100
///   principal_stake = amount - boosted_amount
///   virtual_stake   = boosted_amount * multiplier
///   total_stake     = principal_stake + virtual_stake
///
/// With no boost (allocation_pct = 0) total_stake == amount.
fn compute_total_stake(amount: i128, allocation_pct: u32, multiplier: u32) -> i128 {
    let boosted = amount * allocation_pct as i128 / 100;
    let principal = amount - boosted;
    let virtual_stake = boosted * multiplier as i128;
    principal + virtual_stake
}

fn compute_credits(
    amount: i128,
    allocation_pct: u32,
    multiplier: u32,
    credit_rate: i128,
    ledgers_elapsed: u32,
) -> i128 {
    compute_total_stake(amount, allocation_pct, multiplier) * credit_rate * ledgers_elapsed as i128
}

fn checkpoint(env: &Env, user: &Address, stake: &mut UserStake) {
    let allocation_pct = get_user_boost(env, user).unwrap_or(0);
    let multiplier = read_global_multiplier(env);
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(stake.start_ledger);
    stake.credits_banked += compute_credits(
        stake.amount,
        allocation_pct,
        multiplier,
        stake.credit_rate,
        elapsed,
    );
    stake.start_ledger = current;
    stake.credit_rate = read_credit_rate(env);
}

fn checkpoint_position(env: &Env, position: &mut Position) {
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(position.checkpoint_ledger);
    position.total_credits += position.amount * position.credit_rate * elapsed as i128;
    position.checkpoint_ledger = current;
    position.credit_rate = read_credit_rate(env);
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
        assert!(global_multiplier >= 1, "multiplier must be >= 1");
        assert!(credit_rate > 0, "credit_rate must be positive");

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
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(amount > 0, "amount must be positive");

        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &user) {
            return Err(PoolError::NotWhitelisted);
        }

        bump_instance(&env);

        let current = env.ledger().sequence();
        let mut position = if let Some(mut existing) = get_position(&env, &user) {
            checkpoint_position(&env, &mut existing);
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

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &user,
            &env.current_contract_address(),
            &amount,
        );

        set_position(&env, &user, &position);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("locked")),
            (user, amount),
        );
        Ok(())
    }

    pub fn unlock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(amount > 0, "amount must be positive");
        bump_instance(&env);

        let mut position = get_position(&env, &user).expect("no active position");
        assert!(amount <= position.amount, "insufficient locked balance");

        let current = env.ledger().sequence();
        assert!(
            current >= position.unlock_ledger,
            "minimum lock period not elapsed"
        );

        checkpoint_position(&env, &mut position);
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
        Ok(position.total_credits + position.amount * position.credit_rate * elapsed as i128)
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

    // ── Whitelist system ──────────────────────────────────────────────────────

    /// Admin: enable whitelist mode. Admin must authorise.
    pub fn enable_whitelist(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::WhitelistEnabled, &true);
        Ok(())
    }

    /// Admin: disable whitelist mode. Admin must authorise.
    pub fn disable_whitelist(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::WhitelistEnabled, &false);
        Ok(())
    }

    /// Admin: add `user` to the whitelist. Admin must authorise.
    pub fn add_to_whitelist(env: Env, user: Address) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let key = DataKey::Whitelisted(user.clone());
        env.storage().persistent().set(&key, &true);
        bump_user(&env, &key);
        Ok(())
    }

    /// Admin: remove `user` from the whitelist. Admin must authorise.
    pub fn remove_from_whitelist(env: Env, user: Address) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let key = DataKey::Whitelisted(user.clone());
        env.storage().persistent().remove(&key);
        Ok(())
    }

    /// Public: check if `user` is whitelisted. Bumps TTL of the entry if whitelisted.
    pub fn is_whitelisted(env: Env, user: Address) -> bool {
        bump_instance(&env);
        is_user_whitelisted(&env, &user)
    }

    /// Admin: batch add multiple `users` to the whitelist. Capped at 50 addresses per call. Admin must authorise.
    pub fn batch_add_to_whitelist(env: Env, users: Vec<Address>) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        assert!(users.len() <= 50, "max 50 addresses per call");
        bump_instance(&env);

        for user in users.iter() {
            let key = DataKey::Whitelisted(user.clone());
            env.storage().persistent().set(&key, &true);
            bump_user(&env, &key);
        }
        Ok(())
    }

    // ── Boost / Stake system ─────────────────────────────────────────────────

    /// Stake `amount` tokens. If a prior stake exists, earned credits are checkpointed first.
    pub fn stake(env: Env, from: Address, amount: i128) -> Result<(), PoolError> {
        from.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        require_initialized(&env)?;
        assert!(amount > 0, "amount must be positive");

        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &from) {
            return Err(PoolError::NotWhitelisted);
        }

        bump_instance(&env);

        let current = env.ledger().sequence();
        let mut new_stake = if let Some(mut existing) = get_user_stake(&env, &from) {
            checkpoint(&env, &from, &mut existing);
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

        // Pull tokens from caller into the contract.
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
        assert!(!pool_is_paused(&env), "pool is paused");
        require_initialized(&env)?;
        bump_instance(&env);

        let mut stake = get_user_stake(&env, &from).expect("no active stake");
        checkpoint(&env, &from, &mut stake);
        let total_credits = stake.credits_banked;

        // Return staked tokens to caller.
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
        assert!(!pool_is_paused(&env), "pool is paused");
        require_initialized(&env)?;
        assert!(
            allocation_pct >= 1 && allocation_pct <= 100,
            "allocation_pct must be 1-100"
        );
        bump_instance(&env);

        if let Some(mut stake) = get_user_stake(&env, &user) {
            checkpoint(&env, &user, &mut stake);
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
        assert!(multiplier >= 1, "multiplier must be >= 1");
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
        Ok(stake.credits_banked
            + compute_credits(
                stake.amount,
                allocation_pct,
                multiplier,
                stake.credit_rate,
                elapsed,
            ))
    }

    pub fn get_stake(env: Env, user: Address) -> Result<Option<UserStake>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_user_stake(&env, &user))
    }
}

mod test;
