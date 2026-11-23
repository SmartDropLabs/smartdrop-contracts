#![no_std]

mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short, vec, Address, Bytes, BytesN, Env, IntoVal, String,
    Symbol, Val, Vec,
};
use types::{
    ConfigSuggestion, DataKey, ListPoolsResponse, OptimizationKind, PoolComparison,
    PoolConfigChangeEntry, PoolConfigHistoryPage, PoolMetadata, PoolRecord, PoolSort, PoolStatus,
    RefreshPoolTtlsResponse, MAX_POOL_DESCRIPTION_LEN, MAX_POOL_NAME_LEN,
};

pub use types::{ConfigComparison, ConfigSuggestions};

pub use types::DependencyGraph;
pub use types::FactoryError;
pub use types::HealthStatus;
pub use types::PoolParams;
pub use types::{PoolMetadata, PoolMetrics, Snapshot};

// ~30 days at ~5 s/ledger; extend to ~60 days when below threshold.
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;
// Bound the number of pool IDs examined per call to stay within Soroban's 100 footprint entries limit.
// Increased from 50 to 200 to support large registries with many pools, reducing the number of
// paginated calls needed for asset-based lookups in `get_pools_by_asset`. The indexed path
// (DataKey::AssetPools) makes this safe — it only reads matching pool IDs from the index rather
// than scanning all pool IDs linearly. See #601.
const MAX_POOL_SCAN_PER_CALL: u32 = 200;

/// Ledgers per day at the network's ~5s/ledger target, used to convert
/// `create_pool`'s caller-facing `daily_rate` into the pool's native
/// per-ledger `credit_rate`. See `daily_rate_to_credit_rate`.
const LEDGERS_PER_DAY: u128 = 17_280;
// Minimum stake in the asset's smallest units. This is 0.1 token for the
// standard 7-decimal Stellar asset convention and prevents dust positions.
const MIN_STAKE_AMOUNT: i128 = 1_000_000;
// Maximum stake per position, mirroring the farming pool's own limit so a
// `min_stake_amount` the pool would reject is caught here first (issue #323).
const MAX_STAKE_AMOUNT: i128 = 10i128.pow(18);
// Minimum lock period in ledgers required to prevent flash-loan-style attacks.
const MIN_LOCK_PERIOD: u32 = 1;
// Maximum lock period in ledgers, mirroring the farming pool's own
// `MAX_LOCK_PERIOD` so an effectively permanent lock is caught before pool
// deployment (#532-#535).
// 10 years at 5s/ledger.
const MAX_MIN_LOCK_PERIOD: u32 = 63_072_000;
// Maximum global multiplier, mirroring the farming pool's own limit so an
// invalid multiplier is caught before pool deployment (#494).
const MAX_GLOBAL_MULTIPLIER: u32 = 1_000;
// Maximum credit rate per ledger, mirroring the farming pool's own limit so an
// excessive daily rate is caught before pool deployment (#495).
const MAX_CREDIT_RATE: i128 = 100_000_000;
// Maximum caller-facing daily rate, derived from the credit-rate ceiling so
// `create_pool`'s public unit is bounded directly rather than only after
// ledger conversion (#524-#527).
const MAX_DAILY_RATE: u128 = MAX_CREDIT_RATE as u128 * LEDGERS_PER_DAY;
// Maximum number of pools that `create_pools_batch` will create in a single
// call. Each entry deploys a full farming-pool WASM instance, so an unbounded
// batch is a resource-exhaustion vector against the deploying transaction
// (#528-#531).
const MAX_BATCH_SIZE: u32 = 10;
// Risk-score bounds and thresholds (#614). Scores run 0 (safest) to 100
// (riskiest) and are computed from factory-owned state only.
const RISK_SCORE_MAX: u32 = 100;
const RISK_SCORE_DEPRECATED_PENALTY: u32 = 40;
const RISK_SCORE_OUTDATED_WASM_PENALTY: u32 = 10;
const RISK_SCORE_TVL_THRESHOLD: i128 = MIN_STAKE_AMOUNT;
// Upper bound for `compare_pools`: each entry reads several storage keys, so an
// unbounded list could exhaust the host's footprint budget (#617).
const MAX_COMPARE_POOLS: u32 = 10;
// Upper bound for `set_pool_dependencies`: an unbounded dependency list would
// let a single call exhaust the footprint budget (#668).
const MAX_POOL_DEPENDENCIES: u32 = 10;

/// Convert a "credits per day" figure into the deployed pool's native
/// "credits per ledger" `credit_rate`.
///
/// `daily_rate` is kept as `create_pool`'s public unit because a per-day
/// figure is what off-chain/product code already reasons about; ledger-level
/// rates are an implementation detail of `FarmingPool`.
///
/// Any `daily_rate` smaller than `LEDGERS_PER_DAY` (17,280) would round up
/// to 1 credit per ledger under ceiling division, over-accruing significantly
/// relative to the requested rate. To prevent this economic distortion, rates
/// below `LEDGERS_PER_DAY` are rejected with `InvalidCreditRate`. Callers must
/// supply at least `LEDGERS_PER_DAY` credits per day. See #600.
///
/// Returns `InvalidCreditRate` if `daily_rate == 0`, if `daily_rate <
/// LEDGERS_PER_DAY`, or if the converted `credit_rate` exceeds `MAX_CREDIT_RATE`
/// or does not fit in `i128`.
fn daily_rate_to_credit_rate(daily_rate: u128) -> Result<i128, FactoryError> {
    if daily_rate == 0 {
        return Err(FactoryError::InvalidCreditRate);
    }
    // Reject rates that cannot be represented as a whole number of credits per
    // ledger without rounding: any daily_rate < LEDGERS_PER_DAY maps to
    // 1 credit/ledger via ceiling division, which over-accrues. See #600.
    if daily_rate < LEDGERS_PER_DAY {
        return Err(FactoryError::InvalidCreditRate);
    }
    let per_ledger = daily_rate / LEDGERS_PER_DAY;
    let credit_rate = i128::try_from(per_ledger).map_err(|_| FactoryError::InvalidCreditRate)?;
    if credit_rate > MAX_CREDIT_RATE {
        return Err(FactoryError::InvalidCreditRate);
    }
    Ok(credit_rate)
}

/// Publishes a `pool_gap` event for a pool ID below `PoolCount` whose record is
/// missing from storage (expired or never written), so callers and indexers
/// can see gaps in the registry instead of the ID being skipped silently
/// (issue #325).
fn publish_pool_gap(env: &Env, pool_id: u32) {
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("factory"), symbol_short!("pool_gap")),
        pool_id,
    );
}

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn bump_pool(env: &Env, pool_id: u32) {
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::Pool(pool_id), TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn bump_asset_pools(env: &Env, asset: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::AssetPools(asset.clone()),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_asset_pool_count(env: &Env, asset: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::AssetPoolCount(asset.clone()),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_admin_pools(env: &Env, admin: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolsByAdmin(admin.clone()),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_pool_tvl(env: &Env, pool_id: u32) {
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::PoolTvl(pool_id), TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn bump_wasm_pools(env: &Env, wasm_hash: &BytesN<32>) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolsByWasmHash(wasm_hash.clone()),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_pool_dependencies(env: &Env, pool_id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolDependencies(pool_id),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_pools_with_dependencies(env: &Env) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolsWithDependencies,
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

/// Reject any call that lands on a factory whose state was never seeded.
///
/// `initialize` is the only writer of `DataKey::Admin`, so its presence is the
/// canonical "this factory exists" marker. Every public entry point except
/// `initialize` runs this first so callers get a typed `NotInitialized` error
/// instead of a host panic (or a misleading zero/empty result from the getters).
fn require_initialized(env: &Env) -> Result<(), FactoryError> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(FactoryError::NotInitialized);
    }
    Ok(())
}

fn load_admin(env: &Env) -> Result<Address, FactoryError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)
}

/// Read the pool WASM hash.
///
/// Kept separate from `load_admin` because `Admin` and `WasmHash` are distinct
/// instance entries: a reader of one must not assume the other was proven present.
fn load_wasm_hash(env: &Env) -> Result<BytesN<32>, FactoryError> {
    env.storage()
        .instance()
        .get(&DataKey::WasmHash)
        .ok_or(FactoryError::NotInitialized)
}

/// Read the running count of successful `upgrade_pool` calls (#258).
fn read_upgrade_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::UpgradeCount)
        .unwrap_or(0)
}

/// Build a 32-byte salt from a pool ID so each pool gets a unique, reproducible address.
/// Live TVL of a single deployed pool, read straight from the pool via one
/// cross-contract call to its `total_staked` getter.
///
/// `FarmingPool::total_staked` already covers every token held for a user —
/// `lock_assets` credits both `TotalStaked` and `TotalLocked`, so
/// `total_locked` is a subset of `total_staked`, not a separate term to add.
/// Returns `PoolQueryFailed` if the pool does not answer the getter (e.g. an
/// older WASM predating it).
fn query_pool_tvl(env: &Env, pool: &Address) -> Result<i128, FactoryError> {
    let no_args: Vec<Val> = vec![env];
    match env.try_invoke_contract::<i128, soroban_sdk::Error>(
        pool,
        &Symbol::new(env, "total_staked"),
        no_args,
    ) {
        Ok(Ok(v)) => Ok(v),
        _ => Err(FactoryError::PoolQueryFailed),
    }
}

/// Re-read one pool's live TVL and fold the change into the `total_tvl`
/// accumulator: `total_tvl += live - previous_snapshot`, then store `live` as
/// the new snapshot. Emits `tvl_sync = (pool_id, old_snapshot, live)` when the
/// value moved. Returns the pool's live TVL.
fn apply_tvl_sync(env: &Env, pool_id: u32, pool: &Address) -> Result<i128, FactoryError> {
    let live = query_pool_tvl(env, pool)?;
    let previous = read_pool_tvl(env, pool_id);
    if live != previous {
        let aggregate = read_total_tvl(env)
            .saturating_add(live)
            .saturating_sub(previous);
        env.storage().instance().set(&DataKey::TotalTvl, &aggregate);
        env.storage()
            .persistent()
            .set(&DataKey::PoolTvl(pool_id), &live);
        bump_pool_tvl(env, pool_id);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("tvl_sync")),
            (pool_id, previous, live),
        );
    }
    Ok(live)
}

fn pool_salt(env: &Env, pool_id: u32) -> BytesN<32> {
    let mut bytes = [0u8; 32];
    bytes[28..].copy_from_slice(&pool_id.to_be_bytes());
    BytesN::from_array(env, &bytes)
}

fn validate_asset(env: &Env, asset: &Address) -> Result<(), FactoryError> {
    // Probe a read-only SEP-41 entry point (`symbol`) and require the address to
    // answer it. An address that is not a deployed token contract never answers,
    // so accepting that case (the previous `Err(_) => Ok(())` fallback) let a
    // plain account or unrelated contract be registered as a pool's staking
    // asset (#430).
    match env.try_invoke_contract::<Val, soroban_sdk::Error>(
        asset,
        &Symbol::new(env, "symbol"),
        vec![env],
    ) {
        Ok(Ok(_)) => {}
        _ => return Err(FactoryError::InvalidAsset),
    }
    // Validation 49: Also require the SEP-41 `balance` entry point. A contract
    // that answers `symbol` but not `balance` cannot support staking, because
    // `stake`/`lock_assets` read the caller's token balance before transferring
    // (#542).
    match env.try_invoke_contract::<i128, soroban_sdk::Error>(
        asset,
        &Symbol::new(env, "balance"),
        (env.current_contract_address(),).into_val(env),
    ) {
        Ok(Ok(_)) => Ok(()),
        _ => Err(FactoryError::InvalidAsset),
    }
}

