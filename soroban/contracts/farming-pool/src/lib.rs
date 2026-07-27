                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       #![no_std]
#![allow(deprecated)]

#[cfg(test)]
mod mock_reentrant_token;
mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Vec};
use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};
use types::{BoostConfig, DataKey, PoolError, Position, UserStake};
use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, BytesN, Env};
pub use types::PoolError;
use types::{BoostConfig, DataKey, Position, UserStake};

// Expose compiled WASM bytes so sibling crates (e.g. `factory`) can upload the
// real farming-pool contract in their integration tests via:
//   `env.deployer().upload_contract_wasm(farming_pool::WASM)`
// Gated behind `testutils` feature (enabled by factory's dev-dependency) so it
// is never included in on-chain release builds.
#[cfg(any(test, feature = "testutils"))]
pub const WASM: &[u8] = soroban_sdk::contractfile!(
    file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../target/wasm32v1-none/release/farming_pool.wasm"
    ),
);

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
        min_stake_amount: i128,
    ) -> Result<(), PoolError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(PoolError::AlreadyInitialized);
        }
        if global_multiplier < 1 {
            return Err(PoolError::InvalidMultiplier);
        }
        if credit_rate <= 0 {
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
            .set(&DataKey::MinStakeAmount, &min_stake_amount);
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        bump_instance(&env);
        Ok(())
    }

    pub fn admin(env: Env) -> Result<Address, PoolError> {
        bump_instance(&env);
        get_admin(&env).unwrap()
    }

    /// Admin: transfer admin rights to `new_admin`. Current admin must authorise.
    ///
    /// Supports key rotation and governance handoffs without redeploying the pool.
    /// Emits a `("pool", "adm_xfr")` event with `(old_admin, new_admin)`.
    pub fn transfer_admin(env: Env, new_admin: Address) {
        let current = get_admin(&env).unwrap();
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

        // token::TokenClient::new(&env, &get_stake_token(&env)).transfer(
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
            env.current_contract_address(),
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

        // token::TokenClient::new(&env, &get_stake_token(&env)).transfer(
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
            .saturating_sub(pos.checkpoint_ledger);
        pos.total_credits + pos.amount * rate * elapsed as i128;
        Ok(pos.total_credits + pos.amount * rate * elapsed as i128)
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
        get_admin(&env).unwrap().require_auth();
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        admin.require_auth();
        if !pool_is_paused(&env) {
            return Err(PoolError::NotPaused);
        }
        bump_instance(&env);

        let mut total_returned: i128 = 0;
        let mut banked_credits: i128 = 0;
        let stake_token = get_stake_token(&env)?;
        let token = token::TokenClient::new(&env, &stake_token);

        // ── Checks-effects-interactions ──────────────────────────────────────
        // Both branches below clear per-user storage *before* the external
        // token.transfer call.  `stake_token` is an admin-supplied address,
        // not necessarily a trusted Stellar Asset Contract, and its
        // `transfer` is a synchronous cross-contract call.  Clearing the
        // record first ensures that a reentrant call (if the host ever
        // permitted same-contract reentry) would see None/an already-cleared
        // record and cannot double-payout the same user's funds.
        //
        // This is the designated incident-response path — during an active
        // emergency the token itself (or its configuration) is most likely to
        // be unusual or compromised, making CEI discipline here especially
        // important.  See #72.
        // ──────────────────────────────────────────────────────────────────────

        if let Some(position) = get_position(&env, &user) {
            remove_position(&env, &user);
            total_returned += position.amount;
            banked_credits += position.total_credits;
            token.transfer(&env.current_contract_address(), &user, &position.amount);
        }

        if let Some(stake) = get_user_stake(&env, &user) {
            remove_user_stake(&env, &user);
            total_returned += stake.amount;
            banked_credits += stake.credits_banked;
            token.transfer(&env.current_contract_address(), &user, &stake.amount);
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

        // Checks-effects-interactions: persist state *before* the external
        // token transfer below. `stake_token` is an admin-supplied address,
        // not necessarily a trusted Stellar Asset Contract, and its
        // `transfer` is a synchronous cross-contract call that could
        // otherwise observe (or, on a future host that permits it, mutate)
        // stake state while it's still only a local variable. If the
        // transfer fails, the whole invocation reverts and this write is
        // rolled back with it — Soroban's per-invocation atomicity, not
        // manual sequencing, is what keeps this safe on failure. See #71.
        new_stake.credit_rate = read_credit_rate(&env);
        set_user_stake(&env, &from, &new_stake);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );

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
        let amount = stake.amount;

        // Checks-effects-interactions: clear state *before* the external
        // token transfer below. `stake_token` is an admin-supplied address,
        // not necessarily a trusted Stellar Asset Contract, and its
        // `transfer` is a synchronous cross-contract call that could
        // otherwise observe (or, on a future host that permits it, mutate)
        // the not-yet-removed UserStake, allowing a reentrant double-payout.
        // Removing the record first ensures a reentrant call sees None/an
        // already-cleared UserStake and cannot obtain a second payout.
        // See #71.
        remove_user_stake(&env, &from);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &from,
            &amount,
        );

        Ok(total_credits)
    }

    pub fn set_boost(env: Env, user: Address, allocation_pct: u32) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_not_paused(&env)?;
        if !(1..=100).contains(&allocation_pct) {
            return Err(PoolError::InvalidAllocation);
        }
        require_not_paused(&env)?;

        require_initialized(&env)?;
        assert!(
            (1..=100).contains(&allocation_pct),
            "allocation_pct must be 1-100"
        );
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

    /// Set the global credit multiplier. Rejects 0 and anything above
    /// `MAX_GLOBAL_MULTIPLIER` — see #89 for the overflow-safety derivation.
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

    pub fn set_min_stake_amount(env: Env, amount: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::MinStakeAmount, &amount);

        Ok(())
    }
    /// Return the current min stake amount , or `None` if not staked.
    pub fn get_min_stake_amount(env: Env) -> Result<i128, PoolError>  {
        require_initialized(&env)?;
        let min_stake = env.storage().instance()
            .get::<DataKey, i128>(&DataKey::MinStakeAmount)
            .unwrap_or(1);

        Ok(min_stake)
    }


    /// Return the current stake record for `user`, or `None` if not staked.
    pub fn get_stake(env: Env, user: Address) -> Result<Option<UserStake>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_user_stake(&env, &user))
    }
}

mod test;
