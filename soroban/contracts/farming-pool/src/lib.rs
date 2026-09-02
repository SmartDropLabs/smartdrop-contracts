#![no_std]
#![allow(deprecated)]

#[cfg(test)]
mod mock_reentrant_token;
mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, BytesN, Env, Vec};
pub use types::PoolError;
use types::{
    BankedCreditTotals, BoostConfig, DataKey, ListWhitelistedResponse, Position, UserStake,
};

// Expose compiled WASM bytes so sibling crates (e.g. `factory`) can upload the
// real farming-pool contract in their integration tests via:
//   `env.deployer().upload_contract_wasm(farming_pool::WASM)`
// Gated behind `testutils` feature (enabled by factory's dev-dependency) so it
// is never included in on-chain release builds.
#[cfg(any(test, feature = "testutils"))]
mod wasm_import {
    // Scoped in its own module (per contractimport!'s documented pattern) so
    // its generated `WASM`/`Client` items don't collide with this crate's
    // own `#[contract]`-generated names. No sha256 pinning here — unlike
    // contractfile!, contractimport! doesn't require it, which matters
    // because this WASM is our own sibling crate's freshly-built output on
    // every CI run, not a fixed external artifact: Rust/LLVM codegen isn't
    // bit-for-bit reproducible across separate `cargo build` invocations of
    // the same source, so a hardcoded hash here would break intermittently
    // for reasons unrelated to any real content change.
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/farming_pool.wasm");
}
#[cfg(any(test, feature = "testutils"))]
pub use wasm_import::WASM;

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5s/ledger).
const USER_TTL_THRESHOLD: u32 = 518_400;
const USER_TTL_EXTEND_TO: u32 = 1_036_800;

/// Current contract schema version, written by `initialize`/`migrate`. A pool
/// deployed before `SchemaVersion` was tracked at all reads back `SCHEMA_VERSION`
/// via `read_schema_version`'s `unwrap_or`, so it's treated as already current
/// rather than needing a migration.
const SCHEMA_VERSION: u32 = 1;

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
const MAX_STAKE_AMOUNT: i128 = 10i128.pow(18);

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

fn require_staking_not_paused(env: &Env) -> Result<(), PoolError> {
    if pool_is_paused(env) || pool_is_staking_paused(env) {
        return Err(PoolError::Paused);
    }
    Ok(())
}

fn require_withdrawals_not_paused(env: &Env) -> Result<(), PoolError> {
    if pool_is_paused(env) || pool_is_withdrawals_paused(env) {
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

fn read_total_credits(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalCredits)
        .unwrap_or(0)
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

fn pool_is_staking_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::PausedStaking)
        .unwrap_or(false)
}

fn pool_is_withdrawals_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::PausedWithdrawals)
        .unwrap_or(false)
}

fn read_global_multiplier_change_ledger(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::GlobalMultiplierChangeLedger)
        .unwrap_or(0)
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

fn add_total_staked(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalStaked)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalStaked,
        &total.checked_add(amount).expect("total stake overflow"),
    );
}

fn subtract_total_staked(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalStaked)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalStaked,
        &total.checked_sub(amount).expect("total stake underflow"),
    );
}

fn add_total_locked(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalLocked)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalLocked,
        &total.checked_add(amount).expect("total locked overflow"),
    );
}

fn subtract_total_locked(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalLocked)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalLocked,
        &total.checked_sub(amount).expect("total locked underflow"),
    );
}

fn read_boost_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::BoostCount)
        .unwrap_or(0)
}

fn increment_boost_count(env: &Env) {
    let count = read_boost_count(env);
    env.storage()
        .instance()
        .set(&DataKey::BoostCount, &(count + 1));
}

fn is_user_staked(env: &Env, user: &Address) -> bool {
    get_position(env, user).is_some() || get_user_stake(env, user).is_some()
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

fn read_lock_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::LockCount)
        .unwrap_or(0)
}

fn increment_lock_count(env: &Env) {
    let count = read_lock_count(env);
    env.storage()
        .instance()
        .set(&DataKey::LockCount, &(count + 1));
}

fn read_unstake_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::UnstakeCount)
        .unwrap_or(0)
}

fn increment_unstake_count(env: &Env) {
    let count = read_unstake_count(env);
    env.storage()
        .instance()
        .set(&DataKey::UnstakeCount, &(count + 1));
}

fn read_active_stake_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::ActiveStakeCount)
        .unwrap_or(0)
}

fn increment_active_stake_count(env: &Env) {
    let count = read_active_stake_count(env);
    env.storage()
        .instance()
        .set(&DataKey::ActiveStakeCount, &(count + 1));
}

fn decrement_active_stake_count(env: &Env) {
    let count = read_active_stake_count(env);
    if count > 0 {
        env.storage()
            .instance()
            .set(&DataKey::ActiveStakeCount, &(count - 1));
    }
}

fn read_credit_rate_change_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::CreditRateChangeCount)
        .unwrap_or(0)
}

fn increment_credit_rate_change_count(env: &Env) {
    let count = read_credit_rate_change_count(env);
    env.storage()
        .instance()
        .set(&DataKey::CreditRateChangeCount, &(count + 1));
}

fn get_emergency_withdrawal_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::EmergencyWithdrawalCount)
        .unwrap_or(0)
}

fn increment_emergency_withdrawal_count(env: &Env) {
    let count = get_emergency_withdrawal_count(env);
    env.storage().instance().set(
        &DataKey::EmergencyWithdrawalCount,
        &(count.saturating_add(1)),
    );
}

fn set_banked_credits(env: &Env, user: &Address, totals: BankedCreditTotals) {
    let key = DataKey::BankedCredits(user.clone());
    env.storage().persistent().set(&key, &totals);
    bump_user(env, &key);
}

fn add_total_distributed_credits(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalDistributedCredits)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalDistributedCredits,
        &total.checked_add(amount).expect("total credits overflow"),
    );
}

