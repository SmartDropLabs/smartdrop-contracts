#![no_std]

mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};
use types::{DataKey, ListPoolsResponse, PoolRecord, PoolSort};

pub use types::FactoryError;

// ~30 days at ~5 s/ledger; extend to ~60 days when below threshold.
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;
// Bound the number of pool IDs examined per call to stay within Soroban's 100 footprint entries limit.
const MAX_POOL_SCAN_PER_CALL: u32 = 50;

/// Ledgers per day at the network's ~5s/ledger target, used to convert
/// `create_pool`'s caller-facing `daily_rate` into the pool's native
/// per-ledger `credit_rate`. See `daily_rate_to_credit_rate`.
const LEDGERS_PER_DAY: u128 = 17_280;
// Minimum stake in the asset's smallest units. This is 0.1 token for the
// standard 7-decimal Stellar asset convention and prevents dust positions.
const MIN_STAKE_AMOUNT: i128 = 1_000_000;
// Minimum lock period in ledgers required to prevent flash-loan-style attacks.
const MIN_LOCK_PERIOD: u32 = 1;

/// Convert a "credits per day" figure into the deployed pool's native
/// "credits per ledger" `credit_rate`.
///
/// `daily_rate` is kept as `create_pool`'s public unit because a per-day
/// figure is what off-chain/product code already reasons about; ledger-level
/// rates are an implementation detail of `FarmingPool`.
///
/// Conversion uses **ceiling** division (`(daily_rate + LEDGERS_PER_DAY - 1) /
/// LEDGERS_PER_DAY`) rather than truncation. Truncation silently dropped valid
/// small daily rates: e.g. `daily_rate = 17_279` divided by `LEDGERS_PER_DAY =
/// 17_280` truncated to `0`, which `FarmingPool::initialize` then rejects with
/// `InvalidCreditRate` (see #148). Ceiling division guarantees that *any*
/// non-zero `daily_rate` yields a positive `credit_rate` of at least `1`
/// (≈ `LEDGERS_PER_DAY` credits/day). The smallest `daily_rate` that converts
/// to exactly `1` ledger credit is `LEDGERS_PER_DAY` itself; smaller values
/// still round up to `1` and therefore over-accrue slightly, so callers
/// wanting an exact daily rate should pass a multiple of `LEDGERS_PER_DAY`.
/// Returns `InvalidCreditRate` only if `daily_rate == 0` or the rounded
/// `credit_rate` does not fit in `i128`.
fn daily_rate_to_credit_rate(daily_rate: u128) -> Result<i128, FactoryError> {
    if daily_rate == 0 {
        return Err(FactoryError::InvalidCreditRate);
    }
    let per_ledger = daily_rate.div_ceil(LEDGERS_PER_DAY);
    i128::try_from(per_ledger).map_err(|_| FactoryError::InvalidCreditRate)
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

fn bump_admin_pools(env: &Env, admin: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolsByAdmin(admin.clone()),
        TTL_THRESHOLD,
        TTL_EXTEND_TO,
    );
}