fn sort_precedes(sort: PoolSort, left: &(u32, PoolRecord), right: &(u32, PoolRecord)) -> bool {
    let ordering = match sort {
        PoolSort::PoolId => left.0.cmp(&right.0),
        PoolSort::CreditRate => left.1.credit_rate.cmp(&right.1.credit_rate),
        PoolSort::GlobalMultiplier => left.1.global_multiplier.cmp(&right.1.global_multiplier),
        PoolSort::MinLockPeriod => left.1.min_lock_period.cmp(&right.1.min_lock_period),
    };
    ordering.is_lt() || (ordering.is_eq() && left.0 < right.0)
}

fn insert_sorted(records: &mut Vec<(u32, PoolRecord)>, record: (u32, PoolRecord), sort: PoolSort) {
    let mut insert_at: u32 = records.len();
    for (index, existing) in records.iter().enumerate() {
        if sort_precedes(sort, &record, &existing) {
            insert_at = index as u32;
            break;
        }
    }
    records.insert(insert_at, record);
}

fn read_admin_transfer_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::AdminTransferCount)
        .unwrap_or(0)
}

fn read_total_tvl(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalTvl)
        .unwrap_or(0)
}

fn read_pool_tvl(env: &Env, pool_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::PoolTvl(pool_id))
        .unwrap_or(0)
}

fn read_pool_insurance(env: &Env, pool_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::InsuranceFund(pool_id))
        .unwrap_or(0)
}

/// Deterministic 0-100 risk score derived solely from factory-owned state (#614).
///
/// Higher scores mean higher risk. Deprecation, short lock windows, high credit
/// rates, low TVL, and running an outdated WASM add points; a funded insurance
/// fund offsets them. No cross-contract calls are made, so the score cannot be
/// manipulated by a misbehaving pool and is safe to use inside read-only
/// comparison queries.
fn compute_pool_risk_score(
    env: &Env,
    pool_id: u32,
    record: &PoolRecord,
) -> Result<u32, FactoryError> {
    let mut score: u32 = 0;

    if record.deprecated {
        score += RISK_SCORE_DEPRECATED_PENALTY;
    }

    let one_day = LEDGERS_PER_DAY as u32;
    score += match record.min_lock_period {
        period if period >= 30 * one_day => 0,
        period if period >= 7 * one_day => 10,
        period if period >= one_day => 20,
        _ => 30,
    };

    if record.credit_rate > 0 {
        let rate_pct = (record.credit_rate as u128).saturating_mul(100) / (MAX_CREDIT_RATE as u128);
        if rate_pct >= 50 {
            score += 20;
        } else if rate_pct >= 10 {
            score += 10;
        }
    }

    let tvl = read_pool_tvl(env, pool_id);
    if tvl <= 0 {
        score += 20;
    } else if tvl < RISK_SCORE_TVL_THRESHOLD {
        score += 10;
    }

    let insurance = read_pool_insurance(env, pool_id);
    if insurance >= 10 * RISK_SCORE_TVL_THRESHOLD {
        score = score.saturating_sub(20);
    } else if insurance >= RISK_SCORE_TVL_THRESHOLD {
        score = score.saturating_sub(10);
    }

    if load_wasm_hash(env)? != record.wasm_hash {
        score += RISK_SCORE_OUTDATED_WASM_PENALTY;
    }

    Ok(score.min(RISK_SCORE_MAX))
}

/// Load a pool and build its `PoolComparison` row, bumping the record TTL.
/// Shared by `compare_pools` and `compare_configs`.
fn build_pool_comparison(env: &Env, pool_id: u32) -> Result<PoolComparison, FactoryError> {
    let record = env
        .storage()
        .persistent()
        .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
        .ok_or(FactoryError::PoolNotFound)?;
    bump_pool(env, pool_id);
    let risk_score = compute_pool_risk_score(env, pool_id, &record)?;

    Ok(PoolComparison {
        pool_id,
        address: record.address,
        asset: record.asset,
        daily_rate: record.daily_rate,
        credit_rate: record.credit_rate,
        global_multiplier: record.global_multiplier,
        min_lock_period: record.min_lock_period,
        tvl: read_pool_tvl(env, pool_id),
        risk_score,
        deprecated: record.deprecated,
    })
}

/// Configuration changes that clear each risk signal `compute_pool_risk_score`
/// fires for `record`. Thresholds are kept in lock-step with that function.
fn config_suggestions(
    env: &Env,
    pool_id: u32,
    record: &PoolRecord,
) -> Result<Vec<ConfigSuggestion>, FactoryError> {
    let mut out: Vec<ConfigSuggestion> = Vec::new(env);

    if record.deprecated {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::MigrateDeprecated,
            current: 1,
            suggested: 0,
        });
        // Tuning a decommissioned pool is pointless; migration is the advice.
        return Ok(out);
    }

    let lowest_risk_lock = 30 * LEDGERS_PER_DAY as u32;
    if record.min_lock_period < lowest_risk_lock {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::IncreaseMinLockPeriod,
            current: record.min_lock_period as i128,
            suggested: lowest_risk_lock as i128,
        });
    }

    // Risk fires at `credit_rate * 100 / MAX_CREDIT_RATE >= 10`, i.e. at or
    // above MAX_CREDIT_RATE / 10; suggest the largest rate below that.
    let high_rate_floor = MAX_CREDIT_RATE / 10;
    if record.credit_rate >= high_rate_floor {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::ReduceCreditRate,
            current: record.credit_rate,
            suggested: high_rate_floor - 1,
        });
    }

    let tvl = read_pool_tvl(env, pool_id);
    if tvl < RISK_SCORE_TVL_THRESHOLD {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::IncreaseTvl,
            current: tvl,
            suggested: RISK_SCORE_TVL_THRESHOLD,
        });
    }

    if load_wasm_hash(env)? != record.wasm_hash {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::UpgradeWasm,
            current: record.version as i128,
            suggested: record.version as i128 + 1,
        });
    }

    // Insurance only discounts other penalties, so it is advice only when at
    // least one penalty above is firing.
    let insurance = read_pool_insurance(env, pool_id);
    let full_discount = 10 * RISK_SCORE_TVL_THRESHOLD;
    if !out.is_empty() && insurance < full_discount {
        out.push_back(ConfigSuggestion {
            kind: OptimizationKind::IncreaseInsurance,
            current: insurance,
            suggested: full_discount,
        });
    }

    Ok(out)
}

/// Pool ID that wins a strict comparison, or `None` on a tie.
fn pick_winner(a_id: u32, a_wins: bool, b_id: u32, b_wins: bool) -> Option<u32> {
    if a_wins {
        Some(a_id)
    } else if b_wins {
        Some(b_id)
    } else {
        None
    }
}

fn increment_admin_transfer_count(env: &Env) {
    let count = read_admin_transfer_count(env);
    env.storage()
        .instance()
        .set(&DataKey::AdminTransferCount, &(count + 1));
}

fn validate_factory_admin(env: &Env, admin: &Address) -> Result<(), FactoryError> {
    let zero_admin = Address::from_string(&String::from_str(
        env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    if admin == &zero_admin {
        return Err(FactoryError::InvalidAdmin);
    }
    Ok(())
}

fn read_pool_config_history_count(env: &Env, pool_id: u32) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::PoolConfigHistoryCount(pool_id))
        .unwrap_or(0)
}

fn record_config_change(
    env: &Env,
    pool_id: u32,
    action: Symbol,
    old_config: Option<PoolRecord>,
    new_config: Option<PoolRecord>,
) {
    let count = read_pool_config_history_count(env, pool_id);
    let ledger = env.ledger().sequence();
    let entry = PoolConfigChangeEntry {
        pool_id,
        action,
        old_config,
        new_config,
        ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::PoolConfigHistory(pool_id, count), &entry);
    env.storage()
        .persistent()
        .set(&DataKey::PoolConfigHistoryCount(pool_id), &(count + 1));
}

