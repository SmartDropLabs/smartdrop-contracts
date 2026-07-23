#![no_std]

mod types;
#[cfg(test)]
mod mock_reentrant_token;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, BytesN, Env};
pub use types::PoolError;
use types::{BoostConfig, DataKey, Position, UserStake};

pub const SCHEMA_VERSION: u32 = 1;

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5s/ledger).
const USER_TTL_THRESHOLD: u32 = 518_400;
const USER_TTL_EXTEND_TO: u32 = 1_036_800;

/// Sanity ceilings on `global_multiplier` and `credit_rate` (see #89).
///
/// `compute_credits` computes
/// `compute_total_stake(amount, allocation_pct, multiplier) * credit_rate * ledgers_elapsed`,
/// and `compute_total_stake` reduces to exactly `amount * multiplier` at
/// `allocation_pct = 100` (its worst case: `boosted = amount`, `principal = 0`).
/// The `/100` division in `compute_total_stake` therefore does *not* loosen
/// this bound at the boundary — the naive product below is tight, not a
/// conservative over-estimate.
///
/// Worst-case overflow chain:
///
/// ```text
/// amount_max * multiplier_max * credit_rate_max * elapsed_max <= i128::MAX / 16
/// ```
///
/// Inputs, chosen and justified independently of the multiplier/credit-rate
/// ceilings themselves:
/// - `amount_max = 10^18` — 100 billion whole tokens at Stellar's standard
///   7-decimal ("stroop") convention. Far above any realistic pool TVL, but
///   many orders of magnitude below `i128::MAX` (~1.7 x 10^38).
/// - `elapsed_max = 63_072_000` ledgers — ~10 years at 5s/ledger
///   (`10 * 365 * 24 * 3600 / 5`), a multi-year operational horizon between
///   checkpoints.
/// - Headroom factor of 16x, i.e. the worst-case product must not exceed
///   `i128::MAX / 16`, leaving ample margin beyond the bare non-overflow
///   requirement.
///
/// Solving for `multiplier_max * credit_rate_max`:
/// `(i128::MAX / 16) / (amount_max * elapsed_max) ≈ 1.686 x 10^11`.
///
/// Chosen ceilings (round, human-readable, at or below the derived bound):
/// - `MAX_GLOBAL_MULTIPLIER = 1_000`
/// - `MAX_CREDIT_RATE = 100_000_000` (10^8)
/// - product = 10^11, comfortably under the 1.686 x 10^11 budget.
///
/// Verification: `amount_max * multiplier_max * credit_rate_max * elapsed_max`
/// = `10^18 * 1_000 * 10^8 * 63_072_000` ≈ `6.307 x 10^36`, versus
/// `i128::MAX ≈ 1.701 x 10^38` — a headroom ratio of ~27x, comfortably
/// exceeding the required 16x. (For reference, the earlier sketch pair of
/// 1_000 / 1_000_000_000 gives a worst case of ≈ 6.307 x 10^37, which fits
/// under raw `i128::MAX` but only with ~2.7x headroom — it does not survive
/// this derivation's 16x margin, hence `MAX_CREDIT_RATE` here is 10x smaller.)
const MAX_GLOBAL_MULTIPLIER: u32 = 1_000;
const MAX_CREDIT_RATE: i128 = 100_000_000;

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

fn read_schema_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::SchemaVersion)
        .unwrap_or(SCHEMA_VERSION)
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
    /// Initialize the pool. `global_multiplier` and `credit_rate` are bounded
    /// by `MAX_GLOBAL_MULTIPLIER`/`MAX_CREDIT_RATE` — see #89 for the
    /// overflow-safety derivation shared with `set_global_multiplier` and
    /// `set_credit_rate`.
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
        // Ceilings mirror `set_global_multiplier`/`set_credit_rate` — see #89.
        if !(1..=MAX_GLOBAL_MULTIPLIER).contains(&global_multiplier) {
            return Err(PoolError::InvalidGlobalMultiplier);
        }
        if credit_rate <= 0 || credit_rate > MAX_CREDIT_RATE {
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
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        bump_instance(&env);
        Ok(())
    }

    pub fn admin(env: Env) -> Result<Address, PoolError> {
        bump_instance(&env);
        get_admin(&env)
    }

    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), PoolError> {
        let current = get_admin(&env)?;
        current.require_auth();
        bump_instance(&env);

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );
        Ok(())
    }

    pub fn schema_version(env: Env) -> u32 {
        bump_instance(&env);
        read_schema_version(&env)
    }

    pub fn migrate(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let current = read_schema_version(&env);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        Ok(current)
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("upgraded")),
            new_wasm_hash.clone(),
        );
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    pub fn lock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;

        assert!(amount > 0, "amount must be positive");
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

    pub fn stake(env: Env, from: Address, amount: i128) -> Result<(), PoolError> {
        from.require_auth();
        require_not_paused(&env)?;

        require_initialized(&env)?;
        assert!(amount > 0, "amount must be positive");
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
        require_not_paused(&env)?;

        require_initialized(&env)?;
        bump_instance(&env);

        let mut stake = get_user_stake(&env, &from).expect("no active stake");
        checkpoint(&env, &from, &mut stake);
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
        require_not_paused(&env)?;


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

    /// Set the global credit multiplier. Rejects 0 and anything above
    /// `MAX_GLOBAL_MULTIPLIER` — see #89 for the overflow-safety derivation.
    pub fn set_global_multiplier(env: Env, multiplier: u32) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if !(1..=MAX_GLOBAL_MULTIPLIER).contains(&multiplier) {
            return Err(PoolError::InvalidGlobalMultiplier);
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

    /// Set the credit accrual rate. Rejects non-positive values and anything
    /// above `MAX_CREDIT_RATE` — see #89 for the overflow-safety derivation.
    pub fn set_credit_rate(env: Env, new_rate: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if new_rate <= 0 || new_rate > MAX_CREDIT_RATE {
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
