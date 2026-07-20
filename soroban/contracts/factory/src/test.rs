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

/// Real farming-pool WASM, built by `make test` / CI before `cargo test` runs
/// (see the Makefile and .github/workflows/ci.yml). Lets `create_pool`'s
/// tests exercise the actual cross-contract `initialize` call rather than a
/// stub, per #80's acceptance criteria.
mod farming_pool_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/farming_pool.wasm");
}

/// A distinct, valid contract WASM used to prove that `upgrade_pool` changes
/// the registered pool's executable hash rather than merely re-installing its
/// current farming-pool WASM.
mod replacement_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/factory.wasm");
}

fn upload_farming_pool_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(farming_pool_wasm::WASM)
}

fn upload_replacement_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(replacement_wasm::WASM)
}

/// Builds an initialised factory using the real farming-pool WASM.
fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);

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
                credit_rate: 100 + pool_id as i128,
                global_multiplier: 1 + pool_id,
                min_lock_period: 10 + pool_id,
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
            .map(|record| (record.0, record.1.credit_rate)),
        Some((0, 100))
    );
    assert_eq!(
        page.records
            .get(9)
            .map(|record| (record.0, record.1.credit_rate)),
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
    let page = t.client.get_pools_by_asset(&asset, &0u32, &10u32);
    assert_eq!(page.records.len(), 0, "expected no pools for a fresh factory");
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

// ── upgrade_pool ──────────────────────────────────────────────────────────────

#[test]
fn test_upgrade_pool_hot_swaps_registered_pool_without_changing_factory_hash() {
    let t = setup();
    let original_factory_hash = t.client.pool_wasm_hash();
    let pool_id = t
        .client
        .create_pool(&Address::generate(&t.env), &1_728_000u128, &2u32, &10u64);
    let pool_addr = t.client.get_pool(&pool_id).address;

    let new_wasm_hash = upload_replacement_wasm(&t.env);
    t.client.upgrade_pool(&pool_id, &new_wasm_hash);

    assert_eq!(
        t.env.events().all(),
        vec![
            &t.env,
            (
                pool_addr.clone(),
                vec![
                    &t.env,
                    symbol_short!("pool").into_val(&t.env),
                    symbol_short!("upgraded").into_val(&t.env),
                ],
                new_wasm_hash.clone().into_val(&t.env),
            ),
            (
                t.factory_addr.clone(),
                vec![
                    &t.env,
                    symbol_short!("factory").into_val(&t.env),
                    symbol_short!("pool_upg").into_val(&t.env),
                ],
                (pool_id, pool_addr.clone(), new_wasm_hash.clone()).into_val(&t.env),
            )
        ]
    );

    assert_eq!(
        t.client.pool_wasm_hash(),
        original_factory_hash,
        "pool-by-pool upgrades must not replace the factory default hash"
    );
    assert_eq!(
        pool_record_ttl(&t.env, &t.factory_addr, pool_id),
        TTL_EXTEND_TO
    );
    let record = t.client.get_pool(&pool_id);
    assert_eq!(record.address, pool_addr);
}

#[test]
fn test_upgrade_pool_missing_pool_returns_not_found() {
    let t = setup();
    let new_wasm_hash = t.wasm_hash.clone();
    assert_eq!(
        t.client.try_upgrade_pool(&0u32, &new_wasm_hash),
        Err(Ok(FactoryError::PoolNotFound))
    );
}

#[test]
fn test_upgrade_pool_requires_factory_admin_auth() {
    let t = setup();
    let pool_id = t
        .client
        .create_pool(&Address::generate(&t.env), &1_728_000u128, &2u32, &10u64);

    let not_admin = Address::generate(&t.env);
    let new_wasm_hash = t.wasm_hash.clone();
    let args = (&pool_id, &new_wasm_hash).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "upgrade_pool",
        args,
        sub_invokes: &[],
    };
    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &not_admin,
            invoke: &invoke,
        }])
        .try_upgrade_pool(&pool_id, &new_wasm_hash);

    assert!(result.is_err(), "only the factory admin may upgrade pools");
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
    let result = client2.try_create_pool(&asset, &17_280_000u128, &2u32, &86_400u64);
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
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let id_a = client.create_pool(&Address::generate(&env), &8_640_000u128, &2u32, &100u64);
    let id_b = client.create_pool(&Address::generate(&env), &17_280_000u128, &3u32, &200u64);
    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(client.pool_count(), 2);
}

