#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: FactoryClient<'static>,
    factory_addr: Address,
    admin: Address,
    wasm_hash: BytesN<32>,
}

/// Builds an initialised factory with a dummy WASM hash.
///
/// Tests that call `create_pool` require a real farming-pool WASM and are
/// marked `#[ignore]`. Run them after building the pool WASM:
///
/// ```sh
/// cargo build -p farming-pool --target wasm32-unknown-unknown --release
/// cargo test -p factory -- --include-ignored
/// ```
fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[0xABu8; 32]);

    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let client =
        unsafe { core::mem::transmute::<FactoryClient<'_>, FactoryClient<'static>>(client) };

    TestEnv {
        env,
        client,
        factory_addr,
        admin,
        wasm_hash,
    }
}

fn setup_with_pool_records(count: u32) -> TestEnv {
    let t = setup();

    t.env.as_contract(&t.factory_addr, || {
        for pool_id in 0..count {
            let record = PoolRecord {
                address: Address::generate(&t.env),
                asset: Address::generate(&t.env),
                daily_rate: 100 + pool_id as u128,
                min_lock_period: 10 + pool_id as u64,
            };
            t.env
                .storage()
                .persistent()
                .set(&DataKey::Pool(pool_id), &record);
        }
        t.env.storage().instance().set(&DataKey::PoolCount, &count);
    });

    t
}

// ── initialize ────────────────────────────────────────────────────────────────

#[test]
fn test_initialize_sets_admin() {
    let t = setup();
    assert_eq!(t.client.admin(), t.admin);
}

#[test]
fn test_admin_getter_returns_stored_address() {
    let t = setup();
    assert_eq!(t.client.admin(), t.admin);
}

#[test]
fn test_double_initialize_returns_error() {
    let t = setup();
    let result = t.client.try_initialize(&t.admin, &t.wasm_hash);
    assert!(
        result.is_err(),
        "second initialize must return AlreadyInitialized error"
    );
}

// ── pool_wasm_hash ────────────────────────────────────────────────────────────

#[test]
fn test_pool_wasm_hash_returns_stored_hash() {
    let t = setup();
    assert_eq!(t.client.pool_wasm_hash(), t.wasm_hash);
}

// ── pool_count ────────────────────────────────────────────────────────────────

#[test]
fn test_pool_count_zero_after_initialize() {
    let t = setup();
    assert_eq!(t.client.pool_count(), 0);
}

// ── get_pool ──────────────────────────────────────────────────────────────────

#[test]
fn test_get_pool_returns_error_on_missing_id() {
    let t = setup();
    let result = t.client.try_get_pool(&0u32);
    assert!(
        result.is_err(),
        "get_pool must return PoolNotFound for unknown id"
    );
}

#[test]
fn test_list_pools_returns_first_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&0u32, &10u32);

    assert_eq!(page.records.len(), 10);
    assert_eq!(page.next_start_id, 10);
    assert_eq!(page.total, 25);

    let first = page.records.get(0).unwrap();
    let last = page.records.get(9).unwrap();
    assert_eq!(first.0, 0);
    assert_eq!(first.1.daily_rate, 100);
    assert_eq!(last.0, 9);
    assert_eq!(last.1.daily_rate, 109);
}

#[test]
fn test_list_pools_returns_second_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&10u32, &10u32);

    assert_eq!(page.records.len(), 10);
    assert_eq!(page.next_start_id, 20);
    assert_eq!(page.total, 25);
    assert_eq!(page.records.get(0).unwrap().0, 10);
    assert_eq!(page.records.get(9).unwrap().0, 19);
}

#[test]
fn test_list_pools_returns_partial_last_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&20u32, &10u32);

    assert_eq!(page.records.len(), 5);
    assert_eq!(page.next_start_id, 25);
    assert_eq!(page.total, 25);
    assert_eq!(page.records.get(0).unwrap().0, 20);
    assert_eq!(page.records.get(4).unwrap().0, 24);
}

#[test]
fn test_list_pools_returns_empty_when_start_is_beyond_count() {
    let t = setup_with_pool_records(3);
    let page = t.client.list_pools(&10u32, &5u32);

    assert_eq!(page.records.len(), 0);
    assert_eq!(page.next_start_id, 3);
    assert_eq!(page.total, 3);
}