fn add_total_credits(env: &Env, amount: i128) {
    let total = read_total_credits(env);
    env.storage().instance().set(
        &DataKey::TotalCredits,
        &total.checked_add(amount).expect("total credits overflow"),
    );
}

fn read_total_banked_credits(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalBankedCredits)
        .unwrap_or(0)
}

fn add_total_banked_credits(env: &Env, amount: i128) {
    let total = read_total_banked_credits(env);
    env.storage().instance().set(
        &DataKey::TotalBankedCredits,
        &total.checked_add(amount).expect("total banked credits overflow"),
    );
}

fn subtract_total_banked_credits(env: &Env, amount: i128) {
    let total = read_total_banked_credits(env);
    env.storage().instance().set(
        &DataKey::TotalBankedCredits,
        &total.checked_sub(amount).expect("total banked credits underflow"),
    );
}

fn read_total_credits_earned(env: &Env, user: &Address) -> i128 {
    let key = DataKey::TotalCreditsEarned(user.clone());
    env.storage().persistent().get(&key).unwrap_or(0)
}

fn add_total_credits_earned(env: &Env, user: &Address, amount: i128) {
    let key = DataKey::TotalCreditsEarned(user.clone());
    let total = env.storage().persistent().get::<DataKey, i128>(&key).unwrap_or(0);
    env.storage().persistent().set(
        &key,
        &total.checked_add(amount).expect("user lifetime credits overflow"),
    );
    bump_user(env, &key);
}

fn read_total_deposits(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalDeposits)
        .unwrap_or(0)
}

fn add_total_deposits(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalDeposits)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalDeposits,
        &total.checked_add(amount).expect("total deposits overflow"),
    );
}

fn read_total_withdrawals(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalWithdrawals)
        .unwrap_or(0)
}

fn add_total_withdrawals(env: &Env, amount: i128) {
    let total = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::TotalWithdrawals)
        .unwrap_or(0);
    env.storage().instance().set(
        &DataKey::TotalWithdrawals,
        &total
            .checked_add(amount)
            .expect("total withdrawals overflow"),
    );
}

fn read_total_boost_allocations(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::TotalBoostAlloc)
        .unwrap_or(0)
}

fn add_total_boost_allocation(env: &Env, delta: i64) {
    let total = read_total_boost_allocations(env);
    if delta >= 0 {
        env.storage().instance().set(
            &DataKey::TotalBoostAlloc,
            &total
                .checked_add(delta as u64)
                .expect("total boost alloc overflow"),
        );
    } else {
        let sub = (-delta) as u64;
        env.storage().instance().set(
            &DataKey::TotalBoostAlloc,
            &total.checked_sub(sub).expect("total boost alloc underflow"),
        );
    }
}

fn read_boost_user_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::BoostUserCount)
        .unwrap_or(0)
}

fn increment_boost_user_count(env: &Env) {
    let count = read_boost_user_count(env);
    env.storage()
        .instance()
        .set(&DataKey::BoostUserCount, &(count + 1));
}

// `set_boost` rejects a zero `allocation_pct` (see `test_set_boost_rejects_zero_allocation`),
// so there is currently no path that clears a user's boost back to zero. Kept as the
// symmetric counterpart to `increment_boost_user_count` for when such a path is added.
#[allow(dead_code)]
fn decrement_boost_user_count(env: &Env) {
    let count = read_boost_user_count(env);
    if count > 0 {
        env.storage()
            .instance()
            .set(&DataKey::BoostUserCount, &(count - 1));
    }
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

fn get_whitelisted_users_list(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistedUsers)
        .unwrap_or(Vec::new(env))
}

fn set_whitelisted_users_list(env: &Env, users: &Vec<Address>) {
    env.storage()
        .instance()
        .set(&DataKey::WhitelistedUsers, users);
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

fn compute_stake_accrual(env: &Env, user: &Address, stake: &UserStake, current: u32) -> i128 {
    let allocation_pct = get_user_boost(env, user).unwrap_or(0);
    let current_multiplier = read_global_multiplier(env);
    let change_ledger = read_global_multiplier_change_ledger(env);
    let elapsed_since_start = current.saturating_sub(stake.start_ledger);

    if stake.multiplier == current_multiplier || change_ledger <= stake.start_ledger {
        return compute_credits(
            stake.amount,
            allocation_pct,
            stake.multiplier,
            stake.credit_rate,
            elapsed_since_start,
        );
    }

    let pre_change_elapsed = change_ledger
        .saturating_sub(stake.start_ledger)
        .min(elapsed_since_start);
    let post_change_elapsed = current
        .saturating_sub(change_ledger)
        .min(elapsed_since_start.saturating_sub(pre_change_elapsed));

    compute_credits(
        stake.amount,
        allocation_pct,
        stake.multiplier,
        stake.credit_rate,
        pre_change_elapsed,
    ) + compute_credits(
        stake.amount,
        allocation_pct,
        current_multiplier,
        stake.credit_rate,
        post_change_elapsed,
    )
}

/// Snapshot the user's current credit accrual and adopt the latest global
/// multiplier and credit rate.
///
/// This is called internally by `stake`, `unstake`, and `set_boost` to
/// freeze the user's accrued credits under the *old* rate/multiplier before
/// switching them to the *current* values for future accrual.
///
/// # Design trade-off: rate changes between checkpoints
///
/// `credit_rate` and `global_multiplier` are global parameters that can be
/// changed by the admin at any time (via `set_credit_rate` /
/// `set_global_multiplier`). Because each user's snapshot is only updated
/// when *they* trigger a checkpoint (stake, unstake, or set_boost), users
/// who checkpoint less frequently may earn credits at a different effective
/// rate than those who checkpoint more often during a rate change window.
///
/// This is an intentional design choice: it keeps credit accrual fully
/// local to each user's storage entry (no shared counter to synchronise),
/// avoids front-running concerns around rate changes, and ensures that the
/// cost of a rate change is O(1) rather than O(n) in the number of users.
/// Integrators should be aware that a user's on-chain credit balance may
/// temporarily reflect an outdated rate until their next checkpoint.
///
/// Helper function to perform a checkpoint on a user's `UserStake`.
///
/// Computes and banks accrued credits based on the active boost configuration,
/// updates `start_ledger`, and snapshots current global multiplier / credit rate.
/// Emits `(symbol_short!("pool"), symbol_short!("chkpt"))` when accrued > 0.
fn checkpoint(env: &Env, user: &Address, stake: &mut UserStake) {
    let current = env.ledger().sequence();
    let accrued = compute_stake_accrual(env, user, stake, current);
    stake.credits_banked += accrued;
    add_total_credits(env, accrued);
    add_total_distributed_credits(env, accrued);
    if accrued > 0 {
        add_total_banked_credits(env, accrued);
        add_total_credits_earned(env, user, accrued);
    }
    stake.start_ledger = current;
    stake.credit_rate = read_credit_rate(env);
    stake.multiplier = read_global_multiplier(env);

    if accrued > 0 {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("chkpt")),
            (user.clone(), accrued, stake.credits_banked),
        );
    }
}