#[test]
fn test_get_pool_returns_correct_record() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let asset = Address::generate(&env);
    let id = client.create_pool(&asset, &4_320_000u128, &2u32, &50u64);
    let record = client.get_pool(&id);
    assert_eq!(record.asset, asset);
    assert_eq!(record.credit_rate, 250);
    assert_eq!(record.global_multiplier, 2);
    assert_eq!(record.min_lock_period, 50u32);
}

#[test]
fn test_get_pools_by_asset_returns_matching_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);

    let id_0 = client.create_pool(&asset_a, &1_728_000u128, &2u32, &10u64);
    let id_1 = client.create_pool(&asset_b, &3_456_000u128, &2u32, &20u64);
    let id_2 = client.create_pool(&asset_a, &5_184_000u128, &2u32, &30u64);

    let by_a = client.get_pools_by_asset(&asset_a, &0u32, &10u32);
    assert_eq!(by_a.records.len(), 2);
    assert_eq!(by_a.records.get(0).map(|r| r.0), Some(id_0));
    assert_eq!(by_a.records.get(1).map(|r| r.0), Some(id_2));

    let by_b = client.get_pools_by_asset(&asset_b, &0u32, &10u32);
    assert_eq!(by_b.records.len(), 1);
    assert_eq!(by_b.records.get(0).map(|r| r.0), Some(id_1));
}

#[test]
fn test_get_pools_by_asset_unknown_asset_returns_empty() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&Address::generate(&env), &1_728_000u128, &2u32, &10u64);
    let unknown = Address::generate(&env);
    let result = client.get_pools_by_asset(&unknown, &0u32, &10u32);
    assert_eq!(result.records.len(), 0);
}

#[test]
fn test_get_pools_by_asset_paginates_large_matching_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_mock_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // Create 25 pools all sharing the same asset
    let asset = Address::generate(&env);
    for i in 0..25 {
        client.create_pool(&asset, &(100 + i as u128), &(10 + i as u64));
    }

    // First page should return 20 records
    let page1 = client.get_pools_by_asset(&asset, &0u32, &20u32);
    assert_eq!(page1.records.len(), 20);
    assert_eq!(page1.records.get(0).map(|r| r.0), Some(0));
    assert_eq!(page1.records.get(19).map(|r| r.0), Some(19));
    assert_eq!(page1.next_start_id, 20);
    assert_eq!(page1.total, 25);

    // Second page should return remaining 5 records
    let page2 = client.get_pools_by_asset(&asset, &page1.next_start_id, &20u32);
    assert_eq!(page2.records.len(), 5);
    assert_eq!(page2.records.get(0).map(|r| r.0), Some(20));
    assert_eq!(page2.records.get(4).map(|r| r.0), Some(24));
    assert_eq!(page2.next_start_id, 25);
    assert_eq!(page2.total, 25);

    // Third page should be empty
    let page3 = client.get_pools_by_asset(&asset, &page2.next_start_id, &20u32);
    assert_eq!(page3.records.len(), 0);
    assert_eq!(page3.next_start_id, 25);
    assert_eq!(page3.total, 25);
}

#[test]
fn test_create_pool_emits_pool_crtd_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&Address::generate(&env), &5_184_000u128, &2u32, &30u64);
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
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);
    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);
    let id_a = client.create_pool(&asset_a, &1_728_000u128, &2u32, &10u64);
    let id_b = client.create_pool(&asset_b, &3_456_000u128, &2u32, &20u64);
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
    let args = (&asset, 17_280_000u128, 2u32, 86_400u64).into_val(&t.env);
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
        .try_create_pool(&asset, &17_280_000u128, &2u32, &86_400u64);

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
        .create_pool(&Address::generate(&t.env), &8_640_000u128, &2u32, &100u64);
    assert_eq!(id_a, 0);
    assert_eq!(t.client.pool_count(), 1);

    let id_b = t
        .client
        .create_pool(&Address::generate(&t.env), &17_280_000u128, &2u32, &200u64);
    assert_eq!(id_b, 1);
    assert_eq!(t.client.pool_count(), 2);
}

#[test]
fn test_create_pool_uses_deterministic_pool_addresses() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let expected_before = expected_pool_address(&t.env, &t.factory_addr, 0);
    let expected_again = expected_pool_address(&t.env, &t.factory_addr, 0);

    let id = t.client.create_pool(&asset, &4_320_000u128, &2u32, &50u64);
    let record = t.client.get_pool(&id);

    assert_eq!(id, 0);
    assert_eq!(expected_before, expected_again);
    assert_eq!(record.address, expected_before);
}