fn create_pool_inner(
    env: Env,
    admin: Address,
    asset: Address,
    daily_rate: u128,
    global_multiplier: u32,
    min_lock_period: u64,
    min_stake_amount: i128,
) -> Result<u32, FactoryError> {
    bump_instance(&env);

    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::PoolCreationPaused)
        .unwrap_or(false);
    if paused {
        return Err(FactoryError::PoolCreationPaused);
    }

    validate_asset(&env, &asset)?;

    // Validation 23: Ensure asset address is not zero/null
    let zero_addr = Address::from_string(&String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    if asset == zero_addr {
        return Err(FactoryError::InvalidAsset);
    }

    // Validation 46: Ensure the staking asset is not the factory contract
    // itself. A self-referential asset would make the pool's staking token
    // the factory, which is never a valid SEP-41 stake asset (#536-#539).
    if asset == env.current_contract_address() {
        return Err(FactoryError::InvalidAsset);
    }

    // Validation 48: Ensure the staking asset is not also the pool admin. A
    // pool administered by its own staking token cannot authorize admin
    // actions (pause, upgrade, multiplier changes), leaving the pool
    // unmanageable (#541).
    if asset == admin {
        return Err(FactoryError::InvalidAsset);
    }

    // Validation 24: Ensure global_multiplier is strictly within bounds
    // Must be at least 1 and at most MAX_GLOBAL_MULTIPLIER (1000)
    if global_multiplier < 1 {
        return Err(FactoryError::InvalidGlobalMultiplier);
    }
    if global_multiplier > MAX_GLOBAL_MULTIPLIER {
        return Err(FactoryError::InvalidGlobalMultiplier);
    }

    // Validation 31: Bound the caller-facing daily_rate directly so an
    // oversized rate is rejected at the API boundary before ledger conversion.
    if daily_rate > MAX_DAILY_RATE {
        return Err(FactoryError::InvalidCreditRate);
    }

    // Validation 25: Convert and validate daily_rate with strict bounds checking
    let credit_rate = daily_rate_to_credit_rate(daily_rate)?;

    // Validation 26: Ensure daily_rate doesn't cause overflow when converted
    // and that the resulting credit_rate doesn't exceed protocol limits
    if credit_rate <= 0 || credit_rate > MAX_CREDIT_RATE {
        return Err(FactoryError::InvalidCreditRate);
    }

    // Validation 45: Reject a zero min_lock_period before the u32 conversion
    // so a lockless pool is refused at the boundary with a typed error (#536-#539).
    if min_lock_period == 0 {
        return Err(FactoryError::MinLockPeriodTooShort);
    }
    let min_lock_period: u32 = min_lock_period
        .try_into()
        .map_err(|_| FactoryError::MinLockPeriodOutOfRange)?;
    if min_lock_period < MIN_LOCK_PERIOD {
        return Err(FactoryError::MinLockPeriodTooShort);
    }
    if min_lock_period > MAX_MIN_LOCK_PERIOD {
        return Err(FactoryError::MinLockPeriodAboveCeiling);
    }
    // Validation 44: Reject an oversized raw min_stake_amount before the
    // non-positive defaulting logic, so a clearly out-of-range request is
    // refused at the boundary (#536-#539).
    if min_stake_amount > MAX_STAKE_AMOUNT {
        return Err(FactoryError::InvalidMinStakeAmount);
    }
    // Validation 47: Reject an undersized raw min_stake_amount (magnitude
    // beyond MAX_STAKE_AMOUNT on the negative side) before the non-positive
    // defaulting logic. A value like `i128::MIN` is corrupt input, not a
    // request to use the dust default (#540).
    if min_stake_amount < -MAX_STAKE_AMOUNT {
        return Err(FactoryError::InvalidMinStakeAmount);
    }
    let effective_min_stake = if min_stake_amount <= 0 {
        MIN_STAKE_AMOUNT
    } else {
        min_stake_amount
    };
    // The requested amount must lie within [MIN_STAKE_AMOUNT,
    // MAX_STAKE_AMOUNT] so the pool is usable.
    if !(MIN_STAKE_AMOUNT..=MAX_STAKE_AMOUNT).contains(&effective_min_stake) {
        return Err(FactoryError::InvalidMinStakeAmount);
    }

    let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
    let next_count = pool_id
        .checked_add(1)
        .ok_or(FactoryError::PoolCountOverflow)?;
    env.storage()
        .instance()
        .set(&DataKey::PoolCount, &next_count);
    let wasm_hash = load_wasm_hash(&env)?;
    let salt = pool_salt(&env, pool_id);

    // Deploy a fresh farming-pool instance. The resulting address is
    // deterministic: keccak256(factory_address || salt).
    let pool_address = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash.clone(), ());

    // Call the freshly deployed pool's `initialize` directly via
    // `invoke_contract` rather than depending on the `farming-pool`
    // crate's generated Client: pulling that crate in as a normal
    // dependency causes its own `#[contractimpl]`-exported WASM symbols
    // (e.g. `admin`, `transfer_admin`) to collide with the factory's own
    // exports of the same names when both are linked into one cdylib.
    let init_args: Vec<Val> = vec![
        &env,
        admin.into_val(&env),
        asset.into_val(&env),
        global_multiplier.into_val(&env),
        credit_rate.into_val(&env),
        min_lock_period.into_val(&env),
        effective_min_stake.into_val(&env),
    ];
    let _: () = env.invoke_contract(&pool_address, &Symbol::new(&env, "initialize"), init_args);

    let record = PoolRecord {
        address: pool_address.clone(),
        asset: asset.clone(),
        credit_rate,
        global_multiplier,
        min_lock_period,
        daily_rate,
        wasm_hash: wasm_hash.clone(),
        deprecated: false,
        version: 0,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Pool(pool_id), &record);
    bump_pool(&env, pool_id);

    // Record pool creation in configuration history (#680)
    record_config_change(
        &env,
        pool_id,
        symbol_short!("created"),
        None,
        Some(record.clone()),
    );

    // A freshly deployed pool holds nothing, so its contribution to
    // `total_tvl` starts at 0. Recording the baseline explicitly keeps the
    // first `sync_pool_tvl` a pure delta against a known value (#249).
    env.storage()
        .persistent()
        .set(&DataKey::PoolTvl(pool_id), &0i128);
    bump_pool_tvl(&env, pool_id);

    let asset_key = DataKey::AssetPools(asset.clone());
    let mut asset_pool_ids: Vec<u32> = env
        .storage()
        .persistent()
        .get(&asset_key)
        .unwrap_or_else(|| vec![&env]);
    asset_pool_ids.push_back(pool_id);
    env.storage().persistent().set(&asset_key, &asset_pool_ids);
    bump_asset_pools(&env, &asset);

    let asset_pool_count_key = DataKey::AssetPoolCount(asset.clone());
    let asset_pool_count: u32 = env
        .storage()
        .persistent()
        .get(&asset_pool_count_key)
        // Lazily initialize the counter for pools created before this index
        // existed, preserving correct counts after a contract upgrade.
        .unwrap_or(asset_pool_ids.len().saturating_sub(1))
        .checked_add(1)
        .ok_or(FactoryError::PoolCountOverflow)?;
    env.storage()
        .persistent()
        .set(&asset_pool_count_key, &asset_pool_count);
    bump_asset_pool_count(&env, &asset);

    let admin_key = DataKey::PoolsByAdmin(admin.clone());
    let mut admin_pool_ids: Vec<u32> = env
        .storage()
        .persistent()
        .get(&admin_key)
        .unwrap_or_else(|| vec![&env]);
    admin_pool_ids.push_back(pool_id);
    env.storage().persistent().set(&admin_key, &admin_pool_ids);
    bump_admin_pools(&env, &admin);

    // Emit enriched event so indexers get the full pool parameters in one shot.
    // `effective_min_stake` is the resolved dust-thresholded value actually
    // passed to the pool's `initialize` (#330), not the raw caller input.
    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("factory"), symbol_short!("pool_crtd")),
        (
            pool_id,
            pool_address,
            admin,
            asset,
            credit_rate,
            global_multiplier,
            min_lock_period,
            daily_rate,
            wasm_hash,
            effective_min_stake,
        ),
    );

    Ok(pool_id)
}

#[contract]
pub struct Factory;

