#![no_std]

mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short, token, Address, Env,
};
use types::{BoostConfig, DataKey, Position, UserStake};

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5s/ledger).
const USER_TTL_THRESHOLD: u32 = 518_400;
const USER_TTL_EXTEND_TO: u32 = 1_036_800;

// ── Storage helpers ───────────────────────────────────────────────────────────

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

fn get_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn get_global_multiplier(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::GlobalMultiplier)
        .unwrap_or(1)
}

fn get_credit_rate(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::CreditRate)
        .unwrap_or(1)
}

fn get_stake_token(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::StakeToken).unwrap()
}

fn get_min_lock_period(env: &Env) -> u32 {
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
    let val: Option<u32> = env.storage().persistent().get(&key);
    if val.is_some() {
        bump_user(env, &key);
    }
    val
}

fn get_user_stake(env: &Env, user: &Address) -> Option<UserStake> {
    let key = DataKey::UserStake(user.clone());
    let val: Option<UserStake> = env.storage().persistent().get(&key);
    if val.is_some() {
        bump_user(env, &key);
    }
    val
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
    let key = DataKey::UserPosition(user.clone());
    let val: Option<Position> = env.storage().persistent().get(&key);
    if val.is_some() {
        bump_user(env, &key);
    }
    val
}

fn set_position(env: &Env, user: &Address, pos: &Position) {
    let key = DataKey::UserPosition(user.clone());
    env.storage().persistent().set(&key, pos);
    bump_user(env, &key);
}

fn remove_position(env: &Env, user: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::UserPosition(user.clone()));
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
    let virtual_s = boosted * multiplier as i128;
    principal + virtual_s
}

/// Credits earned over `ledgers_elapsed` ledgers at the given stake and boost.
fn compute_credits(
    amount: i128,
    allocation_pct: u32,
    multiplier: u32,
    credit_rate: i128,
    ledgers_elapsed: u32,
) -> i128 {
    compute_total_stake(amount, allocation_pct, multiplier) * credit_rate * ledgers_elapsed as i128
}

/// Checkpoint a user's earned credits into `credits_banked` and reset `start_ledger`.
/// Call this before any change that affects the credit accrual rate (boost or stake amount).
fn checkpoint(env: &Env, user: &Address, stake: &mut UserStake) {
    let allocation_pct = get_user_boost(env, user).unwrap_or(0);
    let multiplier = get_global_multiplier(env);
    let rate = get_credit_rate(env);
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(stake.start_ledger);
    stake.credits_banked += compute_credits(stake.amount, allocation_pct, multiplier, rate, elapsed);
    stake.start_ledger = current;
}