#[test]
fn test_create_pool_rejects_zero_daily_rate() {
    let t = setup();
    let asset = Address::generate(&t.env);

    // Below LEDGERS_PER_DAY (17_280), the daily_rate -> credit_rate conversion
    // truncates to zero, which FarmingPool::initialize would reject anyway
    // (credit_rate must be > 0) — create_pool must catch this itself rather
    // than deploying a pool that can never be initialized.
    let result = t.client.try_create_pool(&asset, &0u128, &2u32, &25u64);
    assert_eq!(result, Err(Ok(FactoryError::InvalidCreditRate)));
    assert_eq!(t.client.pool_count(), 0);
}

#[test]
fn test_create_pool_rejects_daily_rate_below_ledgers_per_day() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let result = t.client.try_create_pool(&asset, &17_279u128, &2u32, &25u64);
    assert_eq!(result, Err(Ok(FactoryError::InvalidCreditRate)));
}

#[test]
fn test_create_pool_rejects_global_multiplier_below_one() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &0u32, &25u64);
    assert_eq!(result, Err(Ok(FactoryError::InvalidGlobalMultiplier)));
}

#[test]
fn test_create_pool_rejects_min_lock_period_out_of_u32_range() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let too_large = (u32::MAX as u64) + 1;
    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &2u32, &too_large);
    assert_eq!(result, Err(Ok(FactoryError::MinLockPeriodOutOfRange)));
}

#[test]
fn test_get_pool_bumps_pool_record_ttl() {
    let t = setup();
    let id = t
        .client
        .create_pool(&Address::generate(&t.env), &4_320_000u128, &2u32, &50u64);

    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    advance_ledgers(&t.env, TTL_EXTEND_TO - TTL_THRESHOLD + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    assert_eq!(t.client.try_get_pool(&id).is_ok(), true);
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_refresh_pool_ttls_restores_ttl_for_unqueried_pool() {
    let t = setup();
    let id = t
        .client
        .create_pool(&Address::generate(&t.env), &250u128, &50u64);

    // Initial TTL after creation
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    // Advance ledgers past TTL_EXTEND_TO without ever calling get_pool on this pool
    advance_ledgers(&t.env, TTL_EXTEND_TO + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    // Call refresh_pool_ttls to restore TTL without a specific get_pool query
    assert_eq!(
        t.client.try_refresh_pool_ttls(&id, &1u32),
        Ok(Ok(()))
    );

    // Verify TTL is restored
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_create_pool_emits_pool_crtd_event_with_payload() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let expected_address = expected_pool_address(&t.env, &t.factory_addr, 0);
    let id = t.client.create_pool(&asset, &5_184_000u128, &2u32, &30u64);

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
                (id, expected_address, asset, 300i128, 2u32, 30u32).into_val(&t.env),
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
    let old_args = (&old_asset, 1_728_000u128, 2u32, 10u64).into_val(&t.env);
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
        .try_create_pool(&old_asset, &1_728_000u128, &2u32, &10u64);

    assert!(
        old_result.is_err(),
        "old admin must not authorize new pool creation"
    );
    assert_eq!(t.client.pool_count(), 0);

    let new_asset = Address::generate(&t.env);
    let new_args = (&new_asset, 3_456_000u128, 2u32, 20u64).into_val(&t.env);
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
        .create_pool(&new_asset, &3_456_000u128, &2u32, &20u64);

    assert_eq!(new_id, 0);
    assert_eq!(t.client.pool_count(), 1);
}

// ── create_pool ↔ deployed pool consistency (#80 acceptance criterion) ───────

#[test]
fn test_create_pool_configures_deployed_pool_matching_factory_record() {
    let t = setup();
    let asset = Address::generate(&t.env);

    let id = t
        .client
        .create_pool(&asset, &17_280_000u128, &3u32, &86_400u64);
    let record = t.client.get_pool(&id);

    assert_eq!(record.credit_rate, 1_000);
    assert_eq!(record.global_multiplier, 3);
    assert_eq!(record.min_lock_period, 86_400);

    let pool_client = farming_pool_wasm::Client::new(&t.env, &record.address);
    assert_eq!(pool_client.admin(), t.admin);
    assert_eq!(pool_client.credit_rate(), record.credit_rate);
    assert_eq!(pool_client.min_lock_period(), record.min_lock_period);

    // No direct global_multiplier getter exists on FarmingPool; observe it
    // indirectly via get_boost_config, which echoes the pool's stored
    // multiplier alongside a user's allocation once they've opted into boost.
    let user = Address::generate(&t.env);
    pool_client.set_boost(&user, &50u32);
    let boost = pool_client.get_boost_config(&user).unwrap();
    assert_eq!(boost.multiplier, record.global_multiplier);
}

#[test]
fn test_create_pool_admin_matches_factory_admin_at_creation_time() {
    let t = setup();
    let asset = Address::generate(&t.env);
    let id = t.client.create_pool(&asset, &17_280_000u128, &2u32, &10u64);
    let record = t.client.get_pool(&id);

    let pool_client = farming_pool_wasm::Client::new(&t.env, &record.address);
    assert_eq!(pool_client.admin(), t.admin);

    // Rotating the factory's admin does not retroactively change an
    // already-deployed pool's admin — documented tradeoff from #80.
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(pool_client.admin(), t.admin);
    assert_ne!(pool_client.admin(), new_admin);
}

// ── set_pool_wasm_hash ────────────────────────────────────────────────────────

#[test]
fn test_set_pool_wasm_hash_requires_admin_auth() {
    let t = setup();
    let new_hash = BytesN::from_array(&t.env, &[0xABu8; 32]);
    let not_admin = Address::generate(&t.env);
    let args = (&new_hash,).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "set_pool_wasm_hash",
        args,
        sub_invokes: &[],
    };
    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &not_admin,
            invoke: &invoke,
        }])
        .try_set_pool_wasm_hash(&new_hash);

    assert!(
        result.is_err(),
        "only the current admin may set pool wasm hash"
    );
    assert_eq!(t.client.pool_wasm_hash(), t.wasm_hash);
}

