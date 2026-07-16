#![no_std]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, vec, Address, BytesN, Env, Vec};
use types::{DataKey, FactoryError, ListPoolsResponse, PoolRecord};

// ~30 days at ~5 s/ledger; extend to ~60 days when below threshold.
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;

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

    /// Return all pool IDs whose staking asset matches `asset`.
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
        bump_instance(&env);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0);
        let mut matches: Vec<u32> = vec![&env];
        for pool_id in 0..count {
            let key = DataKey::Pool(pool_id);
            if let Some(record) = env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
                if record.asset == asset {
                    bump_pool(&env, pool_id);
                    matches.push_back(pool_id);
                }
            }
        }
        Ok(matches)
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

    /// Create and register a new farming pool. Admin-only.
    ///
    /// The `pool_crtd` event now includes `asset`, `daily_rate`, and
    /// `min_lock_period` alongside `pool_id` and `pool_address` so off-chain
    /// indexers can reconstruct the full pool state without a follow-up RPC call.
    ///
    /// Returns `NotInitialized` if the factory has not been initialized.
    pub fn create_pool(
        env: Env,
        asset: Address,
        daily_rate: u128,
        min_lock_period: u64,
    ) -> Result<u32, FactoryError> {
        require_initialized(&env)?;
        let admin = load_admin(&env)?;
        admin.require_auth();
        bump_instance(&env);

        let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
        let wasm_hash = load_wasm_hash(&env)?;
        let salt = pool_salt(&env, pool_id);

        // Deploy a fresh farming-pool instance. The resulting address is
        // deterministic: keccak256(factory_address || salt).
        let pool_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, ());

        let record = PoolRecord {
            address: pool_address.clone(),
            asset: asset.clone(),
            daily_rate,
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
            (pool_id, pool_address, asset, daily_rate, min_lock_period),
        );

        Ok(pool_id)
    }
}

mod test;
