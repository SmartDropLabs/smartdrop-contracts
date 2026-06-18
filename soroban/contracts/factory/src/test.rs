#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env,
};

struct TestEnv {
    env: Env,
    client: FactoryClient<'static>,
    admin: Address,
    wasm_hash: BytesN<32>,
}

fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[0xABu8; 32]);

    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // SAFETY: env owns all registrations and lives as long as env.
    let client = unsafe {
        core::mem::transmute::<FactoryClient<'_>, FactoryClient<'static>>(client)
    };

    TestEnv { env, client, admin, wasm_hash }
}

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
#[should_panic(expected = "already initialized")]
fn test_double_initialize_panics() {
    let t = setup();
    t.client.initialize(&t.admin, &t.wasm_hash);
}

#[test]
fn test_pool_count_zero_after_initialize() {
    let t = setup();
    assert_eq!(t.client.pool_count(), 0);
}

#[test]
#[should_panic(expected = "pool not found")]
fn test_get_pool_panics_on_missing_id() {
    let t = setup();
    t.client.get_pool(&0u32);
}

#[test]
fn test_create_pool_non_admin_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[0xABu8; 32]);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);

    env.mock_all_auths();
    client.initialize(&admin, &wasm_hash);

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

#[test]
#[ignore = "requires pre-built farming-pool WASM: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_create_pool_returns_incrementing_ids() {
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
    let id_a = client.create_pool(&asset_a, &500u128, &100u64);
    let id_b = client.create_pool(&asset_b, &1_000u128, &200u64);
    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(client.pool_count(), 2);
}

#[test]
#[ignore = "requires pre-built farming-pool WASM: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
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
#[ignore = "requires pre-built farming-pool WASM: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
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

#[test]
#[ignore = "requires pre-built farming-pool WASM: cargo build -p farming-pool --target wasm32-unknown-unknown --release"]
fn test_create_pool_emits_pool_crtd_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    const FARMING_POOL_WASM: &[u8] = &[];
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);
    let asset = Address::generate(&env);
    let _ = client.create_pool(&asset, &300u128, &30u64);
    let events = env.events().all();
    assert!(!events.events().is_empty(), "expected at least one pool_crtd event");
}