#[contractimpl]
impl Factory {
    /// Initialize the factory. Returns `AlreadyInitialized` if called more than once.
    ///
    /// Deliberately exempt from `require_initialized`: this is the function that
    /// establishes the initialised state, and it already enforces the inverse
    /// precondition via the `has(&DataKey::Admin)` check below.
    pub fn initialize(
        env: Env,
        admin: Address,
        pool_wasm_hash: BytesN<32>,
    ) -> Result<(), FactoryError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(FactoryError::AlreadyInitialized);
        }

        // Issue #374: Validate admin address before storing
        let zero_admin = Address::from_string(&soroban_sdk::String::from_str(
            &env,
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        ));
        if admin == zero_admin {
            return Err(FactoryError::InvalidAdmin);
        }

        if pool_wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(FactoryError::InvalidWasmHash);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::WasmHash, &pool_wasm_hash);
        env.storage()
            .persistent()
            .set(&DataKey::ApprovedWasmHash(pool_wasm_hash), &true);
        // Keep the initial approval alive for as long as the factory is used.
        let approved_hash = load_wasm_hash(&env)?;
        env.storage().persistent().extend_ttl(
            &DataKey::ApprovedWasmHash(approved_hash),
            TTL_THRESHOLD,
            TTL_EXTEND_TO,
        );
        env.storage().instance().set(&DataKey::PoolCount, &0u32);
        bump_instance(&env);
        Ok(())
    }

    /// Return the current admin address.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn admin(env: Env) -> Result<Address, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        load_admin(&env)
    }

    /// Return the WASM hash of the pool implementation this factory deploys.
    ///
    /// Clients can call this to verify which farming-pool build is active before
    /// trusting a pool address returned by `create_pool`.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn pool_wasm_hash(env: Env) -> Result<BytesN<32>, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        load_wasm_hash(&env)
    }

    /// Return the total number of pools registered by this factory.
    ///
    /// Guarded even though the underlying read defaults to 0: an uninitialized
    /// factory has no registry at all, and reporting `0` would be indistinguishable
    /// from an initialized factory that has yet to create a pool.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn pool_count(env: Env) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0))
    }

    /// Return whether a pool record exists for `pool_id` without loading it.
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn pool_exists(env: Env, pool_id: u32) -> Result<bool, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let exists = env.storage().persistent().has(&DataKey::Pool(pool_id));
        if exists {
            bump_pool(&env, pool_id);
        }
        Ok(exists)
    }

    /// Return the `PoolRecord` for `pool_id`.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized, or
    /// `PoolNotFound` if `pool_id` has not been created yet.
    ///
    /// # TTL keep-alive
    /// On success, extends the persistent TTL of the requested pool record (and
    /// the factory instance) when remaining TTL falls below `TTL_THRESHOLD`.
    /// Pools that are never individually queried should be kept alive via
    /// paginated `list_pools` reads, asset-range queries, or the permissionless
    /// `refresh_pool_ttls` function.
    pub fn get_pool(env: Env, pool_id: u32) -> Result<PoolRecord, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let key = DataKey::Pool(pool_id);
        match env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
            Some(r) => {
                bump_pool(&env, pool_id);
                Ok(r)
            }
            None => Err(FactoryError::PoolNotFound),
        }
    }

    /// Return a deterministic 0-100 risk score for a registered pool (#614).
    ///
    /// `0` means no risk signals were detected; `100` means every signal fired.
    /// The score is derived entirely from factory-owned state, so it is safe to
    /// call from other read-only queries and cannot be gamed by a pool contract.
    ///
    /// Returns `NotInitialized` before `initialize`, or `PoolNotFound` for an
    /// unknown `pool_id`. Extends the pool record TTL on success, like `get_pool`.
    pub fn get_pool_risk(env: Env, pool_id: u32) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let key = DataKey::Pool(pool_id);
        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&key)
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);
        compute_pool_risk_score(&env, pool_id, &record)
    }

    /// Compare up to `MAX_COMPARE_POOLS` registered pools side by side (#617).
    ///
    /// Results follow the exact order of `pool_ids`; no sorting is applied so
    /// callers can align the response with their own UI. Each entry includes
    /// the pool's `risk_score` from `get_pool_risk`, letting dashboards rank
    /// pools without one call per pool. Duplicate IDs are rejected so a single
    /// pool cannot skew comparisons.
    ///
    /// Returns `NotInitialized` before `initialize`, `EmptyPoolList` for an
    /// empty request, `TooManyPools` above the cap, `DuplicatePoolId` for
    /// repeated IDs, and `PoolNotFound` for any unknown ID.
    pub fn compare_pools(
        env: Env,
        pool_ids: Vec<u32>,
    ) -> Result<Vec<PoolComparison>, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        if pool_ids.is_empty() {
            return Err(FactoryError::EmptyPoolList);
        }
        if pool_ids.len() > MAX_COMPARE_POOLS {
            return Err(FactoryError::TooManyPools);
        }

        let mut seen: Vec<u32> = Vec::new(&env);
        let mut comparisons: Vec<PoolComparison> = Vec::new(&env);
        for pool_id in pool_ids.iter() {
            if seen.contains(pool_id) {
                return Err(FactoryError::DuplicatePoolId);
            }
            seen.push_back(pool_id);
            comparisons.push_back(build_pool_comparison(&env, pool_id)?);
        }

        Ok(comparisons)
    }

    /// Compare two pool configurations head to head.
    ///
    /// Returns both pools' `PoolComparison` rows plus signed `a - b` deltas
    /// for every tunable field, whether they stake the same asset, and which
    /// pool wins on yield (`credit_rate * global_multiplier`) and on risk.
    /// Use `compare_pools` for more than two pools without the deltas.
    ///
    /// Returns `NotInitialized` before `initialize`, `DuplicatePoolId` when
    /// `a == b`, and `PoolNotFound` for an unknown ID.
    pub fn compare_configs(env: Env, a: u32, b: u32) -> Result<ConfigComparison, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        if a == b {
            return Err(FactoryError::DuplicatePoolId);
        }

        let pa = build_pool_comparison(&env, a)?;
        let pb = build_pool_comparison(&env, b)?;

        // Both factors are bounded (MAX_CREDIT_RATE, MAX_GLOBAL_MULTIPLIER),
        // so the products cannot overflow i128.
        let yield_a = pa.credit_rate * pa.global_multiplier as i128;
        let yield_b = pb.credit_rate * pb.global_multiplier as i128;

        Ok(ConfigComparison {
            // daily_rate is capped at MAX_DAILY_RATE, which fits in i128.
            daily_rate_diff: pa.daily_rate as i128 - pb.daily_rate as i128,
            credit_rate_diff: pa.credit_rate - pb.credit_rate,
            global_multiplier_diff: pa.global_multiplier as i64 - pb.global_multiplier as i64,
            min_lock_period_diff: pa.min_lock_period as i64 - pb.min_lock_period as i64,
            tvl_diff: pa.tvl.saturating_sub(pb.tvl),
            risk_score_diff: pa.risk_score as i64 - pb.risk_score as i64,
            same_asset: pa.asset == pb.asset,
            higher_yield: pick_winner(a, yield_a > yield_b, b, yield_b > yield_a),
            lower_risk: pick_winner(
                a,
                pa.risk_score < pb.risk_score,
                b,
                pb.risk_score < pa.risk_score,
            ),
            a: pa,
            b: pb,
        })
    }

    /// Suggest configuration changes that would lower a pool's risk score.
    ///
    /// Each suggestion corresponds to one signal in `get_pool_risk` and gives
    /// the field's current value and the value that clears that signal. A
    /// deprecated pool gets a single `MigrateDeprecated` suggestion. An empty
    /// list means no risk signal is firing. Read-only: nothing is applied.
    ///
    /// Returns `NotInitialized` before `initialize`, or `PoolNotFound` for an
    /// unknown `pool_id`. Extends the pool record TTL on success.
    pub fn get_config_optimization(
        env: Env,
        pool_id: u32,
    ) -> Result<ConfigSuggestions, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        Ok(ConfigSuggestions {
            pool_id,
            risk_score: compute_pool_risk_score(&env, pool_id, &record)?,
            suggestions: config_suggestions(&env, pool_id, &record)?,
        })
    }

    /// Set the pool IDs that `pool_id` depends on (#668). Admin-only.
    ///
    /// Each entry in `deps` must be an existing, distinct pool ID other than
    /// `pool_id` itself. Calling this again for the same `pool_id` replaces
    /// its previously stored dependency list; passing an empty `deps`
    /// clears it. Query the resulting graph via `get_dependency_graph` or a
    /// single pool's edges via `get_pool_dependencies`.
    ///
    /// Returns `NotInitialized` before `initialize`, `PoolNotFound` if
    /// `pool_id` or any dependency does not exist, `SelfDependency` if
    /// `pool_id` appears in `deps`, and `TooManyDependencies` above
    /// `MAX_POOL_DEPENDENCIES`.
    pub fn set_pool_dependencies(
        env: Env,
        pool_id: u32,
        deps: Vec<u32>,
    ) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        if !env.storage().persistent().has(&DataKey::Pool(pool_id)) {
            return Err(FactoryError::PoolNotFound);
        }
        if deps.len() > MAX_POOL_DEPENDENCIES {
            return Err(FactoryError::TooManyDependencies);
        }
        for dep in deps.iter() {
            if dep == pool_id {
                return Err(FactoryError::SelfDependency);
            }
            if !env.storage().persistent().has(&DataKey::Pool(dep)) {
                return Err(FactoryError::PoolNotFound);
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::PoolDependencies(pool_id), &deps);
        bump_pool_dependencies(&env, pool_id);

        let mut pools_with_deps: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::PoolsWithDependencies)
            .unwrap_or_else(|| vec![&env]);
        if !pools_with_deps.contains(pool_id) {
            pools_with_deps.push_back(pool_id);
            env.storage()
                .persistent()
                .set(&DataKey::PoolsWithDependencies, &pools_with_deps);
        }
        bump_pools_with_dependencies(&env);

        Ok(())
    }

    /// Return the pool IDs that `pool_id` depends on, as set by
    /// `set_pool_dependencies` (#668). Returns an empty list if none have
    /// been set. Returns `PoolNotFound` if `pool_id` does not exist.
    pub fn get_pool_dependencies(env: Env, pool_id: u32) -> Result<Vec<u32>, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        if !env.storage().persistent().has(&DataKey::Pool(pool_id)) {
            return Err(FactoryError::PoolNotFound);
        }

        let deps = env
            .storage()
            .persistent()
            .get(&DataKey::PoolDependencies(pool_id))
            .unwrap_or_else(|| vec![&env]);
        bump_pool_dependencies(&env, pool_id);
        Ok(deps)
    }

    /// Attach human-readable metadata to an existing pool (#641).
    ///
    /// `name` must be 1–64 bytes; `description` may be empty but must not
    /// exceed 256 bytes.  Returns `InvalidMetadata` if either constraint is
    /// violated, `PoolNotFound` if the pool does not exist.  Admin auth is
    /// required.
    pub fn set_pool_metadata(
        env: Env,
        pool_id: u32,
        name: Bytes,
        description: Bytes,
    ) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        // Reject the call before any storage touch if the pool does not exist.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Pool(pool_id))
        {
            return Err(FactoryError::PoolNotFound);
        }

        if name.len() == 0 || name.len() > MAX_POOL_NAME_LEN {
            return Err(FactoryError::InvalidMetadata);
        }
        if description.len() > MAX_POOL_DESCRIPTION_LEN {
            return Err(FactoryError::InvalidMetadata);
        }

        let metadata = PoolMetadata { name, description };
        env.storage()
            .persistent()
            .set(&DataKey::PoolMetadata(pool_id), &metadata);

        Ok(())
    }

    /// Return the metadata attached to `pool_id` via `set_pool_metadata` (#641).
    ///
    /// Returns `None` when no metadata has been set, `PoolNotFound` when the
    /// pool does not exist.
    pub fn get_pool_metadata(
        env: Env,
        pool_id: u32,
    ) -> Result<Option<PoolMetadata>, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        if !env
            .storage()
            .persistent()
            .has(&DataKey::Pool(pool_id))
        {
            return Err(FactoryError::PoolNotFound);
        }

        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::PoolMetadata(pool_id)))
    }

    /// Return the full pool dependency graph (#668).
    ///
    /// Includes one edge per pool that has had dependencies set via
    /// `set_pool_dependencies`; pools with none configured are omitted.
    /// Lets dashboards and off-chain tooling visualize how pools compose
    /// without walking every pool ID individually.
    pub fn get_dependency_graph(env: Env) -> Result<DependencyGraph, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let pools_with_deps: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::PoolsWithDependencies)
            .unwrap_or_else(|| vec![&env]);

        let mut edges: Vec<(u32, Vec<u32>)> = Vec::new(&env);
        for pool_id in pools_with_deps.iter() {
            let deps = env
                .storage()
                .persistent()
                .get(&DataKey::PoolDependencies(pool_id))
                .unwrap_or_else(|| vec![&env]);
            bump_pool_dependencies(&env, pool_id);
            edges.push_back((pool_id, deps));
        }
        bump_pools_with_dependencies(&env);

        Ok(DependencyGraph { edges })
    }

    /// Return a page of pool records in ascending pool ID order.
    ///
    /// `limit` is capped at 20 records so callers can page through large
    /// registries without unbounded contract work.
    ///
    /// Guarded like `pool_count`: an empty page from an uninitialized factory
    /// would be indistinguishable from an initialized but empty registry.
    ///
    /// # TTL keep-alive
    /// Extends the persistent TTL of every pool record returned in this page
    /// (plus the factory instance). Unlike `get_pool`, which bumps one record
    /// per call, each paginated read refreshes all pools in the window. Indexers
    /// that page through the registry therefore keep listed pools alive more
    /// aggressively than pools accessed only via `get_pool(id)`. For deliberate
    /// full-registry maintenance independent of read patterns, use
    /// `refresh_pool_ttls`.
    ///
    /// A pool ID below the registry count whose record is missing from storage
    /// is left out of `records` and reported with a `("factory", "pool_gap")`
    /// event carrying the ID, so gaps are visible to callers and indexers.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn list_pools(
        env: Env,
        start_id: u32,
        limit: u32,
    ) -> Result<ListPoolsResponse, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        if start_id >= count {
            return Ok(ListPoolsResponse {
                records: vec![&env],
                next_start_id: count,
                total: count,
                has_more: false,
            });
        }
        let capped_limit = if limit == 0 { 20 } else { limit.min(20) };
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];

        for pool_id in start_id..end {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                bump_pool(&env, pool_id);
                records.push_back((pool_id, record));
            } else {
                publish_pool_gap(&env, pool_id);
            }
        }

        let has_more = end < count;
        Ok(ListPoolsResponse {
            records,
            next_start_id: if end < count { end } else { count },
            total: count,
            has_more,
        })
    }

    /// Return a page of pool records sorted by a supported stored field.
    ///
    /// `start_id` and `limit` retain the same bounded ID-window semantics as
    /// `list_pools`; sorting is applied to the records found in that window.
    /// Use `PoolSort::PoolId` for behavior equivalent to `list_pools`.
    /// Records with equal sort values are ordered by ascending pool ID.
    /// Creation time, TVL, and asset ordering are not available from the
    /// factory's current on-chain record and should be supplied by an indexer.
    pub fn list_pools_sorted(
        env: Env,
        start_id: u32,
        limit: u32,
        sort: PoolSort,
    ) -> Result<ListPoolsResponse, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        if start_id >= count {
            return Ok(ListPoolsResponse {
                records: vec![&env],
                next_start_id: count,
                total: count,
                has_more: false,
            });
        }
        let capped_limit = if limit == 0 { 20 } else { limit.min(20) };
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];

        for pool_id in start_id..end {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                bump_pool(&env, pool_id);
                insert_sorted(&mut records, (pool_id, record), sort);
            } else {
                publish_pool_gap(&env, pool_id);
            }
        }

        let has_more = end < count;
        Ok(ListPoolsResponse {
            records,
            next_start_id: if end < count { end } else { count },
            total: count,
            has_more,
        })
    }

    /// Return a page of pool records whose staking asset matches `asset`, scanning up to `scan_limit` pool IDs.
    ///
    /// Scans at most `scan_limit` (capped at `MAX_POOL_SCAN_PER_CALL` = 200) pool IDs starting from
    /// `start_id` and collects matching records from that bounded window. `limit` is capped at 20
    /// records so callers can page through large registries without unbounded contract work.
    ///
    /// # Resource Limit & Gas Economics
    /// Without a scan bound, asset-matching queries perform an unbounded O(n) walk across the registry,
    /// inspecting every pool ID until `limit` matches are found or the registry is exhausted. As
    /// `pool_count` scales to thousands of pools, sparse queries would exceed Soroban's per-transaction
    /// footprint limits (100 entries) and CPU instruction budget. Bounding each call to at most 200 IDs
    /// guarantees bounded predictable gas and CPU consumption per call.
    ///
    /// # Caller Range Scanning & Pagination
    /// Callers can specify `scan_limit` (up to 200) to tune the scan window. Callers resume pagination
    /// using `next_start_id` until `next_start_id == total`.
    ///
    /// # Asset Index
    /// `create_pool` maintains a secondary on-chain index, `DataKey::AssetPools(asset) -> Vec<u32>`
    /// (with `DataKey::AssetPoolCount(asset)` as its constant-time companion), so this lookup reads
    /// only the pool IDs registered for `asset` instead of walking the whole registry. The bounded
    /// registry scan below is only a fallback for records that predate the index (#397). Off-chain
    /// applications that need lookups across thousands of pools with no transaction at all should
    /// still index the `(symbol_short!("factory"), symbol_short!("pool_crtd"))` events emitted by
    /// `create_pool`, which include `asset` and `pool_id` in their payload.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn get_pools_by_asset_range(
        env: Env,
        asset: Address,
        start_id: u32,
        scan_limit: u32,
        limit: u32,
    ) -> Result<ListPoolsResponse, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        if start_id >= count {
            return Ok(ListPoolsResponse {
                records: vec![&env],
                next_start_id: count,
                total: count,
                has_more: false,
            });
        }
        let capped_limit = if limit == 0 { 20 } else { limit.min(20) };
        let effective_scan = if scan_limit == 0 {
            MAX_POOL_SCAN_PER_CALL
        } else {
            scan_limit.min(MAX_POOL_SCAN_PER_CALL)
        };
        let scan_end = start_id.saturating_add(effective_scan).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];
        let mut next_start_id = scan_end;

        let asset_key = DataKey::AssetPools(asset.clone());
        if let Some(asset_ids) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<u32>>(&asset_key)
        {
            bump_asset_pools(&env, &asset);
            for pool_id in asset_ids.iter() {
                if pool_id < start_id {
                    continue;
                }
                if pool_id >= scan_end {
                    next_start_id = scan_end;
                    break;
                }
                // (#327) The limit check runs *before* pushing, so `pool_id`
                // is the first unreturned match: resuming from it includes it
                // in the next page instead of skipping it.
                if records.len() >= capped_limit {
                    next_start_id = pool_id;
                    break;
                }
                let key = DataKey::Pool(pool_id);
                if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                    bump_pool(&env, pool_id);
                    records.push_back((pool_id, record));
                }
            }
            let has_more = next_start_id < count;
            return Ok(ListPoolsResponse {
                records,
                next_start_id,
                total: count,
                has_more,
            });
        }

        for pool_id in start_id..scan_end {
            // (#327) Same invariant as the indexed path above: resume at the
            // first unprocessed ID so a match sitting on the limit boundary
            // is returned by the next page rather than skipped.
            if records.len() >= capped_limit {
                next_start_id = pool_id;
                break;
            }
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                if record.asset == asset {
                    bump_pool(&env, pool_id);
                    records.push_back((pool_id, record));
                }
            }
        }

        let has_more = next_start_id < count;
        Ok(ListPoolsResponse {
            records,
            next_start_id,
            total: count,
            has_more,
        })
    }

    /// Return a page of pool records whose staking asset matches `asset`.
    ///
    /// Scans at most `MAX_POOL_SCAN_PER_CALL` (200) pool IDs starting from `start_id`
    /// and collects matching records from that bounded window. Equivalent to calling
    /// `get_pools_by_asset_range(env, asset, start_id, MAX_POOL_SCAN_PER_CALL, limit)`.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn get_pools_by_asset(
        env: Env,
        asset: Address,
        start_id: u32,
        limit: u32,
    ) -> Result<ListPoolsResponse, FactoryError> {
        Self::get_pools_by_asset_range(env, asset, start_id, MAX_POOL_SCAN_PER_CALL, limit)
    }

    /// Return the number of pools registered for `asset` in constant time.
    ///
    /// Older factory deployments may not yet have a stored count; for those,
    /// the existing asset index is used once to seed the counter.
    pub fn pool_count_by_asset(env: Env, asset: Address) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count_key = DataKey::AssetPoolCount(asset.clone());
        if let Some(count) = env.storage().persistent().get::<DataKey, u32>(&count_key) {
            bump_asset_pool_count(&env, &asset);
            return Ok(count);
        }

        let asset_pool_ids: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::AssetPools(asset.clone()))
            .unwrap_or_else(|| vec![&env]);
        let count = asset_pool_ids.len();
        if count > 0 {
            env.storage().persistent().set(&count_key, &count);
            bump_asset_pool_count(&env, &asset);
        }
        Ok(count)
    }

    /// Return the list of pool IDs created by `admin`.
    pub fn get_pools_by_admin(env: Env, admin: Address) -> Result<Vec<u32>, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let admin_key = DataKey::PoolsByAdmin(admin.clone());
        if env.storage().persistent().has(&admin_key) {
            bump_admin_pools(&env, &admin);
        }
        Ok(env
            .storage()
            .persistent()
            .get(&admin_key)
            .unwrap_or_else(|| vec![&env]))
    }

    /// Return the number of pools created by `admin` (#236).
    ///
    /// Equivalent to `get_pools_by_admin(admin).len()`, exposed directly so
    /// callers who only need the count avoid paying for the full ID list.
    pub fn get_admin_pool_count(env: Env, admin: Address) -> Result<u32, FactoryError> {
        Ok(Self::get_pools_by_admin(env, admin)?.len())
    }

    /// Refresh TTLs for a range of pool records to prevent archival.
    ///
    /// This permissionless function allows keepers or any caller to proactively
    /// extend the TTL of Pool records without requiring specific get_pool or
    /// get_pools_by_asset queries. This is critical for long-lived factory
    /// deployments where early pools may go unqueried for extended periods.
    ///
    /// # Arguments
    /// * `start_id` - The first pool ID to refresh (inclusive)
    /// * `limit` - Maximum number of pools to refresh in this call (capped at 20)
    ///
    /// # Important Notes
    /// - This is a **keep-alive mechanism** that prevents archival by refreshing
    ///   TTLs before expiry. It does NOT restore already-archived entries.
    /// - Already-archived entries require off-chain RestoreFootprint operations
    ///   (e.g., via Soroban CLI) submitted alongside a transaction referencing the
    ///   expired key - this is outside contract code's control.
    /// - Instance storage (Admin, WasmHash, PoolCount) is bumped by bump_instance
    ///   in nearly every public function, so it does not require separate refresh.
    /// - Only persistent Pool(u32) records are at risk of archival due to their
    ///   narrower bump coverage (only from pool-specific read paths).
    ///
    /// # Keeper Cadence
    /// Operators should call this function across the full ID range at least once
    /// every ~45 days (between TTL_THRESHOLD of ~30 days and TTL_EXTEND_TO of
    /// ~60 days) to ensure all pool records remain accessible.
    ///
    /// # Events
    /// Emits a `ttl_ref` event with `(start_id, end)` so off-chain monitoring
    /// can track when each pool range was last refreshed (#328).
    ///
    /// # Security implications of being permissionless (#168)
    /// Any caller may extend pool-record TTLs. The blast radius is intentionally
    /// bounded and non-hazardous:
    /// - A refresh only **extends** TTLs; it can never shorten them, prune
    ///   records, or mutate pool data. It therefore cannot corrupt state.
    /// - Each call is capped at 20 pool IDs and fees are paid by the caller, so
    ///   an attacker cannot cheaply undercut rational keepers nor permanently
    ///   pin a pool against archival — every ~45-day cycle requires a fresh
    ///   low-value call and re-accruing storage rent.
    /// - Pool records that should be retired are at most kept alive (not
    ///   protected in any other way): they remain stale and can be ignored by
    ///   frontends, and the pool's on-chain asset transfers are unaffected.
    /// - The maximum consequential cost of abuse is bounded storage-rent for
    ///   stale records, which is recovered through fees paid by the refresher.
    ///
    /// This is why the function deliberately remains permissionless: an
    /// admin-gated or rate-limited variant would reintroduce the archival risk
    /// it exists to prevent (a factory whose only keep-alive relies on a single
    /// admin becomes a single point of failure). For pools that must be
    /// decommissioned, the durable admin path is to stop refreshing them and
    /// let their TTL lapse, or archive/white-list them off-chain rather than
    /// relying on attacker interference.
    ///
    /// Returns a `RefreshPoolTtlsResponse` listing the IDs that were actually
    /// refreshed (#393). The existence check was already in place; the sweep
    /// previously returned `Ok(())`, so a caller could not tell which IDs had
    /// a record and were kept alive versus which were skipped as missing.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn refresh_pool_ttls(
        env: Env,
        start_id: u32,
        limit: u32,
    ) -> Result<RefreshPoolTtlsResponse, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        let capped_limit = limit.min(20);
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut refreshed: Vec<u32> = Vec::new(&env);
        let mut missing = 0u32;
        for pool_id in start_id..end {
            if env.storage().persistent().has(&DataKey::Pool(pool_id)) {
                bump_pool(&env, pool_id);
                refreshed.push_back(pool_id);
            } else {
                missing += 1;
            }
        }
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("ttl_ref")),
            (start_id, end),
        );
        Ok(RefreshPoolTtlsResponse {
            refreshed,
            end_id: end,
            missing,
        })
    }

    /// Transfer admin rights to `new_admin`. Current admin must authorise.
    ///
    /// Supports key rotation and future governance handoffs without redeploying
    /// the factory. Emits a `adm_xfr` event with `(old_admin, new_admin)`.
    ///
    /// Rejects the zero address (`GAAA...WHF`) with `InvalidAdmin`: handing
    /// the factory to an unusable admin would permanently lock it (#329).
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let current = load_admin(&env)?;
        current.require_auth();
        bump_instance(&env);
        // Validate after auth so unauthenticated callers still hit the auth
        // check first (mirrors `set_pool_wasm_hash`'s validate-after-auth order).
        if new_admin
            == Address::from_string(&String::from_str(
                &env,
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
            ))
        {
            return Err(FactoryError::InvalidAdmin);
        }
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("adm_xfr")),
            (current, new_admin.clone()),
        );
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        increment_admin_transfer_count(&env);
        Ok(())
    }

    /// Return the total number of admin transfers performed.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn admin_transfer_count(env: Env) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_admin_transfer_count(&env))
    }

    pub fn get_admin_transfer_count(env: Env) -> Result<u32, FactoryError> {
        Self::admin_transfer_count(env)
    }

    /// Upgrade one registered farming pool in place. Admin-only.
    ///
    /// This deliberately does not update the factory-level `WasmHash`; it is a
    /// pool-by-pool hot swap for a pre-installed WASM hash.
    ///
    /// A pool's admin is fixed to the factory's admin *at creation time*
    /// (see `create_pool`'s docs) and does not follow later `transfer_admin`
    /// calls on either the factory or the pool itself. The pool's own
    /// `upgrade` entry point independently requires its stored admin's
    /// authorization, so once the two admins diverge, this factory-admin-gated
    /// call would silently also need a second signature from the pool's
    /// (possibly unrelated) admin to succeed. Rather than let that surface as
    /// an opaque missing-authorization host trap, check the pool's current
    /// admin against the factory's admin up front and fail with a typed
    /// `PoolAdminMismatch` error when they no longer match.
    ///
    /// Furthermore, if the deployed pool does not support upgrades (e.g. an older
    /// deployment lacking an `upgrade` entry point) or if invocation fails, `try_invoke_contract`
    /// catches the failure and returns a typed `PoolUpgradeFailed` error instead of panicking.
    fn upgrade_pool_inner(
        env: &Env,
        admin: &Address,
        pool_id: u32,
        new_wasm_hash: &BytesN<32>,
    ) -> Result<(), FactoryError> {
        let key = DataKey::Pool(pool_id);
        let mut record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&key)
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(env, pool_id);

        let pool_admin_args: Vec<Val> = vec![env];
        let pool_admin_res = env.try_invoke_contract::<Address, soroban_sdk::Error>(
            &record.address,
            &Symbol::new(env, "admin"),
            pool_admin_args,
        );
        let pool_admin = match pool_admin_res {
            Ok(Ok(addr)) => addr,
            _ => return Err(FactoryError::PoolUpgradeFailed),
        };
        if &pool_admin != admin {
            return Err(FactoryError::PoolAdminMismatch);
        }
        if new_wasm_hash == &record.wasm_hash {
            return Err(FactoryError::PoolUpgradeFailed);
        }
        if new_wasm_hash == &BytesN::from_array(env, &[0u8; 32]) {
            return Err(FactoryError::InvalidWasmHash);
        }

        let upgrade_args: Vec<Val> = vec![env, new_wasm_hash.clone().into_val(env)];
        let upgrade_res = env.try_invoke_contract::<(), soroban_sdk::Error>(
            &record.address,
            &Symbol::new(env, "upgrade"),
            upgrade_args,
        );
        match upgrade_res {
            Ok(Ok(())) => {}
            _ => return Err(FactoryError::PoolUpgradeFailed),
        }

        let old_hash = record.wasm_hash.clone();
        let old_record = record.clone();
        record.wasm_hash = new_wasm_hash.clone();
        record.version = record.version.saturating_add(1);
        env.storage().persistent().set(&key, &record);

        // Record upgrade in configuration history (#680)
        record_config_change(
            env,
            pool_id,
            symbol_short!("upgraded"),
            Some(old_record),
            Some(record.clone()),
        );

        let old_wasm_key = DataKey::PoolsByWasmHash(old_hash.clone());
        if let Some(old_pool_ids) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<u32>>(&old_wasm_key)
        {
            let mut new_old_ids: Vec<u32> = vec![env];
            for id in old_pool_ids.iter() {
                if id != pool_id {
                    new_old_ids.push_back(id);
                }
            }
            env.storage().persistent().set(&old_wasm_key, &new_old_ids);
        }

        let new_wasm_key = DataKey::PoolsByWasmHash(new_wasm_hash.clone());
        let mut new_pool_ids: Vec<u32> = env
            .storage()
            .persistent()
            .get(&new_wasm_key)
            .unwrap_or_else(|| vec![env]);
        new_pool_ids.push_back(pool_id);
        env.storage().persistent().set(&new_wasm_key, &new_pool_ids);
        bump_wasm_pools(env, new_wasm_hash);

        env.storage().instance().set(
            &DataKey::UpgradeCount,
            &read_upgrade_count(env).saturating_add(1),
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("pool_upg")),
            (pool_id, record.address, old_hash, new_wasm_hash.clone()),
        );

        Ok(())
    }

    /// Hot-swap the underlying WASM bytecode of a single registered farming pool (#258).
    ///
    /// Upgrades only the target pool and leaves the factory's own `WasmHash`
    /// (the default hash for future `create_pool` calls) unchanged.
    ///
    /// Pool records keep a snapshot of the WASM hash they were deployed with.
    /// When upgrading via this function:
    /// 1. The factory verifies caller is factory admin and that the target pool's admin matches.
    /// 2. Invokes the pool's `upgrade(new_wasm_hash)` entry point.
    /// 3. Updates the `PoolRecord.wasm_hash` in factory storage so registry queries reflect the change.
    /// 4. Updates the secondary `PoolsByWasmHash` index and increments `UpgradeCount`.
    /// 5. Emits a `pool_upg` event with old and new WASM hashes for full auditability.
    ///
    /// Furthermore, if the deployed pool does not support upgrades (e.g. an older
    /// deployment lacking an `upgrade` entry point) or if invocation fails, `try_invoke_contract`
    /// catches the failure and returns a typed `PoolUpgradeFailed` error instead of panicking.
    pub fn upgrade_pool(
        env: Env,
        pool_id: u32,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        validate_factory_admin(&env, &admin)?;
        admin.require_auth();
        bump_instance(&env);

        Self::upgrade_pool_inner(&env, &admin, pool_id, &new_wasm_hash)
    }

    /// Record one authenticated, immutable ballot per wallet for a registered pool.
    ///
    /// Voting is advisory: it never directly changes pool parameters or grants
    /// administrator privileges. One-wallet-one-vote is not stake-weighted and
    /// must not be treated as Sybil-resistant governance.
    pub fn vote_on_pool(
        env: Env,
        pool_id: u32,
        voter: Address,
        approve: bool,
    ) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        voter.require_auth();
        let pool_key = DataKey::Pool(pool_id);
        if !env.storage().persistent().has(&pool_key) {
            return Err(FactoryError::PoolNotFound);
        }
        bump_instance(&env);
        bump_pool(&env, pool_id);
        let ballot_key = DataKey::PoolVote(pool_id, voter.clone());
        if env.storage().persistent().has(&ballot_key) {
            return Err(FactoryError::AlreadyVoted);
        }
        let count_key = if approve {
            DataKey::PoolYesVotes(pool_id)
        } else {
            DataKey::PoolNoVotes(pool_id)
        };
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let next = count.checked_add(1).ok_or(FactoryError::PoolCountOverflow)?;
        env.storage().persistent().set(&ballot_key, &approve);
        env.storage().persistent().extend_ttl(&ballot_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        env.storage().persistent().set(&count_key, &next);
        env.storage().persistent().extend_ttl(&count_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        #[allow(deprecated)]
        env.events().publish((symbol_short!("factory"), symbol_short!("pool_vote")), (pool_id, voter, approve));
        Ok(())
    }

    /// Read approval and rejection totals for a registered pool.
    pub fn pool_vote_totals(env: Env, pool_id: u32) -> Result<(u32, u32), FactoryError> {
        require_initialized(&env)?;
        let pool_key = DataKey::Pool(pool_id);
        if !env.storage().persistent().has(&pool_key) {
            return Err(FactoryError::PoolNotFound);
        }
        bump_instance(&env);
        bump_pool(&env, pool_id);
        let yes_key = DataKey::PoolYesVotes(pool_id);
        let no_key = DataKey::PoolNoVotes(pool_id);
        let yes = env.storage().persistent().get(&yes_key).unwrap_or(0);
        let no = env.storage().persistent().get(&no_key).unwrap_or(0);
        if env.storage().persistent().has(&yes_key) {
            env.storage().persistent().extend_ttl(&yes_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
        if env.storage().persistent().has(&no_key) {
            env.storage().persistent().extend_ttl(&no_key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
        Ok((yes, no))
    }

    /// Read a wallet's ballot, or None if no ballot exists.
    pub fn pool_vote_of(env: Env, pool_id: u32, voter: Address) -> Result<Option<bool>, FactoryError> {
        require_initialized(&env)?;
        if !env.storage().persistent().has(&DataKey::Pool(pool_id)) {
            return Err(FactoryError::PoolNotFound);
        }
        bump_instance(&env);
        bump_pool(&env, pool_id);
        let key = DataKey::PoolVote(pool_id, voter);
        let vote = env.storage().persistent().get(&key);
        if vote.is_some() {
            env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
        Ok(vote)
    }

    /// In-place WASM migration: preserves the pool's address and persistent
    /// storage. This is not a transfer of assets to a new contract address.
    /// Uses the existing audited admin authorization and upgrade checks.
    pub fn migrate_pool(env: Env, pool_id: u32, new_wasm_hash: BytesN<32>) -> Result<(), FactoryError> {
        Self::upgrade_pool(env, pool_id, new_wasm_hash)
    }

    /// Upgrade multiple registered farming pools in batch. Admin-only.
    ///
    /// Atomically upgrades all specified `pool_ids` to `new_wasm_hash`. If any upgrade fails,
    /// the transaction reverts and rolls back the entire batch.
    pub fn upgrade_pools_batch(
        env: Env,
        pool_ids: Vec<u32>,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        validate_factory_admin(&env, &admin)?;
        admin.require_auth();
        bump_instance(&env);

        for pool_id in pool_ids.iter() {
            Self::upgrade_pool_inner(&env, &admin, pool_id, &new_wasm_hash)?;
        }
        Ok(())
    }

    /// Total number of successful `upgrade_pool` calls performed by this
    /// factory, for pool-version tracking and analytics (#258).
    pub fn upgrade_count(env: Env) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_upgrade_count(&env))
    }

    /// Aggregate value locked across every pool this factory has created, in
    /// the pools' staking-asset base units (#249).
    ///
    /// This is an O(1) read of an incrementally-maintained accumulator, not a
    /// live fan-out across pools. Each pool contributes the TVL captured by its
    /// most recent `sync_pool_tvl` call; `create_pool` seeds a new pool at 0.
    /// Staking activity between syncs is not reflected until `sync_pool_tvl`
    /// (or `sync_all_pool_tvls`) runs for that pool. This is deliberate: a
    /// factory receives no callback from a pool's stake / unstake, and a true
    /// live sum would need an unbounded cross-contract fan-out that does not
    /// fit Soroban's per-invocation footprint limit. Dashboards that need a
    /// fresh figure should run `sync_all_pool_tvls` first.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn total_tvl(env: Env) -> Result<i128, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_total_tvl(&env))
    }

    /// The per-pool TVL term currently folded into `total_tvl` for `pool_id` —
    /// the value captured by the last `sync_pool_tvl` for this pool, or 0 if it
    /// has never been synced since creation.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized, or
    /// `PoolNotFound` if `pool_id` has not been created.
    pub fn pool_tvl_synced(env: Env, pool_id: u32) -> Result<i128, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        if !env.storage().persistent().has(&DataKey::Pool(pool_id)) {
            return Err(FactoryError::PoolNotFound);
        }
        Ok(read_pool_tvl(&env, pool_id))
    }

    /// Live TVL of one pool, read straight from the deployed pool contract
    /// via its `total_staked` getter. Unlike the `total_tvl` accumulator this
    /// always reflects the pool's current state, at the cost of a
    /// cross-contract call.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized,
    /// `PoolNotFound` for an unknown `pool_id`, or `PoolQueryFailed` if the
    /// deployed pool does not answer the TVL getters.
    pub fn pool_tvl(env: Env, pool_id: u32) -> Result<i128, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);
        query_pool_tvl(&env, &record.address)
    }

    /// Refresh one pool's contribution to `total_tvl` and return its live TVL.
    ///
    /// Permissionless — dashboards, keepers, or the pool's own users can call
    /// it to keep the aggregate current. Reads the pool's live TVL, adjusts the
    /// `total_tvl` accumulator by the delta versus this pool's last-synced
    /// value, and stores the new snapshot. Emits a `tvl_sync` event carrying
    /// `(pool_id, old_snapshot, new_tvl)` when the value changed.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized,
    /// `PoolNotFound` for an unknown `pool_id`, or `PoolQueryFailed` if the
    /// deployed pool does not answer the TVL getters.
    pub fn sync_pool_tvl(env: Env, pool_id: u32) -> Result<i128, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);
        apply_tvl_sync(&env, pool_id, &record.address)
    }

    /// Batch-refresh a contiguous range of pools' `total_tvl` contributions,
    /// starting at `start_id` and covering at most
    /// `min(limit, MAX_POOL_SCAN_PER_CALL)` pool IDs. Pools that fail to answer
    /// the TVL getters are skipped rather than aborting the batch. Returns the
    /// next `start_id` to pass for continued paging, or a value `>= pool_count`
    /// once the registry is exhausted. Permissionless, mirroring
    /// `refresh_pool_ttls`.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn sync_all_pool_tvls(env: Env, start_id: u32, limit: u32) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        let window = limit.min(MAX_POOL_SCAN_PER_CALL);
        let end = start_id.saturating_add(window).min(count);
        let mut pool_id = start_id;
        while pool_id < end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            {
                let _ = apply_tvl_sync(&env, pool_id, &record.address);
            }
            pool_id += 1;
        }
        Ok(end)
    }

    /// Query the health status of an individual pool (Issue #375).
    ///
    /// Returns a `PoolStatus` struct containing the pool's address, pause status,
    /// and total staked amount. This provides a centralized way to check pool
    /// health without querying each pool contract directly.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized,
    /// `PoolNotFound` for an unknown `pool_id`, or `PoolQueryFailed` if the
    /// deployed pool does not respond to the status queries.
    pub fn pool_status(env: Env, pool_id: u32) -> Result<PoolStatus, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let no_args: Vec<Val> = vec![&env];

        // Query is_paused status
        let is_paused = match env.try_invoke_contract::<bool, soroban_sdk::Error>(
            &record.address,
            &Symbol::new(&env, "is_paused"),
            no_args.clone(),
        ) {
            Ok(Ok(v)) => v,
            _ => return Err(FactoryError::PoolQueryFailed),
        };

        // Query total_staked
        let total_staked = match env.try_invoke_contract::<i128, soroban_sdk::Error>(
            &record.address,
            &Symbol::new(&env, "total_staked"),
            no_args,
        ) {
            Ok(Ok(v)) => v,
            _ => return Err(FactoryError::PoolQueryFailed),
        };

        Ok(PoolStatus {
            address: record.address,
            is_paused,
            total_staked,
        })
    }

    /// Update the WASM hash used for future `create_pool` deployments. Admin-only.
    ///
    /// Allows the admin to point future pool deployments at a corrected or upgraded
    /// farming-pool build without redeploying the factory itself. Existing deployed
    /// pools are unaffected — Soroban contract bytecode is immutable once deployed.
    /// Validates that `new_hash` is non-zero. Callers/admins must verify that the target
    /// WASM has been uploaded to the chain before calling this function.
    ///
    /// Emits a `wasm_set` event with `(old_hash, new_hash)` so that the previous
    /// hash is discoverable off-chain for rollback scenarios.
    pub fn set_pool_wasm_hash(env: Env, new_hash: BytesN<32>) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin: Address = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        if new_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(FactoryError::InvalidWasmHash);
        }

        let old_hash: BytesN<32> = env.storage().instance().get(&DataKey::WasmHash).unwrap();
        if new_hash == old_hash {
            return Err(FactoryError::SameWasmHash);
        }
        // An admin-approved hash may be used for future deployments and
        // upgrades. The approval is intentionally explicit because a hash
        // alone cannot prove that its uploaded WASM implements FarmingPool.
        env.storage()
            .persistent()
            .set(&DataKey::ApprovedWasmHash(new_hash.clone()), &true);
        env.storage().persistent().extend_ttl(
            &DataKey::ApprovedWasmHash(new_hash.clone()),
            TTL_THRESHOLD,
            TTL_EXTEND_TO,
        );
        env.storage().instance().set(&DataKey::WasmHash, &new_hash);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("wasm_set")),
            (old_hash, new_hash),
        );
        Ok(())
    }

    /// Pause pool creation. Admin-only.
    ///
    /// Prevents future `create_pool` calls during maintenance or security emergencies.
    /// Emits a `pause_cr` event.
    pub fn pause_pool_creation(env: Env) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::PoolCreationPaused, &true);
        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("factory"), symbol_short!("pause_cr")), admin);
        Ok(())
    }

    /// Resume pool creation. Admin-only.
    ///
    /// Allows `create_pool` calls after a pause. Emits an `unps_cr` event.
    pub fn unpause_pool_creation(env: Env) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::PoolCreationPaused, &false);
        #[allow(deprecated)]
        env.events()
            .publish((symbol_short!("factory"), symbol_short!("unps_cr")), admin);
        Ok(())
    }

    /// Return whether pool creation is currently paused.
    pub fn is_pool_creation_paused(env: Env) -> Result<bool, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env
            .storage()
            .instance()
            .get(&DataKey::PoolCreationPaused)
            .unwrap_or(false))
    }

    /// Validate pool configuration parameters before creating a pool (issue #704).
    ///
    /// Performs all validation checks that `create_pool` would perform, but
    /// without actually creating a pool. This allows callers to validate
    /// configuration beforehand to improve UX and catch errors at the API
    /// boundary rather than at runtime.
    ///
    /// Returns `Ok(true)` if all validations pass, or an appropriate
    /// `FactoryError` if any check fails.
    ///
    /// ## Validations performed:
    /// - Asset is not zero address
    /// - Asset is not the factory contract itself
    /// - Asset is not the same as the admin address
    /// - Global multiplier is within bounds (1 to MAX_GLOBAL_MULTIPLIER)
    /// - Daily rate is within bounds and converts to a valid credit rate
    /// - Min lock period is within bounds (MIN_LOCK_PERIOD to MAX_MIN_LOCK_PERIOD)
    /// - Min stake amount is within acceptable range
    pub fn validate_config(
        env: Env,
        admin: Address,
        asset: Address,
        daily_rate: u128,
        global_multiplier: u32,
        min_lock_period: u64,
        min_stake_amount: i128,
    ) -> Result<bool, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        validate_asset(&env, &asset)?;

        // Validation: Ensure asset address is not zero/null
        let zero_addr = Address::from_string(&String::from_str(
            &env,
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        ));
        if asset == zero_addr {
            return Err(FactoryError::InvalidAsset);
        }

        // Validation: Ensure the staking asset is not the factory contract itself
        if asset == env.current_contract_address() {
            return Err(FactoryError::InvalidAsset);
        }

        // Validation: Ensure the staking asset is not also the pool admin
        if asset == admin {
            return Err(FactoryError::InvalidAsset);
        }

        // Validation: Ensure global_multiplier is strictly within bounds
        if global_multiplier < 1 {
            return Err(FactoryError::InvalidGlobalMultiplier);
        }
        if global_multiplier > MAX_GLOBAL_MULTIPLIER {
            return Err(FactoryError::InvalidGlobalMultiplier);
        }

        // Validation: Bound the caller-facing daily_rate directly
        if daily_rate > MAX_DAILY_RATE {
            return Err(FactoryError::InvalidCreditRate);
        }

        // Validation: Convert and validate daily_rate
        let credit_rate = daily_rate_to_credit_rate(daily_rate)?;

        if credit_rate <= 0 || credit_rate > MAX_CREDIT_RATE {
            return Err(FactoryError::InvalidCreditRate);
        }

        // Validation: Reject a zero min_lock_period
        if min_lock_period == 0 {
            return Err(FactoryError::MinLockPeriodTooShort);
        }
        let min_lock_period_u32: u32 = min_lock_period
            .try_into()
            .map_err(|_| FactoryError::MinLockPeriodOutOfRange)?;
        if min_lock_period_u32 < MIN_LOCK_PERIOD {
            return Err(FactoryError::MinLockPeriodTooShort);
        }
        if min_lock_period_u32 > MAX_MIN_LOCK_PERIOD {
            return Err(FactoryError::MinLockPeriodAboveCeiling);
        }

        // Validation: Check min_stake_amount bounds
        if min_stake_amount > MAX_STAKE_AMOUNT {
            return Err(FactoryError::InvalidMinStakeAmount);
        }
        if min_stake_amount < -MAX_STAKE_AMOUNT {
            return Err(FactoryError::InvalidMinStakeAmount);
        }

        Ok(true)
    }

    /// Create, deploy, and initialize a new farming pool. Admin-only.
    ///
    /// Unlike the pre-#80 version of this function, the deployed pool is no
    /// longer left uninitialized: `create_pool` calls the pool's own
    /// `initialize` in the same transaction as the deploy, so there is no
    /// window in which an uninitialized pool address is observable on-chain
    /// (closing the front-run window described in #79).
    ///
    /// `daily_rate` is converted to the pool's native per-ledger
    /// `credit_rate` via `daily_rate_to_credit_rate` — see that function's
    /// docs for the conversion and its failure modes.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    /// The pool's admin is fixed to this factory's admin *at creation time*.
    /// A later `transfer_admin` on the factory does not retroactively change
    /// any already-deployed pool's admin — each pool is administered
    /// independently after creation. This is approach B from #80: the
    /// smallest-diff option that avoids the larger "factory proxies every
    /// admin action" design surface.
    ///
    /// The `pool_crtd` event includes `admin`, `asset`, `credit_rate`,
    /// `global_multiplier`, `min_lock_period`, and `min_stake_amount`
    /// alongside `pool_id` and `pool_address` so off-chain indexers can
    /// reconstruct the full pool state — including who created it — without
    /// a follow-up RPC call (#233, #330).
    ///
    /// On failure, no event is emitted: a validation failure reverts this
    /// invocation, and Soroban discards contract events published by reverted
    /// calls. Callers must handle the returned `FactoryError` directly (and,
    /// off-chain, can monitor for failed creation attempts via failed
    /// transaction diagnostics rather than contract events).
    pub fn create_pool(
        env: Env,
        asset: Address,
        daily_rate: u128,
        global_multiplier: u32,
        min_lock_period: u64,
        min_stake_amount: i128,
    ) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        validate_factory_admin(&env, &admin)?;
        admin.require_auth();

        create_pool_inner(
            env,
            admin,
            asset,
            daily_rate,
            global_multiplier,
            min_lock_period,
            min_stake_amount,
        )
    }

    /// Register an externally-deployed farming pool with the factory (#665).
    ///
    /// Unlike `create_pool`, this function does **not** deploy a new contract.
    /// It records a caller-supplied `PoolRecord` in the factory's registry so
    /// the pool appears in `list_pools` / `get_pool` queries.  This is useful
    /// when migrating pools from another factory instance or when a pool was
    /// deployed directly (not via this factory) and needs to be brought under
    /// management.
    ///
    /// The admin is required to authorise the import.  Validation checks:
    /// - the pool address must not already be registered
    /// - the asset must pass the same zero-address / self-referential checks as
    ///   `create_pool`
    /// - `global_multiplier` and `credit_rate` must be within the allowed caps
    pub fn import_pool(env: Env, record: PoolRecord) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        validate_factory_admin(&env, &admin)?;
        admin.require_auth();
        bump_instance(&env);

        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::PoolCreationPaused)
            .unwrap_or(false);
        if paused {
            return Err(FactoryError::PoolCreationPaused);
        }

        validate_asset(&env, &record.asset)?;
        if record.global_multiplier < 1 || record.global_multiplier > MAX_GLOBAL_MULTIPLIER {
            return Err(FactoryError::InvalidGlobalMultiplier);
        }
        if record.credit_rate <= 0 {
            return Err(FactoryError::InvalidCreditRate);
        }

        let pool_id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        let next_count = pool_id.saturating_add(1);

        env.storage()
            .instance()
            .set(&DataKey::PoolCount, &next_count);
        env.storage()
            .persistent()
            .set(&DataKey::Pool(pool_id), &record);
        bump_pool(&env, pool_id);

        let asset_key = DataKey::AssetPools(record.asset.clone());
        let mut asset_pools: Vec<u32> = env
            .storage()
            .persistent()
            .get(&asset_key)
            .unwrap_or_else(|| Vec::new(&env));
        asset_pools.push_back(pool_id);
        env.storage().persistent().set(&asset_key, &asset_pools);

        let asset_count_key = DataKey::AssetPoolCount(record.asset.clone());
        let count: u32 = env
            .storage()
            .persistent()
            .get(&asset_count_key)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&asset_count_key, &count.saturating_add(1));

        let admin_key = DataKey::PoolsByAdmin(admin.clone());
        let mut admin_pools: Vec<u32> = env
            .storage()
            .persistent()
            .get(&admin_key)
            .unwrap_or_else(|| Vec::new(&env));
        admin_pools.push_back(pool_id);
        env.storage().persistent().set(&admin_key, &admin_pools);

        env.storage()
            .persistent()
            .set(&DataKey::PoolTvl(pool_id), &0i128);

        Ok(pool_id)
    }

    /// Create multiple pools atomically using the same validation and setup as
    /// `create_pool`. If any item is invalid, the transaction rolls back every
    /// pool created earlier in this batch.
    pub fn create_pools_batch(env: Env, pools: Vec<PoolParams>) -> Result<Vec<u32>, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        validate_factory_admin(&env, &admin)?;
        admin.require_auth();

        // Validation 50: Reject an empty batch before any work is done. A
        // zero-length request consumes admin authorization and bumps instance
        // TTL while creating nothing (#543).
        if pools.is_empty() {
            return Err(FactoryError::EmptyBatch);
        }
        // Validation 35: Cap the batch size before any pool is deployed. Each
        // entry triggers a WASM deploy inside `create_pool_inner`, so an
        // unbounded batch can exhaust the transaction's resource budget and
        // trap with an opaque host error rather than a typed failure (#528-#531).
        if pools.len() > MAX_BATCH_SIZE {
            return Err(FactoryError::BatchTooLarge);
        }

        let mut pool_ids = vec![&env];
        for params in pools.iter() {
            pool_ids.push_back(create_pool_inner(
                env.clone(),
                admin.clone(),
                params.asset,
                params.daily_rate,
                params.global_multiplier,
                params.min_lock_period,
                params.min_stake_amount,
            )?);
        }
        Ok(pool_ids)
    }

    /// Mark a pool as deprecated. Admin-only.
    ///
    /// Deprecated pools remain in the registry and are still readable, but the
    /// flag signals that the pool is decommissioned — frontends should warn
    /// users, and keepers can stop refreshing its TTLs. This provides an
    /// operational control path for compromised or migrated pools without
    /// destroying on-chain history. See #602.
    ///
    /// Returns `PoolNotFound` for an unknown pool, `AlreadyDeprecated` if the
    /// pool was already marked, or `NotInitialized` if the factory has not been
    /// initialized.
    pub fn deprecate_pool(env: Env, pool_id: u32) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        let key = DataKey::Pool(pool_id);
        let mut record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&key)
            .ok_or(FactoryError::PoolNotFound)?;

        if record.deprecated {
            return Err(FactoryError::AlreadyDeprecated);
        }

        let old_record = record.clone();
        record.deprecated = true;
        env.storage().persistent().set(&key, &record);
        bump_pool(&env, pool_id);

        // Record deprecation in configuration history (#680)
        record_config_change(
            &env,
            pool_id,
            symbol_short!("deprecate"),
            Some(old_record),
            Some(record),
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("pool_dep")),
            pool_id,
        );

        Ok(())
    }

    /// Return whether a pool has been deprecated by the admin.
    ///
    /// Returns `PoolNotFound` for an unknown pool, or `NotInitialized` if the
    /// factory has not been initialized.
    pub fn is_pool_deprecated(env: Env, pool_id: u32) -> Result<bool, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let key = DataKey::Pool(pool_id);
        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&key)
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        Ok(record.deprecated)
    }

    /// Claim from the insurance fund for pool losses. Admin-only.
    ///
    /// Provides a loss protection mechanism where pool insurance claims
    /// can be processed to protect against unexpected losses or pool failures.
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn claim_insurance(env: Env, pool_id: u32, amount: i128) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        if amount <= 0 {
            return Err(FactoryError::InvalidCreditRate);
        }

        let _record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let current_insurance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::InsuranceFund(pool_id))
            .unwrap_or(0);

        if current_insurance < amount {
            return Err(FactoryError::InsufficientFunds);
        }

        env.storage().persistent().set(
            &DataKey::InsuranceFund(pool_id),
            &(current_insurance - amount),
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("ins_claim")),
            (pool_id, amount),
        );

        Ok(())
    }

    /// Get the insurance fund balance for a specific pool.
    pub fn get_insurance_balance(env: Env, pool_id: u32) -> Result<i128, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let _record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;

        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::InsuranceFund(pool_id))
            .unwrap_or(0))
    }
}