#[test]
fn test_set_pool_wasm_hash_updates_future_pools() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // Create a pool with the original WASM hash
    let asset_a = Address::generate(&env);
    let id_a = client.create_pool(&asset_a, &8_640_000u128, &2u32, &100u64);
    assert_eq!(id_a, 0);
    assert_eq!(client.pool_wasm_hash(), wasm_hash);

    // Change to a bogus hash that doesn't correspond to any uploaded WASM
    let bogus_hash = BytesN::from_array(&env, &[0xABu8; 32]);
    assert!(client.try_set_pool_wasm_hash(&bogus_hash).is_ok());
    assert_eq!(client.pool_wasm_hash(), bogus_hash);

    // Future pools should fail to deploy because the bogus hash has no WASM
    let asset_b = Address::generate(&env);
    let result = client.try_create_pool(&asset_b, &8_640_000u128, &2u32, &100u64);
    assert!(
        result.is_err(),
        "create_pool must fail when WASM hash doesn't correspond to uploaded code"
    );

    // Pool 0 is unaffected
    let record = client.get_pool(&id_a);
    assert_eq!(record.asset, asset_a);
}

#[test]
fn test_set_pool_wasm_hash_does_not_affect_already_deployed_pools() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // Create pool 0
    let asset_a = Address::generate(&env);
    let id_a = client.create_pool(&asset_a, &8_640_000u128, &2u32, &100u64);
    let record_a = client.get_pool(&id_a);

    // Change the WASM hash
    let bogus_hash = BytesN::from_array(&env, &[0xCDu8; 32]);
    client.set_pool_wasm_hash(&bogus_hash);
    assert_eq!(client.pool_wasm_hash(), bogus_hash);

    // Existing pool is unaffected
    let record_a_again = client.get_pool(&id_a);
    assert_eq!(record_a_again, record_a);
    assert_eq!(record_a_again.asset, asset_a);
    assert_eq!(record_a_again.credit_rate, 500);
    assert_eq!(record_a_again.global_multiplier, 2);
    assert_eq!(record_a_again.min_lock_period, 100);

    // Deployed pool contract is still functional
    let pool_client = farming_pool_wasm::Client::new(&env, &record_a.address);
    assert_eq!(pool_client.admin(), admin);
}

#[test]
fn test_set_pool_wasm_hash_emits_event_with_old_and_new_hash() {
    let t = setup();
    let new_hash = upload_farming_pool_wasm(&t.env);
    t.client.set_pool_wasm_hash(&new_hash);

    assert_eq!(
        t.env.events().all(),
        vec![
            &t.env,
            (
                t.factory_addr.clone(),
                vec![
                    &t.env,
                    symbol_short!("factory").into_val(&t.env),
                    symbol_short!("wasm_set").into_val(&t.env),
                ],
                (t.wasm_hash.clone(), new_hash.clone()).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_set_pool_wasm_hash_rejects_uninitialized_factory() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    let hash = BytesN::from_array(&env, &[0u8; 32]);

    let result = client.try_set_pool_wasm_hash(&hash);
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}