fn checkpoint_position(env: &Env, _user: &Address, position: &mut Position) {
    let current = env.ledger().sequence();
    let elapsed = current.saturating_sub(position.checkpoint_ledger);
    let delta = position.amount * position.credit_rate * elapsed as i128;
    position.total_credits += delta;
    add_total_credits(env, delta);
    add_total_distributed_credits(env, delta);
    if delta > 0 {
        add_total_banked_credits(env, delta);
        add_total_credits_earned(env, user, delta);
    }
    position.checkpoint_ledger = current;
    position.credit_rate = read_credit_rate(env);

    if delta > 0 {
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("chkpt")),
            (user.clone(), delta, position.total_credits),
        );
    }
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
        // Ceilings mirror `set_global_multiplier`/`set_credit_rate` — see #89.
        if !(1..=MAX_GLOBAL_MULTIPLIER).contains(&global_multiplier) {
            return Err(PoolError::InvalidGlobalMultiplier);
        }
        if credit_rate <= 0 || credit_rate > MAX_CREDIT_RATE {
            return Err(PoolError::InvalidCreditRate);
        }
        let min_stake = if min_stake_amount <= 0 {
            1i128
        } else {
            min_stake_amount
        };
        if min_stake > MAX_STAKE_AMOUNT {
            return Err(PoolError::InvalidMinStakeAmount);
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
            .set(&DataKey::MinStakeAmount, &min_stake);
        env.storage().instance().set(&DataKey::TotalStaked, &0i128);
        env.storage().instance().set(&DataKey::TotalLocked, &0i128);
        env.storage().instance().set(&DataKey::TotalCredits, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalBankedCredits, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposits, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalWithdrawals, &0i128);
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

    /// Propose an admin handoff. The proposed admin must separately call
    /// `accept_admin` and authorise the call before the handoff completes.
    ///
    /// Proposing the current admin cancels any pending handoff. A new proposal
    /// also overwrites an existing pending handoff.
    /// Emits `("pool", "adm_prop")` with `(current_admin, proposed_admin)`.
    pub fn propose_admin(env: Env, new_admin: Address) -> Result<(), PoolError> {
        let current = get_admin(&env)?;
        current.require_auth();
        bump_instance(&env);

        if new_admin == current {
            env.storage().instance().remove(&DataKey::PendingAdmin);
        } else {
            env.storage()
                .instance()
                .set(&DataKey::PendingAdmin, &new_admin);
        }
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("adm_prop")),
            (current, new_admin),
        );
        Ok(())
    }

    /// Accept the pending admin handoff. Only the proposed admin can authorise.
    pub fn accept_admin(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(PoolError::NoPendingAdmin)?;
        pending.require_auth();
        let current = get_admin(&env)?;
        bump_instance(&env);

        env.storage().instance().set(&DataKey::Admin, &pending);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("adm_xfr")),
            (current, pending),
        );
        Ok(())
    }

    /// Admin: transfer admin rights to `new_admin` in one step.
    ///
    /// Deprecated: use `propose_admin` followed by `accept_admin` so the new
    /// admin must prove control of its address before the handoff completes.
    /// Emits a `("pool", "adm_xfr")` event with `(old_admin, new_admin)`.
    #[deprecated(note = "use propose_admin followed by accept_admin")]
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), PoolError> {
        let current = get_admin(&env)?;
        current.require_auth();
        bump_instance(&env);

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
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

    /// Schema migration entry-point (currently a no-op placeholder).
    ///
    /// This function exists so that future schema version bumps can perform
    /// data migrations inside the same entry-point without changing the ABI.
    /// At present `SCHEMA_VERSION == 1` and no stored data needs
    /// transformation, so the call simply stamps the current version and
    /// returns the previous one.
    ///
    /// Admin-only. Returns the schema version *before* this call.
    ///
    /// # Behaviour once real migrations are needed
    ///
    /// When `SCHEMA_VERSION` is bumped, add a `match` over the old version
    /// that performs the necessary storage reads/writes (e.g. re-encoding a
    /// stored struct, adding a new field with a default value, etc.) **before**
    /// writing the new `SCHEMA_VERSION`. Each migration step must be idempotent
    /// and should be tested in isolation.
    pub fn migrate(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let current = read_schema_version(&env);
        let mut version = current;
        while version < SCHEMA_VERSION {
            match version {
                0 => {
                    // Initial schema tracking migration (v0 -> v1)
                    version = 1;
                }
                _ => break,
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("migrated")),
            (current, SCHEMA_VERSION),
        );
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

    /// Lock assets for the minimum lock period. A top-up checkpoints the
    /// existing position and extends its whole-position unlock ledger to the
    /// later of the existing unlock ledger and a fresh period from this call.
    pub fn lock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_staking_not_paused(&env)?;

        assert!(amount > 0, "amount must be positive");

        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &user) {
            return Err(PoolError::NotWhitelisted);
        }
        let existing_amount = get_position(&env, &user).map_or(0i128, |p| p.amount);
        let total_amount = existing_amount + amount;
        let min_stake = Self::get_min_stake_amount(env.clone())?;
        if total_amount < min_stake {
            return Err(PoolError::BelowMinimumStake);
        }

        bump_instance(&env);

        let was_staked = is_user_staked(&env, &user);
        let current = env.ledger().sequence();
        let mut position = if let Some(mut existing) = get_position(&env, &user) {
            checkpoint_position(&env, &user, &mut existing);
            existing.amount += amount;
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
        if !was_staked && is_user_staked(&env, &user) {
            increment_staked_user_count(&env);
        }
        increment_lock_count(&env);
        add_total_staked(&env, amount);
        add_total_locked(&env, amount);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &user,
            env.current_contract_address(),
            &amount,
        );

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("locked")),
            (user, amount, position.amount),
        );
        Ok(())
    }

    pub fn unlock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        require_withdrawals_not_paused(&env)?;

        assert!(amount > 0, "amount must be positive");
        bump_instance(&env);

        let was_staked = is_user_staked(&env, &user);
        let mut position = get_position(&env, &user).expect("no active position");
        assert!(amount <= position.amount, "insufficient locked balance");

        let current = env.ledger().sequence();
        assert!(
            current >= position.unlock_ledger,
            "minimum lock period not elapsed"
        );

        checkpoint_position(&env, &user, &mut position);
        let total_credits = position.total_credits;
        position.amount -= amount;

        // Checks-Effects-Interactions: persist state before token transfer (#70).
        if position.amount == 0 {
            let banked = Self::get_banked_credits_split(env.clone(), user.clone())?;
            set_banked_credits(
                &env,
                &user,
                BankedCreditTotals {
                    position_credits: banked.position_credits + total_credits,
                    stake_credits: banked.stake_credits,
                },
            );
            remove_position(&env, &user);
        } else {
            set_position(&env, &user, &position);
        }
        if was_staked && !is_user_staked(&env, &user) {
            decrement_staked_user_count(&env);
        }
        subtract_total_staked(&env, amount);
        subtract_total_locked(&env, amount);

        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &user,
            &amount,
        );

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("unlocked")),
            (user, amount, total_credits),
        );
        Ok(())
    }

    /// Calculate current accrued credits specifically for the time-locked `Position` staking system.
    ///
    /// See also `get_position_credits` (an explicit alias for this function) and `get_credits`
    /// (which calculates combined credits across both `Position` and `UserStake` systems).
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

    /// Return current accrued credits for a user's time-locked `Position`.
    ///
    /// Alias for `calculate_credits` providing explicit system-specific naming.
    pub fn get_position_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        Self::calculate_credits(env, user)
    }

    pub fn get_user_position(env: Env, user: Address) -> Result<Option<Position>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let Some(mut position) = get_position(&env, &user) else {
            return Ok(None);
        };
        let current = env.ledger().sequence();
        let elapsed = current.saturating_sub(position.checkpoint_ledger);
        position.total_credits += position.amount * position.credit_rate * elapsed as i128;
        position.checkpoint_ledger = current;
        position.credit_rate = read_credit_rate(&env);
        Ok(Some(position))
    }

    /// Lightweight check for whether `user` has an active locked position.
    ///
    /// Returns `true` if the user has a non-zero locked position, `false`
    /// otherwise. This is cheaper than `get_user_position` as it avoids
    /// computing uncommitted credit accrual.
    pub fn has_position(env: Env, user: Address) -> Result<bool, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_position(&env, &user).is_some())
    }

    pub fn pause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &true);
        env.storage().instance().set(&DataKey::PausedStaking, &true);
        env.storage()
            .instance()
            .set(&DataKey::PausedWithdrawals, &true);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("paused")), ());
        Ok(())
    }

    pub fn pause_staking(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::PausedStaking, &true);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("stg_pause")), ());
        Ok(())
    }

    pub fn pause_withdrawals(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::PausedWithdrawals, &true);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("wd_pause")), ());
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::PausedStaking, &false);
        env.storage()
            .instance()
            .set(&DataKey::PausedWithdrawals, &false);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("unpaused")), ());
        Ok(())
    }

    pub fn unpause_staking(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::PausedStaking, &false);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("stg_unps")), ());
        Ok(())
    }

    pub fn unpause_withdrawals(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::PausedWithdrawals, &false);
        env.events()
            .publish((symbol_short!("pool"), symbol_short!("wd_unps")), ());
        Ok(())
    }

    pub fn is_paused(env: Env) -> Result<bool, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(pool_is_paused(&env))
    }

    /// Withdraw staked/locked tokens during emergency when pool is paused.
    ///
    /// Allows users to withdraw their staked/locked assets during an emergency pause.
    /// Requires authorization from `user`. Checkpoints and preserves accrued credit
    /// totals in `BankedCredits`.
    ///
    /// # Usage & Operational Guidelines
    /// - **When to Use**: Called during emergency situations or protocol maintenance when
    ///   the contract has been explicitly paused via `pause()`.
    /// - **Credit Preservation**: Staked/locked assets are returned in full while accrued
    ///   credits are safely preserved in `BankedCredits` split between `position_credits`
    ///   and `stake_credits` (see `get_banked_credits_split`). Users do not forfeit earned credits.
    /// - **User Notification & Off-Chain Tracking**: Every emergency withdrawal emits an
    ///   on-chain `("pool", "emrg_exit")` event with payload `(admin, user, total_returned)`.
    ///   Indexers and frontends notify users of emergency exit transactions by monitoring this topic.
    /// - **Audit Requirements & Privilege Governance**: Because emergency mechanisms handle pool
    ///   assets during security pauses, all admin actions initiating pauses and emergency exits must
    ///   be logged to immutable audit trails and governed by multi-sig or timelock controls.
    pub fn emergency_withdraw(env: Env, user: Address) -> Result<i128, PoolError> {
        user.require_auth();
        require_initialized(&env)?;
        let admin = get_admin(&env)?;
        if !pool_is_paused(&env) {
            return Err(PoolError::NotPaused);
        }
        bump_instance(&env);

        let was_staked = is_user_staked(&env, &user);
        let mut total_returned = 0i128;
        let mut position_credits = 0i128;
        let mut stake_credits = 0i128;
        let stake_token = get_stake_token(&env)?;
        let token = token::TokenClient::new(&env, &stake_token);

        if let Some(position) = get_position(&env, &user) {
            token.transfer(&env.current_contract_address(), &user, &position.amount);
            total_returned += position.amount;
            subtract_total_staked(&env, position.amount);
            subtract_total_locked(&env, position.amount);
            position_credits = position.total_credits;
            remove_position(&env, &user);
        }

        if let Some(stake) = get_user_stake(&env, &user) {
            token.transfer(&env.current_contract_address(), &user, &stake.amount);
            total_returned += stake.amount;
            subtract_total_staked(&env, stake.amount);
            stake_credits = stake.credits_banked;
            remove_user_stake(&env, &user);
            decrement_active_stake_count(&env);
        }

        if was_staked && !is_user_staked(&env, &user) {
            decrement_staked_user_count(&env);
        }

        if total_returned == 0 {
            return Err(PoolError::NoActiveStake);
        }

        add_total_withdrawals(&env, total_returned);

        // Bank the position and stake credits as separate totals so each staking
        // system's accrual history survives even when a user held both (#145).
        if position_credits > 0 || stake_credits > 0 {
            set_banked_credits(
                &env,
                &user,
                BankedCreditTotals {
                    position_credits,
                    stake_credits,
                },
            );
        }

        increment_emergency_withdrawal_count(&env);

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("emrg_exit")),
            (admin, user, total_returned),
        );
        Ok(total_returned)
    }

    /// Total number of successful `emergency_withdraw` calls since pool
    /// initialization, for protocol risk monitoring (#257).
    pub fn emergency_withdrawal_count(env: Env) -> u32 {
        bump_instance(&env);
        get_emergency_withdrawal_count(&env)
    }

    pub fn get_banked_credits(env: Env, user: Address) -> i128 {
        bump_instance(&env);
        let totals = Self::get_banked_credits_split(env, user).unwrap_or(BankedCreditTotals {
            position_credits: 0,
            stake_credits: 0,
        });
        totals.position_credits + totals.stake_credits
    }

    /// Return the banked credits for `user`, split by staking system. The
    /// lock/unlock `position` and boost `stake` histories are kept separate so
    /// that a user who held both does not lose which credits came from where
    /// (#145). Returns zeros when `user` has no banked credits.
    pub fn get_banked_credits_split(
        env: Env,
        user: Address,
    ) -> Result<BankedCreditTotals, PoolError> {
        bump_instance(&env);
        let key = DataKey::BankedCredits(user.clone());
        let value: Option<BankedCreditTotals> = env.storage().persistent().get(&key);
        if value.is_some() {
            bump_user(&env, &key);
        }
        Ok(value.unwrap_or(BankedCreditTotals {
            position_credits: 0,
            stake_credits: 0,
        }))
    }

    // ── Whitelist system ──────────────────────────────────────────────────────

    /// Admin: enable whitelist mode. Admin must authorise.
    pub fn enable_whitelist(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::WhitelistEnabled, &true);
        Ok(())
    }

    /// Admin: disable whitelist mode. Admin must authorise.
    pub fn disable_whitelist(env: Env) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);
        env.storage()
            .instance()
            .set(&DataKey::WhitelistEnabled, &false);
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

        let mut users = get_whitelisted_users_list(&env);
        if !users.contains(&user) {
            users.push_back(user);
            set_whitelisted_users_list(&env, &users);
        }
        Ok(())
    }

    /// Admin: remove `user` from the whitelist. Admin must authorise.
    pub fn remove_from_whitelist(env: Env, user: Address) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        let key = DataKey::Whitelisted(user.clone());
        env.storage().persistent().remove(&key);

        let users = get_whitelisted_users_list(&env);
        let mut new_users: Vec<Address> = Vec::new(&env);
        for u in users.iter() {
            if u != user {
                new_users.push_back(u);
            }
        }
        set_whitelisted_users_list(&env, &new_users);
        Ok(())
    }

    /// Public: check if `user` is whitelisted. Bumps TTL of the entry if whitelisted.
    pub fn is_whitelisted(env: Env, user: Address) -> bool {
        bump_instance(&env);
        is_user_whitelisted(&env, &user)
    }

    /// Return a paginated list of all whitelisted addresses.
    ///
    /// `offset`: zero-based index of the first address to return.
    /// `limit`: maximum number of addresses to return per call.
    ///
    /// Returns a `ListWhitelistedResponse` containing the requested page and
    /// the total number of whitelisted addresses. Call repeatedly with
    /// increasing `offset` until `offset >= total` to retrieve the full list.
    pub fn get_whitelisted_users(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Result<ListWhitelistedResponse, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let all = get_whitelisted_users_list(&env);
        let total = all.len();
        let mut page: Vec<Address> = Vec::new(&env);
        let mut i = offset;
        let mut count = 0u32;
        while i < total && count < limit {
            page.push_back(all.get(i).unwrap());
            i += 1;
            count += 1;
        }

        Ok(ListWhitelistedResponse { users: page, total })
    }

    /// Admin: batch add multiple `users` to the whitelist. Capped at 50 addresses per call. Admin must authorise.
    pub fn batch_add_to_whitelist(env: Env, users: Vec<Address>) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if users.len() > 50 {
            panic!("max 50 addresses per call");
        }
        bump_instance(&env);

        let mut list = get_whitelisted_users_list(&env);
        for user in users.iter() {
            let key = DataKey::Whitelisted(user.clone());
            env.storage().persistent().set(&key, &true);
            bump_user(&env, &key);

            if !list.contains(&user) {
                list.push_back(user);
            }
        }
        set_whitelisted_users_list(&env, &list);
        Ok(())
    }

    /// Admin: batch remove multiple `users` from the whitelist. Capped at 50 addresses per call. Admin must authorise.
    ///
    /// Mirrors `batch_add_to_whitelist` so admins who need to revoke many
    /// users do not have to issue one `remove_from_whitelist` call per user,
    /// which is gas-inefficient. See #167.
    pub fn batch_remove_from_whitelist(env: Env, users: Vec<Address>) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        if users.len() > 50 {
            panic!("max 50 addresses per call");
        }
        bump_instance(&env);

        let mut list = get_whitelisted_users_list(&env);
        for user in users.iter() {
            let key = DataKey::Whitelisted(user.clone());
            env.storage().persistent().remove(&key);

            let mut new_list: Vec<Address> = Vec::new(&env);
            for u in list.iter() {
                if u != user {
                    new_list.push_back(u);
                }
            }
            list = new_list;
        }
        set_whitelisted_users_list(&env, &list);
        Ok(())
    }

    // ── Boost / Stake system ─────────────────────────────────────────────────

    /// Stake `amount` tokens. If a prior stake exists, earned credits are checkpointed first.
    ///
    /// # No minimum lock period — deliberately
    /// Unlike `lock_assets`, this flexible stake system has **no** enforced lock
    /// period and `unstake` can be called immediately. This is an intentional
    /// design property of the boost/stake system (see `docs/staking_systems.md`),
    /// not an oversight: it is a separate continuous-staking model built for
    /// flexible deposits where a lock would defeat its purpose.
    ///
    /// The "flash-staking" concern raised in #169 (stake and immediately
    /// unstake) exposes no privilege, because:
    /// - A stake is **not** a loan of leverage: the pool never owes more than
    ///   the exact staked amount, and `unstake` returns only `stake.amount`.
    /// - Credits accrue linearly over *elapsed ledgers* (see `compute_stake_accrual`),
    ///   so an immediate round-trip banks ~0 credits; there is no fixed
    ///   up-front reward to harvest.
    /// - On stake, the position is checkpointed to the current ledger and
    ///   credit rate; on unstake it is checkpointed again, so any intermediate
    ///   ledger gap is the only thing ever rewarded.
    ///
    /// Pools that require a commitment lock should use the lock/unlock
    /// `Position` system (`lock_assets`/`unlock_assets`) instead.
    pub fn stake(env: Env, from: Address, amount: i128) -> Result<(), PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_staking_not_paused(&env)?;
        assert!(amount > 0, "amount must be positive");

        if whitelist_enabled(&env) && !is_user_whitelisted(&env, &from) {
            return Err(PoolError::NotWhitelisted);
        }
        let min_stake = Self::get_min_stake_amount(env.clone())?;
        if amount < min_stake {
            return Err(PoolError::BelowMinimumStake);
        }

        bump_instance(&env);

        let is_first_stake = get_user_stake(&env, &from).is_none();
        let was_staked = is_user_staked(&env, &from);
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
                multiplier: read_global_multiplier(&env),
            }
        };

        new_stake.credit_rate = read_credit_rate(&env);

        // Checks-effects-interactions: persist state *before* the external
        // token transfer below, consistent with `lock_assets`. See #69, #217.
        set_user_stake(&env, &from, &new_stake);
        if is_first_stake {
            increment_active_stake_count(&env);
        }
        if !was_staked && is_user_staked(&env, &from) {
            increment_staked_user_count(&env);
        }
        add_total_staked(&env, amount);
        add_total_deposits(&env, amount);

        // Pull tokens from caller into the contract.
        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("staked")),
            (from, amount),
        );

        Ok(())
    }

    /// Withdraw the caller's entire flexible stake and bank the accrued credits.
    ///
    /// There is no minimum lock period (see `stake`); the caller may withdraw at
    /// any time. Because credits accrue only over elapsed ledgers (#169), an
    /// immediate stake→unstake round-trip earns no credits, so the lack of a
    /// lock does not create a flash-staking reward.
    pub fn unstake(env: Env, from: Address) -> Result<i128, PoolError> {
        from.require_auth();
        require_initialized(&env)?;
        require_withdrawals_not_paused(&env)?;
        bump_instance(&env);

        let was_staked = is_user_staked(&env, &from);
        let mut stake = get_user_stake(&env, &from).expect("no active stake");
        checkpoint(&env, &from, &mut stake);
        let total_credits = stake.credits_banked;
        if total_credits > 0 {
            subtract_total_banked_credits(&env, total_credits);
        }

        // Return staked tokens to caller.
        let stake_token = get_stake_token(&env)?;
        token::TokenClient::new(&env, &stake_token).transfer(
            &env.current_contract_address(),
            &from,
            &stake.amount,
        );

        env.events().publish(
            (symbol_short!("pool"), symbol_short!("unstaked")),
            (from.clone(), stake.amount, total_credits),
        );

        remove_user_stake(&env, &from);
        decrement_active_stake_count(&env);
        if was_staked && !is_user_staked(&env, &from) {
            decrement_staked_user_count(&env);
        }
        increment_unstake_count(&env);
        subtract_total_staked(&env, stake.amount);
        add_total_withdrawals(&env, stake.amount);
        Ok(total_credits)
    }

    pub fn set_boost(env: Env, user: Address, allocation_pct: u32) -> Result<(), PoolError> {
        require_initialized(&env)?;
        require_staking_not_paused(&env)?;
        get_admin(&env)?.require_auth();
        assert!(
            (1..=100).contains(&allocation_pct),
            "allocation_pct must be 1-100"
        );
        bump_instance(&env);

        let mut stake = get_user_stake(&env, &user).ok_or(PoolError::NoActiveStake)?;
        checkpoint(&env, &user, &mut stake);
        set_user_stake(&env, &user, &stake);

        let old_alloc: u32 = get_user_boost(&env, &user).unwrap_or(0);
        if old_alloc == 0 {
            increment_boost_user_count(&env);
            add_total_boost_allocation(&env, allocation_pct as i64);
        } else {
            let delta = allocation_pct as i64 - old_alloc as i64;
            if delta != 0 {
                add_total_boost_allocation(&env, delta);
            }
        }

        let key = DataKey::UserBoost(user.clone());
        if !env.storage().persistent().has(&key) {
            increment_boost_count(&env);
        }
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
        Ok(Some(BoostConfig {
            multiplier: read_global_multiplier(&env),
            allocation_pct: get_user_boost(&env, &user).unwrap_or(0),
        }))
    }

    pub fn total_credits(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_credits(&env))
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

        // Capture the previous value before overwriting it so the event can
        // carry both terms — off-chain indexers need the old multiplier for
        // audit trails and rollback scenarios (#250).
        let old_multiplier = read_global_multiplier(&env);

        env.storage()
            .instance()
            .set(&DataKey::GlobalMultiplier, &multiplier);
        env.storage().instance().set(
            &DataKey::GlobalMultiplierChangeLedger,
            &env.ledger().sequence(),
        );
        env.events().publish(
            (symbol_short!("boost"), symbol_short!("mult_set")),
            (old_multiplier, multiplier),
        );
        Ok(())
    }

    /// Set the credit accrual rate. Rejects non-positive values and anything
    /// above `MAX_CREDIT_RATE` — see #89 for the overflow-safety derivation.
    ///
    /// The new rate takes effect immediately for *new* checkpoints. Existing
    /// staked or locked users retain their previous rate snapshot until they
    /// interact (e.g. `stake`/`unstake` or `lock_assets`/`unlock_assets`),
    /// at which point `checkpoint` records the new rate. This is by design:
    /// iterating all on-chain user entries would be prohibitively expensive.
    /// Off-chain indexers should apply the rate from the `rate_set` event
    /// when computing credits for users who have not yet checkpointed.
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
        increment_credit_rate_change_count(&env);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("rate_set")),
            (old_rate, new_rate, env.ledger().sequence()),
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

    /// Return the minimum lock period in seconds (assuming ~5s/ledger).
    ///
    /// The raw ledger count in `min_lock_period` is an implementation detail
    /// that frontends and users cannot meaningfully display. This helper
    /// converts it to a human-readable duration so UIs can show days/hours
    /// directly. See #166.
    pub fn min_lock_period_seconds(env: Env) -> Result<u64, PoolError> {
        let ledgers = Self::min_lock_period(env)?;
        Ok((ledgers as u64) * 5)
    }

    pub fn get_min_lock_period_seconds(env: Env) -> Result<u64, PoolError> {
        Self::min_lock_period_seconds(env)
    }

    /// Return current accrued credits specifically for a user's `UserStake` (boost/continuous staking system).
    ///
    /// Includes both live accrual for the active stake and banked stake credits from previous withdrawals.
    pub fn get_stake_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let banked = Self::get_banked_credits_split(env.clone(), user.clone())?;

        let stake_credits = get_user_stake(&env, &user)
            .map(|stake| {
                stake.credits_banked
                    + compute_stake_accrual(&env, &user, &stake, env.ledger().sequence())
            })
            .unwrap_or(0);

        Ok(banked.stake_credits + stake_credits)
    }

    /// Return total combined accrued credits for `user` across all staking systems.
    ///
    /// Merges credits from time-locked `Position` staking, flexible `UserStake` boost staking,
    /// and prior `BankedCredits`. To query individual systems, see `get_position_credits`
    /// (`calculate_credits`) and `get_stake_credits`.
    pub fn get_credits(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let banked = Self::get_banked_credits_split(env.clone(), user.clone())?;

        let position_credits = get_position(&env, &user)
            .map(|position| {
                let elapsed = env
                    .ledger()
                    .sequence()
                    .saturating_sub(position.checkpoint_ledger);
                position.total_credits + position.amount * position.credit_rate * elapsed as i128
            })
            .unwrap_or(0);

        let stake_credits = get_user_stake(&env, &user)
            .map(|stake| {
                stake.credits_banked
                    + compute_stake_accrual(&env, &user, &stake, env.ledger().sequence())
            })
            .unwrap_or(0);

        Ok(banked.position_credits + banked.stake_credits + position_credits + stake_credits)
    }

    pub fn set_min_stake_amount(env: Env, amount: i128) -> Result<(), PoolError> {
        require_initialized(&env)?;
        get_admin(&env)?.require_auth();
        bump_instance(&env);

        if amount <= 0 || amount > MAX_STAKE_AMOUNT {
            return Err(PoolError::InvalidMinStakeAmount);
        }

        let old_amount = Self::get_min_stake_amount(env.clone())?;
        env.storage()
            .instance()
            .set(&DataKey::MinStakeAmount, &amount);
        env.events().publish(
            (symbol_short!("pool"), symbol_short!("minst_set")),
            (old_amount, amount),
        );

        Ok(())
    }
    /// Return the current min stake amount , or `None` if not staked.
    pub fn get_min_stake_amount(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        let min_stake = env
            .storage()
            .instance()
            .get::<DataKey, i128>(&DataKey::MinStakeAmount)
            .unwrap_or(1);

        Ok(min_stake)
    }

    /// Return the current stake record for `user`, or `None` if not staked (#234).
    ///
    /// `credits_banked` in the returned record is not a stale checkpoint: it is
    /// computed on the fly by adding accrual since `start_ledger` up to the
    /// current ledger, the same way `get_credits`/`get_stake_credits` do for
    /// this staking system. The returned `start_ledger`, `credit_rate`, and
    /// `multiplier` reflect this fresh checkpoint too, but — unlike `stake`,
    /// `unstake`, and `set_boost` — none of this is persisted; the on-chain
    /// record is left untouched by this read-only call.
    ///
    /// This total covers only the flexible/boost staking system (`UserStake`).
    /// A user who also holds a locked `Position`, or who has credits carried
    /// over from a prior `emergency_withdraw`, has additional balances not
    /// reflected here — use `get_credits` for the fully merged total across
    /// all systems.
    pub fn get_stake(env: Env, user: Address) -> Result<Option<UserStake>, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let Some(mut stake) = get_user_stake(&env, &user) else {
            return Ok(None);
        };
        let current = env.ledger().sequence();
        stake.credits_banked += compute_stake_accrual(&env, &user, &stake, current);
        stake.start_ledger = current;
        stake.credit_rate = read_credit_rate(&env);
        stake.multiplier = read_global_multiplier(&env);
        Ok(Some(stake))
    }

    /// Ledger at which `user`'s continuous-stake credits were last
    /// checkpointed, or `None` if the user has no active stake (#255).
    ///
    /// `checkpoint` resets `UserStake.start_ledger` to the current ledger on
    /// every `stake` / `unstake` / `set_boost`, so this is the origin the
    /// user's next accrual is measured from.
    pub fn last_checkpoint_ledger(env: Env, user: Address) -> Option<u32> {
        bump_instance(&env);
        get_user_stake(&env, &user).map(|stake| stake.start_ledger)
    }

    pub fn total_staked(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0))
    }

    /// Return the cumulative number of credits distributed to all users since
    /// the pool was initialized.
    ///
    /// The counter grows as credits are committed (banked) for users — at each
    /// `checkpoint`/`checkpoint_position` that occurs on stake, lock, boost,
    /// unlock, and unstake operations — rather than on every read-only
    /// accrual view. It therefore always reflects the sum a protocol-wide
    /// `get_credits` aggregation converges to as users interact, and is the
    /// companion of `total_staked` for reward-rate and inflation analytics.
    pub fn total_distributed_credits(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalDistributedCredits)
            .unwrap_or(0))
    }

    /// Return the total credits currently banked across all users.
    pub fn total_banked_credits(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_banked_credits(&env))
    }

    /// Return the cumulative credits earned by `user` across their lifetime,
    /// including amounts already withdrawn.
    pub fn total_credits_earned(env: Env, user: Address) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_credits_earned(&env, &user))
    }

    /// Return the running total of all tokens deposited into the pool.
    ///
    /// Incremented by `stake` and `lock_assets` with the amount transferred in.
    /// Tracks cumulative inflow for protocol flow analytics; compare with
    /// `total_withdrawals` to derive net flow and with `total_staked` to
    /// reconcile current TVL against historical turnover.
    pub fn total_deposits(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_deposits(&env))
    }

    /// Return the running total of all tokens withdrawn from the pool.
    ///
    /// Incremented by `unstake`, `unlock_assets`, and `emergency_withdraw`
    /// with the amount transferred out. Tracks cumulative outflow for
    /// protocol flow analytics; compare with `total_deposits` to derive
    /// net flow.
    pub fn total_withdrawals(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_withdrawals(&env))
    }

    /// Return the count of currently staked unique users in the pool.
    pub fn staked_user_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::StakedUserCount)
            .unwrap_or(0))
    }

    pub fn get_staked_user_count(env: Env) -> Result<u32, PoolError> {
        Self::staked_user_count(env)
    }

    /// Return the total number of lock operations performed on the pool.
    pub fn lock_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_lock_count(&env))
    }

    pub fn get_lock_count(env: Env) -> Result<u32, PoolError> {
        Self::lock_count(env)
    }

    /// Return the total number of unstake operations performed on the pool.
    pub fn unstake_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_unstake_count(&env))
    }

    pub fn get_unstake_count(env: Env) -> Result<u32, PoolError> {
        Self::unstake_count(env)
    }

    /// Return the total number of boost configurations configured across users (#230).
    pub fn boost_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_boost_count(&env))
    }

    pub fn get_boost_count(env: Env) -> Result<u32, PoolError> {
        Self::boost_count(env)
    }

    /// Return the total tokens locked across all position locking positions (#232).
    pub fn total_locked(env: Env) -> Result<i128, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::TotalLocked)
            .unwrap_or(0))
    }

    pub fn get_total_locked(env: Env) -> Result<i128, PoolError> {
        Self::total_locked(env)
    }

    /// Return the count of currently active stakes in the pool.
    pub fn active_stake_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_active_stake_count(&env))
    }

    pub fn get_active_stake_count(env: Env) -> Result<u32, PoolError> {
        Self::active_stake_count(env)
    }

    /// Return the total number of credit rate changes performed on the pool.
    pub fn credit_rate_change_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_credit_rate_change_count(&env))
    }

    pub fn get_credit_rate_change_count(env: Env) -> Result<u32, PoolError> {
        Self::credit_rate_change_count(env)
    }

    /// Return the number of addresses currently on the whitelist (#248).
    ///
    /// Admins use this for capacity planning without paging the full list via
    /// `get_whitelisted_users`. The value is derived from the canonical
    /// `WhitelistedUsers` list that every add / remove / batch path already
    /// maintains (and dedupes), rather than a parallel counter that could
    /// silently drift out of step with that list.
    pub fn whitelist_count(env: Env) -> Result<u32, PoolError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_whitelisted_users_list(&env).len())
    }

    pub fn get_whitelist_count(env: Env) -> Result<u32, PoolError> {
        Self::whitelist_count(env)
    }
}

mod test;
