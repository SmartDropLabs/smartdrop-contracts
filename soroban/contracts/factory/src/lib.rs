#![no_std]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec};
use types::{DataKey, FactoryError, ListPoolsResponse, PoolRecord};

// ~30 days at ~5 s/ledger; extend to ~60 days when below threshold.
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;

/// Ledgers per day at the network's ~5s/ledger target, used to convert
/// `create_pool`'s caller-facing `daily_rate` into the pool's native
/// per-ledger `credit_rate`. See `daily_rate_to_credit_rate`.
const LEDGERS_PER_DAY: u128 = 17_280;

/// Convert a "credits per day" figure into the deployed pool's native
/// "credits per ledger" `credit_rate`.
///
/// `daily_rate` is kept as `create_pool`'s public unit because a per-day
/// figure is what off-chain/product code already reasons about; ledger-level
/// rates are an implementation detail of `FarmingPool`. Returns
/// `InvalidCreditRate` if the conversion truncates to zero (`initialize`
/// requires `credit_rate > 0`) or does not fit in `i128`.
fn daily_rate_to_credit_rate(daily_rate: u128) -> Result<i128, FactoryError> {
    let per_ledger = daily_rate / LEDGERS_PER_DAY;
    if per_ledger == 0 {
        return Err(FactoryError::InvalidCreditRate);
    }
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

/// Build a 32-byte salt from a pool ID so each pool gets a unique, reproducible address.
fn pool_salt(env: &Env, pool_id: u32) -> BytesN<32> {
    let mut bytes = [0u8; 32];
    bytes[28..].copy_from_slice(&pool_id.to_be_bytes());
    BytesN::from_array(env, &bytes)
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
        let capped_limit = limit.min(20);
        let end = start_id.saturating_add(capped_limit).min(count);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];

        for pool_id in start_id..end {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                bump_pool(&env, pool_id);
                records.push_back((pool_id, record));
            }
        }

        Ok(ListPoolsResponse {
            records,
            next_start_id: if end < count { end } else { count },
            total: count,
        })
    }

    /// Return a page of pool records whose staking asset matches `asset`.
    ///
    /// Scans registered pools starting from `start_id` and collects matching records.
    /// `limit` is capped at 20 records so callers can page through large registries
    /// without unbounded contract work. This prevents denial-of-service by design as
    /// the registry grows organically.
    ///
    /// # Resource Limit Reasoning
    /// Without pagination, this function performs an unbounded O(n) scan over every
    /// pool ever registered, with no ceiling. As pool_count grows, the function gets
    /// strictly more expensive per call and would eventually exceed Soroban's
    /// per-transaction CPU-instruction and read-entry budgets, becoming permanently
    /// unusable. The 20-record cap mirrors list_pools's design to prevent this.
    ///
    /// Scans every registered pool in O(n) and collects matching IDs.
    /// Useful for frontends that need to surface all pools for a given token
    /// without an off-chain indexer.
    ///
    /// Guarded like `pool_count`: an empty result from an uninitialized factory
    /// would be indistinguishable from "no pools hold this asset".
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn get_pools_by_asset(env: Env, asset: Address) -> Result<Vec<u32>, FactoryError> {
        require_initialized(&env)?;
    /// # Secondary Index Consideration
    /// For very large registries, a secondary per-asset index (e.g., DataKey::AssetPools
    /// maintained incrementally in create_pool) would avoid full-registry scans entirely.
    /// This would be a more robust long-term fix but requires changes to create_pool's
    /// write path and potentially a migration/backfill for existing pools.
    pub fn get_pools_by_asset(env: Env, asset: Address, start_id: u32, limit: u32) -> ListPoolsResponse {
        bump_instance(&env);
        let count: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap_or(0);
        let capped_limit = limit.min(20);
        let mut records: Vec<(u32, PoolRecord)> = vec![&env];
        let mut next_start_id = count;

        for pool_id in start_id..count {
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
        Ok(matches)

        ListPoolsResponse {
            records,
            next_start_id,
            total: count,
        }
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
    pub fn refresh_pool_ttls(env: Env, start_id: u32, limit: u32) -> Result<(), FactoryError> {
        bump_instance(&env);
        let count: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap_or(0);
        let capped_limit = limit.min(20);
        let end = start_id.saturating_add(capped_limit).min(count);
        for pool_id in start_id..end {
            if env.storage().persistent().has(&DataKey::Pool(pool_id)) {
                bump_pool(&env, pool_id);
            }
        }
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
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );
        Ok(())
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
    /// The `pool_crtd` event now includes `asset`, `daily_rate`, and
    /// `min_lock_period` alongside `pool_id` and `pool_address` so off-chain
    /// indexers can reconstruct the full pool state without a follow-up RPC call.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    /// The pool's admin is fixed to this factory's admin *at creation time*.
    /// A later `transfer_admin` on the factory does not retroactively change
    /// any already-deployed pool's admin — each pool is administered
    /// independently after creation. This is approach B from #80: the
    /// smallest-diff option that avoids the larger "factory proxies every
    /// admin action" design surface.
    ///
    /// The `pool_crtd` event includes `asset`, `credit_rate`,
    /// `global_multiplier`, and `min_lock_period` alongside `pool_id` and
    /// `pool_address` so off-chain indexers can reconstruct the full pool
    /// state without a follow-up RPC call.
    pub fn create_pool(
        env: Env,
        asset: Address,
        daily_rate: u128,
        min_lock_period: u64,
    ) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        global_multiplier: u32,
        min_lock_period: u64,
    ) -> Result<u32, FactoryError> {
        let admin = load_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        if global_multiplier < 1 {
            return Err(FactoryError::InvalidGlobalMultiplier);
        }
        let credit_rate = daily_rate_to_credit_rate(daily_rate)?;
        let min_lock_period: u32 = min_lock_period
            .try_into()
            .map_err(|_| FactoryError::MinLockPeriodOutOfRange)?;

        let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
        let wasm_hash = load_wasm_hash(&env)?;
        let salt = pool_salt(&env, pool_id);

        // Deploy a fresh farming-pool instance. The resulting address is
        // deterministic: keccak256(factory_address || salt).
        let pool_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, ());

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
        ];
        let _: () = env.invoke_contract(&pool_address, &Symbol::new(&env, "initialize"), init_args);

        let record = PoolRecord {
            address: pool_address.clone(),
            asset: asset.clone(),
            credit_rate,
            global_multiplier,
            min_lock_period,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Pool(pool_id), &record);
        bump_pool(&env, pool_id);
        env.storage()
            .instance()
            .set(&DataKey::PoolCount, &(pool_id + 1));

        // Emit enriched event so indexers get the full pool parameters in one shot.
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("pool_crtd")),
            (
                pool_id,
                pool_address,
                asset,
                credit_rate,
                global_multiplier,
                min_lock_period,
            ),
        );

        Ok(pool_id)
    }
}

mod test;
