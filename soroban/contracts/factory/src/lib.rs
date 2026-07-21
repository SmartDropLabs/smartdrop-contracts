#![no_std]

mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};
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

fn load_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
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
    pub fn admin(env: Env) -> Address {
        bump_instance(&env);
        load_admin(&env)
    }

    /// Return the WASM hash of the pool implementation this factory deploys.
    ///
    /// Clients can call this to verify which farming-pool build is active before
    /// trusting a pool address returned by `create_pool`.
    pub fn pool_wasm_hash(env: Env) -> BytesN<32> {
        bump_instance(&env);
        env.storage().instance().get(&DataKey::WasmHash).unwrap()
    }

    /// Return the total number of pools registered by this factory.
    pub fn pool_count(env: Env) -> u32 {
        bump_instance(&env);
        env.storage()
            .instance()
            .get(&DataKey::PoolCount)
            .unwrap_or(0)
    }

    /// Return the `PoolRecord` for `pool_id`.
    ///
    /// Returns `PoolNotFound` if `pool_id` has not been created yet.
    pub fn get_pool(env: Env, pool_id: u32) -> Result<PoolRecord, FactoryError> {
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
    pub fn list_pools(env: Env, start_id: u32, limit: u32) -> ListPoolsResponse {
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

        ListPoolsResponse {
            records,
            next_start_id: if end < count { end } else { count },
            total: count,
        }
    }

    /// Return all pool IDs whose staking asset matches `asset`.
    ///
    /// Scans every registered pool in O(n) and collects matching IDs.
    /// Useful for frontends that need to surface all pools for a given token
    /// without an off-chain indexer.
    pub fn get_pools_by_asset(env: Env, asset: Address) -> Vec<u32> {
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
        matches
    }

    /// Transfer admin rights to `new_admin`. Current admin must authorise.
    ///
    /// Supports key rotation and future governance handoffs without redeploying
    /// the factory. Emits a `adm_xfr` event with `(old_admin, new_admin)`.
    pub fn transfer_admin(env: Env, new_admin: Address) {
        let current = load_admin(&env);
        current.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("factory"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );
    }

    /// Create, deploy, and initialize a new farming pool in one atomic call. Admin-only.
    ///
    /// Deployment and initialization happen within this single invocation so
    /// there is no externally-observable window where the pool contract
    /// exists but has no admin — closing the front-running gap where anyone
    /// could call the freshly-deployed pool's own `initialize` first and
    /// seize control of it (#79). The factory's own admin becomes the pool's
    /// initial admin; letting the caller delegate a distinct per-pool admin
    /// (and pass a real `global_multiplier`/`credit_rate` instead of the
    /// neutral defaults below) is #80's broader parameterization, not this
    /// fix's scope.
    ///
    /// The `pool_crtd` event now includes `asset`, `daily_rate`, and
    /// `min_lock_period` alongside `pool_id` and `pool_address` so off-chain
    /// indexers can reconstruct the full pool state without a follow-up RPC call.
    pub fn create_pool(env: Env, asset: Address, daily_rate: u128, min_lock_period: u64) -> u32 {
        let admin = load_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
        let wasm_hash: BytesN<32> = env.storage().instance().get(&DataKey::WasmHash).unwrap();
        let salt = pool_salt(&env, pool_id);

        // Deploy a fresh farming-pool instance. The resulting address is
        // deterministic: keccak256(factory_address || salt).
        let pool_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, ());

        // Initialize the pool immediately, in the same invocation that
        // deployed it — no other caller gets a chance to observe an
        // uninitialized pool at this address and claim admin first.
        // `asset` is passed as both the pool's `stake_token` here and
        // `PoolRecord.asset` below from the same variable, so the factory's
        // record of a pool's staking asset can never diverge from what the
        // pool itself was actually initialized with.
        // farming-pool's `min_lock_period` is `u32` while `create_pool`'s is
        // `u64`; guard against silent truncation rather than casting.
        let pool_min_lock_period: u32 = min_lock_period
            .try_into()
            .expect("min_lock_period exceeds u32::MAX ledgers");
        // Called via `invoke_contract` with raw `Val` args rather than the
        // `farming-pool` crate's generated `FarmingPoolClient`: depending on
        // that crate directly pulls its own `#[contractimpl]`-exported WASM
        // symbols (e.g. `admin`, `transfer_admin`) into this build, and those
        // collide at link time with factory's own exports of the same names
        // once both land in one cdylib.
        let init_args: Vec<Val> = vec![
            &env,
            admin.into_val(&env),
            asset.into_val(&env),
            1u32.into_val(&env),
            1i128.into_val(&env),
            pool_min_lock_period.into_val(&env),
        ];
        let _: () = env.invoke_contract(&pool_address, &Symbol::new(&env, "initialize"), init_args);

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

        pool_id
    }
}

mod test;
