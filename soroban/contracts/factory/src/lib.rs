#![no_std]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, Address, BytesN, Env};
use types::{DataKey, PoolRecord};

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
    /// Initialize the factory. Panics if called more than once.
    pub fn initialize(env: Env, admin: Address, pool_wasm_hash: BytesN<32>) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::WasmHash, &pool_wasm_hash);
        env.storage().instance().set(&DataKey::PoolCount, &0u32);
        bump_instance(&env);
    }

    /// Return the current admin address.
    pub fn admin(env: Env) -> Address {
        bump_instance(&env);
        load_admin(&env)
    }

    /// Return the total number of pools registered by this factory.
    pub fn pool_count(env: Env) -> u32 {
        bump_instance(&env);
        env.storage().instance().get(&DataKey::PoolCount).unwrap_or(0)
    }

    /// Return the `PoolRecord` for `pool_id`. Panics with "pool not found" if missing.
    pub fn get_pool(env: Env, pool_id: u32) -> PoolRecord {
        bump_instance(&env);
        let key = DataKey::Pool(pool_id);
        match env.storage().persistent().get::<DataKey, PoolRecord>(&key) {
            Some(r) => {
                bump_pool(&env, pool_id);
                r
            }
            None => panic!("pool not found"),
        }
    }

    /// Create and register a new farming pool. Admin-only.
    pub fn create_pool(
        env: Env,
        asset: Address,
        daily_rate: u128,
        min_lock_period: u64,
    ) -> u32 {
        let admin = load_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        let pool_id: u32 = env.storage().instance().get(&DataKey::PoolCount).unwrap();
        let wasm_hash: BytesN<32> = env.storage().instance().get(&DataKey::WasmHash).unwrap();
        0
    }
}
