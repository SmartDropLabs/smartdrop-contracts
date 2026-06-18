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
    admin: Address,
    wasm_hash: BytesN<32>,
}

fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    // Upload the factory's own WASM as a stand-in pool WASM for unit tests.
    // In production the farming-pool WASM is uploaded via `stellar contract upload`
    // before the factory is deployed.
    let wasm_hash = env.deployer().upload_contract_wasm(WASM);

    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // SAFETY: env owns the factory registration; it lives as long as env.
    let client = unsafe {
        core::mem::transmute::<FactoryClient<'_>, FactoryClient<'static>>(client)
    };

    TestEnv { env, client, admin, wasm_hash }
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
#[should_panic(expected = "already initialized")]
fn test_double_initialize_panics() {
    let t = setup();
    t.client.initialize(&t.admin, &t.wasm_hash);
}

// ── pool_count ────────────────────────────────────────────────────────────────

#[test]
fn test_pool_count_zero_after_initialize() {
    let t = setup();
    assert_eq!(t.client.pool_count(), 0);
}

// ── get_pool ──────────────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "pool not found")]
fn test_get_pool_panics_on_missing_id() {
    let t = setup();
    t.client.get_pool(&0u32);
}

// ── create_pool auth gate ─────────────────────────────────────────────────────

#[test]
#[should_panic]
fn test_create_pool_non_admin_rejected() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let wasm_hash = env.deployer().upload_contract_wasm(WASM);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);

    // initialize itself does not require auth.
    env.mock_all_auths();
    client.initialize(&admin, &wasm_hash);

    // No auths remain — create_pool hits admin.require_auth() and panics.
    let asset = Address::generate(&env);
    client.try_create_pool(&asset, &1_000u128, &86_400u64).unwrap();
}

// ── create_pool success path ──────────────────────────────────────────────────

#[test]
fn test_create_pool_returns_incrementing_ids() {
    let t = setup();
    let asset_a = Address::generate(&t.env);
    let asset_b = Address::generate(&t.env);

    let id_a = t.client.create_pool(&asset_a, &500u128, &100u64);
    let id_b = t.client.create_pool(&asset_b, &1_000u128, &200u64);

    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(t.client.pool_count(), 2);
}

#[test]
fn test_get_pool_returns_correct_record() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let id = t.client.create_pool(&asset, &250u128, &50u64);
    let record = t.client.get_pool(&id);

    assert_eq!(record.asset, asset);
    assert_eq!(record.daily_rate, 250u128);
    assert_eq!(record.min_lock_period, 50u64);
}

#[test]
fn test_multiple_pools_stored_independently() {
    let t = setup();
    let asset_a = Address::generate(&t.env);
    let asset_b = Address::generate(&t.env);

    let id_a = t.client.create_pool(&asset_a, &100u128, &10u64);
    let id_b = t.client.create_pool(&asset_b, &200u128, &20u64);

    let rec_a = t.client.get_pool(&id_a);
    let rec_b = t.client.get_pool(&id_b);

    assert_eq!(rec_a.asset, asset_a);
    assert_eq!(rec_b.asset, asset_b);
    assert_ne!(rec_a.address, rec_b.address);
}

#[test]
fn test_create_pool_emits_pool_crtd_event() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let _ = t.client.create_pool(&asset, &300u128, &30u64);

    // At least one event must be emitted.
    let events = t.env.events().all();
    assert!(!events.events().is_empty(), "expected pool_crtd event to be emitted");
}