mod test;

impl Factory {
    /// Return a `HealthStatus` for the pool identified by `pool_id` (issue #653).
    ///
    /// - `Deprecated` — the pool's `PoolRecord.deprecated` flag is set.
    /// - `Degraded` — the pool exists but its TVL cannot be read (older WASM).
    /// - `Healthy` — the pool exists and its TVL can be read.
    ///
    /// Returns `FactoryError::NotInitialized` if the factory has not been
    /// initialized, or `FactoryError::PoolNotFound` if no pool with that id exists.
    pub fn check_pool_health(env: Env, pool_id: u32) -> Result<HealthStatus, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;

        if record.deprecated {
            return Ok(HealthStatus::Deprecated);
        }

        match query_pool_tvl(&env, &record.address) {
            Ok(_) => Ok(HealthStatus::Healthy),
            Err(_) => Ok(HealthStatus::Degraded),
        }
    }

    /// Returns the configuration change history for a pool (issue #680).
    ///
    /// Provides a paginated list of all configuration change events (creation,
    /// upgrade, deprecation, etc.) for the specified pool in chronological order.
    ///
    /// ## Arguments
    /// * `pool_id` - The pool ID to query
    /// * `start` - Zero-based index to start reading from
    /// * `limit` - Maximum number of events to return
    ///
    /// ## Returns
    /// * `PoolConfigHistoryPage` containing the events and total count
    ///
    /// ## Errors
    /// * `NotInitialized` - If the factory has not been initialized
    /// * `PoolNotFound` - If no pool with the given ID exists
    pub fn get_pool_config_history(
        env: Env,
        pool_id: u32,
        start: u32,
        limit: u32,
    ) -> Result<PoolConfigHistoryPage, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        // Verify pool exists
        let _record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;

        let total = read_pool_config_history_count(&env, pool_id);
        let mut events = Vec::new(&env);

        let end = start.saturating_add(limit).min(total);
        for i in start..end {
            if let Some(event) = env
                .storage()
                .persistent()
                .get::<DataKey, PoolConfigChangeEntry>(&DataKey::PoolConfigHistory(pool_id, i))
            {
                events.push_back(event);
            }
        }

        Ok(PoolConfigHistoryPage { events, total })
    }

    /// Export the pool configuration for a specific pool (issue #698).
    ///
    /// Returns the complete pool configuration data for external use or backup.
    /// This is a read-only operation that does not require authorization.
    ///
    /// ## Arguments
    /// * `pool_id` - The ID of the pool to export
    ///
    /// ## Returns
    /// * `PoolRecord` containing all pool configuration parameters
    ///
    /// ## Errors
    /// * `NotInitialized` - If the factory has not been initialized
    /// * `PoolNotFound` - If no pool with the given ID exists
    pub fn export_config(env: Env, pool_id: u32) -> Result<PoolRecord, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        
        bump_pool(&env, pool_id);
        Ok(record)
    }

    /// Import a pool configuration (issue #701).
    ///
    /// Creates a new pool from an existing PoolRecord configuration.
    /// This allows migrating pools from other factories or restoring backed-up configurations.
    /// Requires admin authorization.
    ///
    /// ## Arguments
    /// Return a snapshot of the pool's current configuration (#710).
    ///
    /// Captures the complete pool configuration including asset, credit rate,
    /// global multiplier, min lock period, daily rate, WASM hash, deprecation
    /// status, and TVL in a single query for analytics and state preservation.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized, or
    /// `PoolNotFound` if the pool ID does not exist.
    pub fn get_config_snapshot(env: Env, pool_id: u32) -> Result<PoolComparison, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let tvl = read_pool_tvl(&env, pool_id);
        let risk_score = compute_pool_risk_score(&env, pool_id, &record)?;

        Ok(PoolComparison {
            pool_id,
            address: record.address,
            asset: record.asset,
            daily_rate: record.daily_rate,
            credit_rate: record.credit_rate,
            global_multiplier: record.global_multiplier,
            min_lock_period: record.min_lock_period,
            tvl,
            risk_score,
            deprecated: record.deprecated,
        })
    }

    /// * `config` - The PoolRecord containing the pool configuration to import
    ///
    /// ## Returns
    /// * `u32` - The newly assigned pool ID
    ///
    /// ## Errors
    /// * `NotInitialized` - If the factory has not been initialized
    /// * `PoolCreationPaused` - If pool creation is currently paused
    /// * Various validation errors if the configuration is invalid
    /// Captures and stores a point-in-time snapshot of a pool's state (#647).
    ///
    /// Snapshots record TVL, global multiplier, credit rate, and deprecation
    /// status at the current ledger so they can be replayed for analytics or
    /// incident review without re-querying historical on-chain state.
    ///
    /// Returns `NotInitialized` or `PoolNotFound` on the usual error paths.
    pub fn take_snapshot(env: Env, pool_id: u32) -> Result<Snapshot, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let tvl = read_pool_tvl(&env, pool_id);
        let ledger = env.ledger().sequence();

        let snapshot = Snapshot {
            pool_id,
            ledger,
            tvl,
            global_multiplier: record.global_multiplier,
            credit_rate: record.credit_rate,
            deprecated: record.deprecated,
        };

        let count = env
            .storage()
            .persistent()
            .get::<DataKey, u32>(&DataKey::PoolSnapshotCount(pool_id))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::PoolSnapshot(pool_id, count), &snapshot);
        env.storage()
            .persistent()
            .set(&DataKey::PoolSnapshotCount(pool_id), &(count + 1));

        Ok(snapshot)
    }

    /// Returns aggregated metrics for a pool, including live TVL, snapshot
    /// count, and the ledger of the most recent snapshot (#650).
    ///
    /// Returns `NotInitialized` or `PoolNotFound` on the usual error paths.
    pub fn get_pool_metrics(env: Env, pool_id: u32) -> Result<PoolMetrics, FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&DataKey::Pool(pool_id))
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let tvl = read_pool_tvl(&env, pool_id);
        let snapshot_count = env
            .storage()
            .persistent()
            .get::<DataKey, u32>(&DataKey::PoolSnapshotCount(pool_id))
            .unwrap_or(0);
        let last_snapshot_ledger = if snapshot_count > 0 {
            env.storage()
                .persistent()
                .get::<DataKey, Snapshot>(&DataKey::PoolSnapshot(pool_id, snapshot_count - 1))
                .map(|s| s.ledger)
                .unwrap_or(0)
        } else {
            0
        };

        Ok(PoolMetrics {
            pool_id,
            tvl,
            snapshot_count,
            last_snapshot_ledger,
            global_multiplier: record.global_multiplier,
            credit_rate: record.credit_rate,
            deprecated: record.deprecated,
        })
    }

    pub fn import_config(env: Env, config: PoolRecord) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();

        // Use the existing internal pool creation logic
        create_pool_inner(
            env,
            admin,
            config.asset,
            config.daily_rate,
            config.global_multiplier,
            config.min_lock_period as u64,
            // min_stake_amount is not stored in PoolRecord, use default
            MIN_STAKE_AMOUNT,
        )
    }
}