/// Checkpoint a position's earned credits and advance the checkpoint ledger.
fn checkpoint_position(env: &Env, pos: &mut Position) {
    let rate = get_credit_rate(env);
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(pos.checkpoint_ledger);
    pos.total_credits += pos.amount * rate * elapsed as i128;
    pos.checkpoint_ledger = current;
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct FarmingPool;

#[contractimpl]
impl FarmingPool {
    /// Initialise the contract. Must be called exactly once before any other function.
    ///
    /// - `global_multiplier`: initial boost multiplier (≥ 1). E.g. `2` = 2× boosted virtual stake.
    /// - `credit_rate`: credits accrued per unit of effective stake per ledger.
    /// - `min_lock_period`: minimum number of ledgers a position must be locked before unlock.
    pub fn initialize(
        env: Env,
        admin: Address,
        stake_token: Address,
        global_multiplier: u32,
        credit_rate: i128,
        min_lock_period: u32,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        assert!(global_multiplier >= 1, "multiplier must be >= 1");
        assert!(credit_rate > 0, "credit_rate must be positive");

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::StakeToken, &stake_token);
        env.storage().instance().set(&DataKey::GlobalMultiplier, &global_multiplier);
        env.storage().instance().set(&DataKey::CreditRate, &credit_rate);
        env.storage().instance().set(&DataKey::MinLockPeriod, &min_lock_period);
        bump_instance(&env);
    }

    // ── Lock / Unlock system ─────────────────────────────────────────────────

    /// Lock `amount` tokens for the caller. If a prior position exists, credits are
    /// checkpointed first and the new amount is added to the existing position.
    ///
    /// Emits a `("pool", "locked")` event with `(user, amount)`.
    pub fn lock_assets(env: Env, user: Address, amount: i128) {
        user.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(amount > 0, "amount must be positive");
        bump_instance(&env);

        let current = env.ledger().sequence();
        let pos = if let Some(mut existing) = get_position(&env, &user) {
            checkpoint_position(&env, &mut existing);
            existing.amount += amount;
            existing
        } else {
            Position {
                amount,
                lock_ledger: current,
                checkpoint_ledger: current,
                total_credits: 0,
            }
        };

        token::TokenClient::new(&env, &get_stake_token(&env))
            .transfer(&user, &env.current_contract_address(), &amount);

        set_position(&env, &user, &pos);

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("locked")),
            (user, amount),
        );
    }

    /// Unlock `amount` tokens for the caller. The minimum lock period (in ledgers) must
    /// have elapsed since the position was created. Partial unlocks are supported; the
    /// remaining balance stays locked.
    ///
    /// Emits a `("pool", "unlocked")` event with `(user, amount, total_credits)`.
    pub fn unlock_assets(env: Env, user: Address, amount: i128) {
        user.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(amount > 0, "amount must be positive");
        bump_instance(&env);

        let mut pos = get_position(&env, &user).expect("no active position");
        assert!(amount <= pos.amount, "insufficient locked balance");

        let current = env.ledger().sequence();
        let min_lock = get_min_lock_period(&env);
        assert!(
            current >= pos.lock_ledger.saturating_add(min_lock),
            "minimum lock period not elapsed"
        );

        checkpoint_position(&env, &mut pos);
        let total_credits = pos.total_credits;
        pos.amount -= amount;

        token::TokenClient::new(&env, &get_stake_token(&env))
            .transfer(&env.current_contract_address(), &user, &amount);

        if pos.amount == 0 {
            remove_position(&env, &user);
        } else {
            set_position(&env, &user, &pos);
        }

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("unlocked")),
            (user, amount, total_credits),
        );
    }

    /// Return total credits for `user` (banked + currently accruing). Returns 0 if no position.
    pub fn calculate_credits(env: Env, user: Address) -> i128 {
        bump_instance(&env);
        let Some(pos) = get_position(&env, &user) else {
            return 0;
        };
        let rate = get_credit_rate(&env);
        let elapsed = env.ledger().sequence().saturating_sub(pos.checkpoint_ledger);
        pos.total_credits + pos.amount * rate * elapsed as i128
    }

    /// Return the current position for `user`, or `None` if no position exists.
    pub fn get_user_position(env: Env, user: Address) -> Option<Position> {
        bump_instance(&env);
        get_position(&env, &user)
    }

    // ── Pause / Unpause ───────────────────────────────────────────────────────

    /// Admin: pause the pool. While paused, `lock_assets`, `unlock_assets`, `stake`,
    /// `unstake`, and `set_boost` are blocked.
    ///
    /// Emits a `("pool", "paused")` event.
    pub fn pause(env: Env) {
        get_admin(&env).require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &true);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("paused")),
            (),
        );
    }

    /// Admin: unpause the pool, restoring normal operation.
    ///
    /// Emits a `("pool", "unpaused")` event.
    pub fn unpause(env: Env) {
        get_admin(&env).require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("unpaused")),
            (),
        );
    }

    /// Return whether the pool is currently paused.
    pub fn is_paused(env: Env) -> bool {
        bump_instance(&env);
        pool_is_paused(&env)
    }

    // ── Boost / Stake system (unchanged) ─────────────────────────────────────

    /// Stake `amount` tokens. If a prior stake exists, earned credits are checkpointed first.
    pub fn stake(env: Env, from: Address, amount: i128) {
        from.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(amount > 0, "amount must be positive");
        bump_instance(&env);

        let current = env.ledger().sequence();
        let new_stake = if let Some(mut existing) = get_user_stake(&env, &from) {
            checkpoint(&env, &from, &mut existing);
            existing.amount += amount;
            existing
        } else {
            UserStake {
                amount,
                start_ledger: current,
                credits_banked: 0,
            }
        };

        // Pull tokens from caller into the contract.
        token::TokenClient::new(&env, &get_stake_token(&env))
            .transfer(&from, &env.current_contract_address(), &amount);

        set_user_stake(&env, &from, &new_stake);
    }

    /// Unstake all tokens. Returns the total credits earned.
    pub fn unstake(env: Env, from: Address) -> i128 {
        from.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        bump_instance(&env);

        let mut stake = get_user_stake(&env, &from).expect("no active stake");
        checkpoint(&env, &from, &mut stake);
        let total_credits = stake.credits_banked;

        // Return staked tokens to caller.
        token::TokenClient::new(&env, &get_stake_token(&env))
            .transfer(&env.current_contract_address(), &from, &stake.amount);

        remove_user_stake(&env, &from);
        total_credits
    }

    /// Set the caller's boost allocation percentage (1–100%).
    ///
    /// Credits earned under the previous allocation are checkpointed first so no
    /// rewards are lost when the boost is updated.
    ///
    /// Emits a `boost_applied` event.
    pub fn set_boost(env: Env, user: Address, allocation_pct: u32) {
        user.require_auth();
        assert!(!pool_is_paused(&env), "pool is paused");
        assert!(
            allocation_pct >= 1 && allocation_pct <= 100,
            "allocation_pct must be 1–100"
        );
        bump_instance(&env);

        // Checkpoint before changing the allocation so prior credits are preserved.
        if let Some(mut stake) = get_user_stake(&env, &user) {
            checkpoint(&env, &user, &mut stake);
            set_user_stake(&env, &user, &stake);
        }

        let key = DataKey::UserBoost(user.clone());
        env.storage().persistent().set(&key, &allocation_pct);
        bump_user(&env, &key);

        let multiplier = get_global_multiplier(&env);
        env.events().publish(
            (symbol_short!("boost"), symbol_short!("applied")),
            (user, allocation_pct, multiplier),
        );
    }

    /// Return the current boost configuration for `user`, or `None` if no boost is set.
    ///
    /// `BoostConfig.multiplier` reflects the current global multiplier.
    /// `BoostConfig.allocation_pct` is the user's chosen allocation.
    pub fn get_boost_config(env: Env, user: Address) -> Option<BoostConfig> {
        bump_instance(&env);
        get_user_boost(&env, &user).map(|allocation_pct| BoostConfig {
            multiplier: get_global_multiplier(&env),
            allocation_pct,
        })
    }

    /// Admin: update the global boost multiplier.
    ///
    /// Emits a `mult_set` event. Note that in-flight credits are not retroactively
    /// recalculated; the new multiplier applies from the next ledger onward for
    /// users whose boost configs are not checkpointed yet.
    pub fn set_global_multiplier(env: Env, multiplier: u32) {
        get_admin(&env).require_auth();
        assert!(multiplier >= 1, "multiplier must be >= 1");
        bump_instance(&env);

        env.storage().instance().set(&DataKey::GlobalMultiplier, &multiplier);

        env.events().publish(
            (symbol_short!("boost"), symbol_short!("mult_set")),
            multiplier,
        );
    }

    /// Return total credits for `user` in the boost/stake system (banked + currently accruing).
    pub fn get_credits(env: Env, user: Address) -> i128 {
        bump_instance(&env);
        let Some(stake) = get_user_stake(&env, &user) else {
            return 0;
        };
        let allocation_pct = get_user_boost(&env, &user).unwrap_or(0);
        let multiplier = get_global_multiplier(&env);
        let rate = get_credit_rate(&env);
        let elapsed = env.ledger().sequence().saturating_sub(stake.start_ledger);
        stake.credits_banked + compute_credits(stake.amount, allocation_pct, multiplier, rate, elapsed)
    }

    /// Return the current stake record for `user`, or `None` if not staked.
    pub fn get_stake(env: Env, user: Address) -> Option<UserStake> {
        bump_instance(&env);
        get_user_stake(&env, &user)
    }
}

mod test;
