#![cfg(test)]

use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{
        storage::Persistent as _, Address as _, AuthorizedFunction, AuthorizedInvocation,
        Events as _, Ledger, MockAuth, MockAuthInvoke,
    },
    vec, Address, BytesN, Env, IntoVal, Symbol,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: FactoryClient<'static>,
    factory_addr: Address,
    admin: Address,
    wasm_hash: BytesN<32>,
}

// Real, compiled farming-pool WASM. `create_pool` (#79) now calls the
// deployed pool's own `initialize` in the same invocation, so a synthetic
// stand-in WASM (previously used here) no longer works — it doesn't
// implement farming-pool's actual contract interface. This is the same
// fixture `factory/tests/factory_pool_integration.rs` uses; see that file's
// module doc comment for how it's built and kept fresh.
const FARMING_POOL_WASM: &[u8] = include_bytes!("../tests/fixtures/farming_pool.wasm");

fn upload_mock_pool_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(FARMING_POOL_WASM)
}

/// Builds an initialised factory with a small deployable mock WASM hash.
fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);

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

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = current + by;
    });
}

fn expected_pool_address(env: &Env, factory_addr: &Address, pool_id: u32) -> Address {
    env.as_contract(factory_addr, || {
        env.deployer()
            .with_current_contract(pool_salt(env, pool_id))
            .deployed_address()
    })
}

fn pool_record_ttl(env: &Env, factory_addr: &Address, pool_id: u32) -> u32 {
    env.as_contract(factory_addr, || {
        env.storage().persistent().get_ttl(&DataKey::Pool(pool_id))
    })
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
fn test_double_initialize_returns_error() {
    let t = setup();
    assert_eq!(
        t.client.try_initialize(&t.admin, &t.wasm_hash),
        Err(Ok(FactoryError::AlreadyInitialized))
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
    assert_eq!(
        t.client.try_get_pool(&0u32),
        Err(Ok(FactoryError::PoolNotFound))
    );
}

#[test]
fn test_list_pools_returns_first_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&0u32, &10u32);

    assert_eq!(page.records.len(), 10);
    assert_eq!(page.next_start_id, 10);
    assert_eq!(page.total, 25);

    assert_eq!(
        page.records
            .get(0)
            .map(|record| (record.0, record.1.daily_rate)),
        Some((0, 100))
    );
    assert_eq!(
        page.records
            .get(9)
            .map(|record| (record.0, record.1.daily_rate)),
        Some((9, 109))
    );
}

#[test]
fn test_list_pools_returns_second_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&10u32, &10u32);

    assert_eq!(page.records.len(), 10);
    assert_eq!(page.next_start_id, 20);
    assert_eq!(page.total, 25);
    assert_eq!(page.records.get(0).map(|record| record.0), Some(10));
    assert_eq!(page.records.get(9).map(|record| record.0), Some(19));
}

#[test]
fn test_list_pools_returns_partial_last_page() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&20u32, &10u32);

    assert_eq!(page.records.len(), 5);
    assert_eq!(page.next_start_id, 25);
    assert_eq!(page.total, 25);
    assert_eq!(page.records.get(0).map(|record| record.0), Some(20));
    assert_eq!(page.records.get(4).map(|record| record.0), Some(24));
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
    assert_eq!(page.records.get(19).map(|record| record.0), Some(19));
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

    assert_eq!(
        t.env.auths(),
        [(
            t.admin.clone(),
            AuthorizedInvocation {
                function: AuthorizedFunction::Contract((
                    t.factory_addr.clone(),
                    Symbol::new(&t.env, "transfer_admin"),
                    (&new_admin,).into_val(&t.env),
                )),
                sub_invocations: [].into(),
            },
        )]
    );
    assert_eq!(t.client.admin(), new_admin);
}

#[test]
fn test_transfer_admin_non_admin_rejected() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    let not_admin = Address::generate(&t.env);
    let args = (&new_admin,).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "transfer_admin",
        args,
        sub_invokes: &[],
    };
    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &not_admin,
            invoke: &invoke,
        }])
        .try_transfer_admin(&new_admin);

    assert!(
        result.is_err(),
        "only the current admin may transfer admin rights"
    );
    assert_eq!(t.client.admin(), t.admin);
}

#[test]
fn test_transfer_admin_emits_event_with_old_and_new_admin() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);

    assert_eq!(
        t.env.events().all(),
        vec![
            &t.env,
            (
                t.factory_addr.clone(),
                vec![
                    &t.env,
                    symbol_short!("factory").into_val(&t.env),
                    symbol_short!("adm_xfr").into_val(&t.env),
                ],
                (t.admin.clone(), new_admin.clone()).into_val(&t.env),
            )
        ]
    );
}

// ── create_pool auth gate ─────────────────────────────────────────────────────

#[test]
fn test_create_pool_rejects_missing_pool_wasm_hash() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let wasm_hash = BytesN::from_array(&env, &[0xABu8; 32]);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);

    env.mock_all_auths();
    client.initialize(&admin, &wasm_hash);

    // This factory points at a hash that was never uploaded, so deployment must fail.
    let env2 = Env::default();
    let factory_addr2 = env2.register(Factory, ());
    let client2 = FactoryClient::new(&env2, &factory_addr2);
    let wasm_hash2 = BytesN::from_array(&env2, &[0xABu8; 32]);
    env2.mock_all_auths();
    client2.initialize(&admin, &wasm_hash2);

    let asset = Address::generate(&env2);
    let result = client2.try_create_pool(&asset, &1_000u128, &86_400u64);
    assert!(
        result.is_err(),
        "create_pool must reject an unknown pool WASM hash"
    );
}