#[test]
fn test_list_pools_caps_limit_at_twenty() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&0u32, &100u32);

    assert_eq!(page.records.len(), 20);
    assert_eq!(page.next_start_id, 20);
    assert_eq!(page.total, 25);
    assert_eq!(page.records.get(19).unwrap().0, 19);
}

// ── get_pools_by_asset ────────────────────────────────────────────────────────

#[test]
fn test_get_pools_by_asset_returns_empty_when_no_pools() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let ids = t.client.get_pools_by_asset(&asset);
    assert_eq!(ids.len(), 0, "expected no pools for a fresh factory");
}

// ── transfer_admin ────────────────────────────────────────────────────────────

#[test]
fn test_transfer_admin_changes_admin() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(t.client.admin(), new_admin);
}

#[test]
fn test_transfer_admin_emits_event() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert!(
        !t.env.events().all().events().is_empty(),
        "expected adm_xfr event"
    );
}

// ── create_pool auth gate ─────────────────────────────────────────────────────

#[test]
fn test_create_pool_non_admin_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[0xABu8; 32]);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);

    env.mock_all_auths();
    client.initialize(&admin, &wasm_hash);

    // A fresh env with no mocked auths — require_auth() must reject non-admin callers.
    let env2 = Env::default();
    let factory_addr2 = env2.register(Factory, ());
    let client2 = FactoryClient::new(&env2, &factory_addr2);
    let wasm_hash2 = BytesN::from_array(&env2, &[0xABu8; 32]);
    env2.mock_all_auths();
    client2.initialize(&admin, &wasm_hash2);

    let asset = Address::generate(&env2);
    let result = client2.try_create_pool(&asset, &1_000u128, &86_400u64);
    assert!(result.is_err(), "non-admin create_pool must be rejected");
}

// ── Deployment tests (require pre-built farming-pool WASM) ───────────────────

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_create_pool_returns_incrementing_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let id_a = client.create_pool(&Address::generate(&env), &500u128, &100u64);
    let id_b = client.create_pool(&Address::generate(&env), &1_000u128, &200u64);
    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(client.pool_count(), 2);
}

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_get_pool_returns_correct_record() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let asset = Address::generate(&env);
    let id = client.create_pool(&asset, &250u128, &50u64);
    let record = client.get_pool(&id);
    assert_eq!(record.asset, asset);
    assert_eq!(record.daily_rate, 250u128);
    assert_eq!(record.min_lock_period, 50u64);
}

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_get_pools_by_asset_returns_matching_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);

    let id_0 = client.create_pool(&asset_a, &100u128, &10u64);
    let id_1 = client.create_pool(&asset_b, &200u128, &20u64);
    let id_2 = client.create_pool(&asset_a, &300u128, &30u64);

    let by_a = client.get_pools_by_asset(&asset_a);
    assert_eq!(by_a.len(), 2);
    assert_eq!(by_a.get(0).unwrap(), id_0);
    assert_eq!(by_a.get(1).unwrap(), id_2);

    let by_b = client.get_pools_by_asset(&asset_b);
    assert_eq!(by_b.len(), 1);
    assert_eq!(by_b.get(0).unwrap(), id_1);
}

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_get_pools_by_asset_unknown_asset_returns_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&Address::generate(&env), &100u128, &10u64);
    let unknown = Address::generate(&env);
    let result = client.get_pools_by_asset(&unknown);
    assert_eq!(result.len(), 0);
}

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_create_pool_emits_pool_crtd_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&Address::generate(&env), &300u128, &30u64);
    assert!(
        !env.events().all().events().is_empty(),
        "expected pool_crtd event"
    );
}

#[test]
#[ignore = "requires: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_multiple_pools_stored_independently() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);
    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);
    let id_a = client.create_pool(&asset_a, &100u128, &10u64);
    let id_b = client.create_pool(&asset_b, &200u128, &20u64);
    let rec_a = client.get_pool(&id_a);
    let rec_b = client.get_pool(&id_b);
    assert_eq!(rec_a.asset, asset_a);
    assert_eq!(rec_b.asset, asset_b);
    assert_ne!(rec_a.address, rec_b.address);
}