fn bump_wasm_pools(env: &Env, wasm_hash: &BytesN<32>) {
    env.storage().persistent().extend_ttl(
        &DataKey::PoolsByWasmHash(wasm_hash.clone()),
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
fn pool_salt(env: &Env, pool_id: u32) -> BytesN<32> {
    let mut bytes = [0u8; 32];
    bytes[28..].copy_from_slice(&pool_id.to_be_bytes());
    BytesN::from_array(env, &bytes)
}

fn validate_asset(env: &Env, asset: &Address) -> Result<(), FactoryError> {
    let args: Vec<Val> = vec![&env, env.current_contract_address().into_val(env)];
    match env.try_invoke_contract::<i128, soroban_sdk::Error>(
        asset,
        &Symbol::new(env, "balance"),
        args,
    ) {
        Ok(Ok(balance)) if balance >= 0 => Ok(()),
        Ok(_) => Err(FactoryError::InvalidAsset),
        Err(_) => Ok(()),
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
    env.storage().instance().get(&DataKey::TotalTvl).unwrap_or(0)
}

fn read_pool_tvl(env: &Env, pool_id: u32) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::PoolTvl(pool_id))
        .unwrap_or(0)
}

fn increment_admin_transfer_count(env: &Env) {
    let count = read_admin_transfer_count(env);
    env.storage()
        .instance()
        .set(&DataKey::AdminTransferCount, &(count + 1));
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
        if admin
            == Address::from_string(&soroban_sdk::String::from_str(
                &env,
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
            ))
        {
            return Err(FactoryError::InvalidAdmin);
        }
        if pool_wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(FactoryError::InvalidWasmHash);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::WasmHash, &pool_wasm_hash);
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
        let capped_limit = if limit == 0 { 20 } else { limit.min(20) };
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];

        for pool_id in start_id..end {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                bump_pool(&env, pool_id);
                records.push_back((pool_id, record));
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
        let capped_limit = if limit == 0 { 20 } else { limit.min(20) };
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];

        for pool_id in start_id..end {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                bump_pool(&env, pool_id);
                insert_sorted(&mut records, (pool_id, record), sort);
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
    /// Scans at most `scan_limit` (capped at `MAX_POOL_SCAN_PER_CALL` = 50) pool IDs starting from
    /// `start_id` and collects matching records from that bounded window. `limit` is capped at 20
    /// records so callers can page through large registries without unbounded contract work.
    ///
    /// # Resource Limit & Gas Economics
    /// Without a scan bound, asset-matching queries perform an unbounded O(n) walk across the registry,
    /// inspecting every pool ID until `limit` matches are found or the registry is exhausted. As
    /// `pool_count` scales to thousands of pools, sparse queries would exceed Soroban's per-transaction
    /// footprint limits (100 entries) and CPU instruction budget. Bounding each call to at most 50 IDs
    /// guarantees bounded predictable gas and CPU consumption per call.
    ///
    /// # Caller Range Scanning & Pagination
    /// Callers can specify `scan_limit` (up to 50) to tune the scan window. Callers resume pagination
    /// using `next_start_id` until `next_start_id == total`.
    ///
    /// # Indexer Recommendation
    /// For off-chain applications (such as frontends and analytics) requiring zero-gas instant lookups
    /// across thousands of pools, developers should index the `(symbol_short!("factory"), symbol_short!("pool_crtd"))`
    /// events emitted by `create_pool`, which include `asset` and `pool_id` in their payload.
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
            return Ok(ListPoolsResponse {
                records,
                next_start_id,
                total: count,
                has_more: next_start_id < count,
            });
        }

        for pool_id in start_id..scan_end {
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
    /// Scans at most `MAX_POOL_SCAN_PER_CALL` (50) pool IDs starting from `start_id`
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
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn refresh_pool_ttls(env: Env, start_id: u32, limit: u32) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        let capped_limit = limit.min(20);
        let end = start_id.saturating_add(capped_limit).min(count);
        for pool_id in start_id..end {
            if env.storage().persistent().has(&DataKey::Pool(pool_id)) {
                bump_pool(&env, pool_id);
            }
        }
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("ttl_ref")),
            (start_id, end),
        );
        Ok(())
    }

    /// Transfer admin rights to `new_admin`. Current admin must authorise.
    ///
    /// Supports key rotation and future governance handoffs without redeploying
    /// the factory. Emits a `adm_xfr` event with `(old_admin, new_admin)`.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        require_initialized(&env)?;
        let current = load_admin(&env)?;
        current.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        increment_admin_transfer_count(&env);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );
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
    pub fn upgrade_pool(
        env: Env,
        pool_id: u32,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), FactoryError> {
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        let key = DataKey::Pool(pool_id);
        let mut record = env
            .storage()
            .persistent()
            .get::<DataKey, PoolRecord>(&key)
            .ok_or(FactoryError::PoolNotFound)?;
        bump_pool(&env, pool_id);

        let pool_admin_args: Vec<Val> = vec![&env];
        let pool_admin_res = env.try_invoke_contract::<Address, soroban_sdk::Error>(
            &record.address,
            &Symbol::new(&env, "admin"),
            pool_admin_args,
        );
        let pool_admin = match pool_admin_res {
            Ok(Ok(addr)) => addr,
            _ => return Err(FactoryError::PoolUpgradeFailed),
        };
        if pool_admin != admin {
            return Err(FactoryError::PoolAdminMismatch);
        }
        if new_wasm_hash == record.wasm_hash {
            return Err(FactoryError::PoolUpgradeFailed);
        }

        let upgrade_args: Vec<Val> = vec![&env, new_wasm_hash.clone().into_val(&env)];
        let upgrade_res = env.try_invoke_contract::<(), soroban_sdk::Error>(
            &record.address,
            &Symbol::new(&env, "upgrade"),
            upgrade_args,
        );
        match upgrade_res {
            Ok(Ok(())) => {}
            _ => return Err(FactoryError::PoolUpgradeFailed),
        }

        let old_hash = record.wasm_hash.clone();
        record.wasm_hash = new_wasm_hash.clone();
        env.storage().persistent().set(&key, &record);

        let old_wasm_key = DataKey::PoolsByWasmHash(old_hash.clone());
        if let Some(old_pool_ids) = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<u32>>(&old_wasm_key)
        {
            let mut new_old_ids: Vec<u32> = vec![&env];
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
            .unwrap_or_else(|| vec![&env]);
        new_pool_ids.push_back(pool_id);
        env.storage().persistent().set(&new_wasm_key, &new_pool_ids);
        bump_wasm_pools(&env, &new_wasm_hash);

        env.storage().instance().set(
            &DataKey::UpgradeCount,
            &read_upgrade_count(&env).saturating_add(1),
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("pool_upg")),
            (pool_id, record.address, old_hash, new_wasm_hash),
        );

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
    /// `global_multiplier`, and `min_lock_period` alongside `pool_id` and
    /// `pool_address` so off-chain indexers can reconstruct the full pool
    /// state — including who created it — without a follow-up RPC call (#233).
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

        validate_asset(&env, &asset)?;

        if global_multiplier < 1 {
            return Err(FactoryError::InvalidGlobalMultiplier);
        }
        let credit_rate = daily_rate_to_credit_rate(daily_rate)?;
        let min_lock_period: u32 = min_lock_period
            .try_into()
            .map_err(|_| FactoryError::MinLockPeriodOutOfRange)?;
        if min_lock_period < MIN_LOCK_PERIOD {
            return Err(FactoryError::MinLockPeriodTooShort);
        }
        let effective_min_stake = if min_stake_amount <= 0 {
            MIN_STAKE_AMOUNT
        } else {
            min_stake_amount
        };
        if effective_min_stake < MIN_STAKE_AMOUNT {
            return Err(FactoryError::InvalidMinStakeAmount);
        }

        let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
        let next_count = pool_id
            .checked_add(1)
            .ok_or(FactoryError::PoolCountOverflow)?;
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
        };
        env.storage()
            .persistent()
            .set(&DataKey::Pool(pool_id), &record);
        bump_pool(&env, pool_id);
        let asset_key = DataKey::AssetPools(asset.clone());
        let mut asset_pool_ids: Vec<u32> = env
            .storage()
            .persistent()
            .get(&asset_key)
            .unwrap_or_else(|| vec![&env]);
        asset_pool_ids.push_back(pool_id);
        env.storage().persistent().set(&asset_key, &asset_pool_ids);
        bump_asset_pools(&env, &asset);

        let admin_key = DataKey::PoolsByAdmin(admin.clone());
        let mut admin_pool_ids: Vec<u32> = env
            .storage()
            .persistent()
            .get(&admin_key)
            .unwrap_or_else(|| vec![&env]);
        admin_pool_ids.push_back(pool_id);
        env.storage().persistent().set(&admin_key, &admin_pool_ids);
        bump_admin_pools(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::PoolCount, &next_count);

        // Emit enriched event so indexers get the full pool parameters in one shot.
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
            ),
        );

        Ok(pool_id)
    }
}

mod test;