// ── Deployment tests (require pre-built farming-pool WASM) ───────────────────

#[test]
fn test_create_pool_returns_incrementing_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
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
fn test_get_pool_returns_correct_record() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
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
fn test_get_pools_by_asset_returns_matching_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
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
    assert_eq!(by_a.get(0), Some(id_0));
    assert_eq!(by_a.get(1), Some(id_2));

    let by_b = client.get_pools_by_asset(&asset_b);
    assert_eq!(by_b.len(), 1);
    assert_eq!(by_b.get(0), Some(id_1));
}

#[test]
fn test_get_pools_by_asset_unknown_asset_returns_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&Address::generate(&env), &100u128, &10u64);
    let unknown = Address::generate(&env);
    let result = client.get_pools_by_asset(&unknown);
    assert_eq!(result.len(), 0);
}

#[test]
fn test_create_pool_emits_pool_crtd_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
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
fn test_multiple_pools_stored_independently() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
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
fn test_create_pool_rejects_unmatched_non_admin_auth() {
    let t = setup();
    let not_admin = Address::generate(&t.env);
    let asset = Address::generate(&t.env);
    let args = (&asset, 1_000u128, 86_400u64).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "create_pool",
        args,
        sub_invokes: &[],
    };
    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &not_admin,
            invoke: &invoke,
        }])
        .try_create_pool(&asset, &1_000u128, &86_400u64);

    assert!(
        result.is_err(),
        "create_pool must require the stored admin auth"
    );
    assert_eq!(t.client.pool_count(), 0);
}

#[test]
fn test_create_pool_increments_count_after_each_pool() {
    let t = setup();

    assert_eq!(t.client.pool_count(), 0);
    let id_a = t
        .client
        .create_pool(&Address::generate(&t.env), &500u128, &100u64);
    assert_eq!(id_a, 0);
    assert_eq!(t.client.pool_count(), 1);

    let id_b = t
        .client
        .create_pool(&Address::generate(&t.env), &1_000u128, &200u64);
    assert_eq!(id_b, 1);
    assert_eq!(t.client.pool_count(), 2);
}

#[test]
fn test_create_pool_uses_deterministic_pool_addresses() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let expected_before = expected_pool_address(&t.env, &t.factory_addr, 0);
    let expected_again = expected_pool_address(&t.env, &t.factory_addr, 0);

    let id = t.client.create_pool(&asset, &250u128, &50u64);
    let record = t.client.get_pool(&id);

    assert_eq!(id, 0);
    assert_eq!(expected_before, expected_again);
    assert_eq!(record.address, expected_before);
}

#[test]
fn test_create_pool_records_zero_daily_rate() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let id = t.client.create_pool(&asset, &0u128, &25u64);
    let record = t.client.get_pool(&id);

    assert_eq!(record.daily_rate, 0);
    assert_eq!(record.asset, asset);
    assert_eq!(record.min_lock_period, 25);
}

#[test]
fn test_get_pool_bumps_pool_record_ttl() {
    let t = setup();
    let id = t
        .client
        .create_pool(&Address::generate(&t.env), &250u128, &50u64);

    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    advance_ledgers(&t.env, TTL_EXTEND_TO - TTL_THRESHOLD + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    assert_eq!(t.client.try_get_pool(&id).is_ok(), true);
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_create_pool_emits_pool_crtd_event_with_payload() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let expected_address = expected_pool_address(&t.env, &t.factory_addr, 0);
    let id = t.client.create_pool(&asset, &300u128, &30u64);

    assert_eq!(
        t.env.events().all(),
        vec![
            &t.env,
            (
                t.factory_addr.clone(),
                vec![
                    &t.env,
                    symbol_short!("factory").into_val(&t.env),
                    symbol_short!("pool_crtd").into_val(&t.env),
                ],
                (id, expected_address, asset, 300u128, 30u64).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_old_admin_cannot_create_pool_after_transfer_but_new_admin_can() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);

    let old_asset = Address::generate(&t.env);
    let old_args = (&old_asset, 100u128, 10u64).into_val(&t.env);
    let old_invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "create_pool",
        args: old_args,
        sub_invokes: &[],
    };
    let old_result = t
        .client
        .mock_auths(&[MockAuth {
            address: &t.admin,
            invoke: &old_invoke,
        }])
        .try_create_pool(&old_asset, &100u128, &10u64);

    assert!(
        old_result.is_err(),
        "old admin must not authorize new pool creation"
    );
    assert_eq!(t.client.pool_count(), 0);

    let new_asset = Address::generate(&t.env);
    let new_args = (&new_asset, 200u128, 20u64).into_val(&t.env);
    let new_invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "create_pool",
        args: new_args,
        sub_invokes: &[],
    };
    let new_id = t
        .client
        .mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &new_invoke,
        }])
        .create_pool(&new_asset, &200u128, &20u64);

    assert_eq!(new_id, 0);
    assert_eq!(t.client.pool_count(), 1);
}
