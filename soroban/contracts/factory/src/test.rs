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

use farming_pool::FarmingPoolClient;

// ── Helpers ───────────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: FactoryClient<'static>,
    factory_addr: Address,
    admin: Address,
    wasm_hash: BytesN<32>,
    asset: Address,
}

/// Real farming-pool WASM, built by `make test` / CI before `cargo test` runs
/// (see the Makefile and .github/workflows/ci.yml). Lets `create_pool`'s
/// tests exercise the actual cross-contract `initialize` call rather than a
/// stub, per #80's acceptance criteria.
mod farming_pool_wasm {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/farming_pool.wasm");
}

const ADD_I32_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x10, 0x03, 0x60, 0x02, 0x7e, 0x7e, 0x01,
    0x7e, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e, 0x60, 0x00, 0x00, 0x02, 0x0d, 0x02, 0x01, 0x78, 0x01,
    0x31, 0x00, 0x00, 0x01, 0x76, 0x01, 0x67, 0x00, 0x00, 0x03, 0x04, 0x03, 0x00, 0x01, 0x02, 0x05,
    0x03, 0x01, 0x00, 0x10, 0x06, 0x19, 0x03, 0x7f, 0x01, 0x41, 0x80, 0x80, 0xc0, 0x00, 0x0b, 0x7f,
    0x00, 0x41, 0x80, 0x80, 0xc0, 0x00, 0x0b, 0x7f, 0x00, 0x41, 0x80, 0x80, 0xc0, 0x00, 0x0b, 0x07,
    0x2f, 0x05, 0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00, 0x03, 0x61, 0x64, 0x64, 0x00,
    0x02, 0x01, 0x5f, 0x00, 0x04, 0x0a, 0x5f, 0x5f, 0x64, 0x61, 0x74, 0x61, 0x5f, 0x65, 0x6e, 0x64,
    0x03, 0x01, 0x0b, 0x5f, 0x5f, 0x68, 0x65, 0x61, 0x70, 0x5f, 0x62, 0x61, 0x73, 0x65, 0x03, 0x02,
    0x0a, 0xe3, 0x01, 0x03, 0xc5, 0x01, 0x02, 0x04, 0x7f, 0x01, 0x7e, 0x23, 0x00, 0x41, 0x20, 0x6b,
    0x22, 0x03, 0x24, 0x00, 0x02, 0x40, 0x20, 0x00, 0x42, 0xff, 0x01, 0x83, 0x42, 0x05, 0x52, 0x20,
    0x01, 0x42, 0xff, 0x01, 0x83, 0x42, 0x05, 0x52, 0x72, 0x45, 0x04, 0x40, 0x20, 0x00, 0x42, 0x20,
    0x88, 0xa7, 0x21, 0x04, 0x20, 0x01, 0x42, 0x20, 0x88, 0xa7, 0x21, 0x05, 0x20, 0x03, 0x42, 0x8e,
    0xd2, 0xa9, 0x13, 0x37, 0x03, 0x08, 0x42, 0x02, 0x21, 0x06, 0x41, 0x01, 0x21, 0x02, 0x03, 0x40,
    0x20, 0x02, 0x04, 0x40, 0x20, 0x02, 0x41, 0x01, 0x6b, 0x21, 0x02, 0x42, 0x8e, 0xd2, 0xa9, 0x13,
    0x21, 0x06, 0x0c, 0x01, 0x0b, 0x0b, 0x20, 0x03, 0x20, 0x06, 0x37, 0x03, 0x10, 0x20, 0x03, 0x41,
    0x10, 0x6a, 0x22, 0x02, 0x41, 0x01, 0x10, 0x03, 0x20, 0x03, 0x20, 0x01, 0x42, 0x80, 0x80, 0x80,
    0x80, 0x70, 0x83, 0x42, 0x05, 0x84, 0x37, 0x03, 0x18, 0x20, 0x03, 0x20, 0x00, 0x42, 0x80, 0x80,
    0x80, 0x80, 0x70, 0x83, 0x42, 0x05, 0x84, 0x37, 0x03, 0x10, 0x20, 0x02, 0x41, 0x02, 0x10, 0x03,
    0x10, 0x00, 0x1a, 0x20, 0x05, 0x41, 0x00, 0x48, 0x20, 0x04, 0x20, 0x05, 0x6a, 0x22, 0x02, 0x20,
    0x04, 0x48, 0x47, 0x0d, 0x01, 0x20, 0x03, 0x41, 0x20, 0x6a, 0x24, 0x00, 0x20, 0x02, 0xad, 0x42,
    0x20, 0x86, 0x42, 0x05, 0x84, 0x0f, 0x0b, 0x00, 0x0b, 0x00, 0x0b, 0x16, 0x00, 0x20, 0x00, 0xad,
    0x42, 0x20, 0x86, 0x42, 0x04, 0x84, 0x20, 0x01, 0xad, 0x42, 0x20, 0x86, 0x42, 0x04, 0x84, 0x10,
    0x01, 0x0b, 0x03, 0x00, 0x01, 0x0b, 0x00, 0x4b, 0x0e, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x61, 0x63,
    0x74, 0x73, 0x70, 0x65, 0x63, 0x76, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x61, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x62, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x05, 0x00, 0x1e, 0x11, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x61, 0x63, 0x74, 0x65, 0x6e,
    0x76, 0x6d, 0x65, 0x74, 0x61, 0x76, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x73, 0x0e, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x61, 0x63, 0x74, 0x6d, 0x65,
    0x74, 0x61, 0x76, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x72, 0x73, 0x76, 0x65,
    0x72, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x31, 0x2e, 0x37, 0x34, 0x2e, 0x30, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x72, 0x73, 0x73, 0x64, 0x6b, 0x76, 0x65, 0x72,
    0x00, 0x00, 0x00, 0x33, 0x32, 0x30, 0x2e, 0x30, 0x2e, 0x30, 0x2d, 0x72, 0x63, 0x32, 0x23, 0x37,
    0x63, 0x31, 0x35, 0x34, 0x62, 0x34, 0x66, 0x65, 0x36, 0x61, 0x33, 0x37, 0x64, 0x37, 0x63, 0x61,
    0x37, 0x31, 0x37, 0x37, 0x33, 0x34, 0x32, 0x64, 0x65, 0x64, 0x62, 0x36, 0x39, 0x66, 0x33, 0x31,
    0x30, 0x38, 0x30, 0x39, 0x35, 0x65, 0x66, 0x00,
];

fn upload_farming_pool_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(farming_pool_wasm::WASM)
}

fn upload_replacement_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(ADD_I32_WASM)
}

/// Register a Stellar Asset Contract and return its address.
/// Used by tests that need a real token for `create_pool`'s `validate_asset` check.
fn register_asset(env: &Env) -> Address {
    let token_admin = Address::generate(env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    // Mint a trivial amount so the SAC is fully initialized.
    let sac = soroban_sdk::token::StellarAssetClient::new(env, &asset.address());
    sac.mint(&token_admin, &0i128);
    asset.address()
}

/// Builds an initialised factory using the real farming-pool WASM.
fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let asset = register_asset(&env);

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
        asset,
    }
}

/// Builds a registered-but-uninitialised factory: no `initialize` call, so no
/// `Admin`/`WasmHash` instance entries exist. Mirrors farming-pool's
/// `setup_uninitialized` helper.
fn setup_uninitialized() -> (Env, FactoryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);

    let client =
        unsafe { core::mem::transmute::<FactoryClient<'_>, FactoryClient<'static>>(client) };

    (env, client)
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
                daily_rate: (100 + pool_id as u128) * 17280,
                wasm_hash: soroban_sdk::BytesN::from_array(&t.env, &[0; 32]),
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

#[test]
fn test_initialize_rejects_zero_address_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    let zero_admin = Address::from_string(&soroban_sdk::String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    assert_eq!(
        client.try_initialize(&zero_admin, &wasm_hash),
        Err(Ok(FactoryError::InvalidAdmin))
    );
}

// ── NotInitialized guard ──────────────────────────────────────────────────────

#[test]
fn test_admin_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    match client.try_admin() {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_pool_wasm_hash_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    match client.try_pool_wasm_hash() {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_create_pool_uninitialized_returns_not_initialized() {
    let (env, client) = setup_uninitialized();
    let asset = register_asset(&env);
    match client.try_create_pool(&asset, &1_000u128, &2u32, &86_400u64, &0i128) {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_pool_count_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    match client.try_pool_count() {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_get_pool_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    // NotInitialized must win over PoolNotFound: the registry does not exist yet.
    match client.try_get_pool(&0u32) {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_list_pools_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    match client.try_list_pools(&0u32, &10u32) {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_get_pools_by_asset_uninitialized_returns_not_initialized() {
    let (env, client) = setup_uninitialized();
    let asset = register_asset(&env);
    match client.try_get_pools_by_asset(&asset, &0u32, &10u32) {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

#[test]
fn test_transfer_admin_uninitialized_returns_not_initialized() {
    let (env, client) = setup_uninitialized();
    let new_admin = Address::generate(&env);
    match client.try_transfer_admin(&new_admin) {
        Err(Ok(FactoryError::NotInitialized)) => {}
        _ => panic!("expected FactoryError::NotInitialized"),
    }
}

/// The guard must not fire once `initialize` has seeded the state.
#[test]
fn test_guard_passes_after_initialize() {
    let t = setup();
    assert!(t.client.try_admin().is_ok());
    assert!(t.client.try_pool_wasm_hash().is_ok());
    assert!(t.client.try_pool_count().is_ok());
}

// ── pool_wasm_hash ────────────────────────────────────────────────────────────

#[test]
fn test_pool_wasm_hash_returns_stored_hash() {
    let t = setup();
    assert_eq!(t.client.pool_wasm_hash(), t.wasm_hash);
}

#[test]
fn test_initialize_rejects_zero_wasm_hash() {
    let (env, client) = setup_uninitialized();
    let admin = Address::generate(&env);
    let zero_hash = BytesN::from_array(&env, &[0u8; 32]);
    assert_eq!(
        client.try_initialize(&admin, &zero_hash),
        Err(Ok(FactoryError::InvalidWasmHash))
    );
}

#[test]
fn test_set_pool_wasm_hash_rejects_zero_hash() {
    let t = setup();
    let zero_hash = BytesN::from_array(&t.env, &[0u8; 32]);
    assert_eq!(
        t.client.try_set_pool_wasm_hash(&zero_hash),
        Err(Ok(FactoryError::InvalidWasmHash))
    );
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
    assert!(page.has_more);

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
    assert!(page.has_more);
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
    assert!(!page.has_more);
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
    assert!(!page.has_more);
}

#[test]
fn test_list_pools_returns_empty_when_start_equals_count() {
    let t = setup_with_pool_records(3);
    let page = t.client.list_pools(&3u32, &5u32);

    assert_eq!(page.records.len(), 0);
    assert_eq!(page.next_start_id, 3);
    assert_eq!(page.total, 3);
    assert!(!page.has_more);
}

#[test]
fn test_list_pools_caps_limit_at_twenty() {
    let t = setup_with_pool_records(25);
    let page = t.client.list_pools(&0u32, &100u32);

    assert_eq!(page.records.len(), 20);
    assert_eq!(page.next_start_id, 20);
    assert_eq!(page.total, 25);
    assert!(page.has_more);
    assert_eq!(page.records.get(19).map(|record| record.0), Some(19));
}

#[test]
fn test_list_pools_has_more_flag_accuracy() {
    let t = setup_with_pool_records(5);

    // First page (limit 3 of 5) -> has_more is true
    let page1 = t.client.list_pools(&0u32, &3u32);
    assert_eq!(page1.records.len(), 3);
    assert_eq!(page1.next_start_id, 3);
    assert!(page1.has_more);

    // Next page (start 3, limit 3 of 5) -> has_more is false (reaches end: 5)
    let page2 = t.client.list_pools(&page1.next_start_id, &3u32);
    assert_eq!(page2.records.len(), 2);
    assert_eq!(page2.next_start_id, 5);
    assert!(!page2.has_more);
}

// ── get_pools_by_asset ────────────────────────────────────────────────────────

#[test]
fn test_get_pools_by_asset_returns_empty_when_no_pools() {
    let t = setup();
    let asset = t.asset.clone();
    let page = t.client.get_pools_by_asset(&asset, &0u32, &10u32);
    assert_eq!(
        page.records.len(),
        0,
        "expected no pools for a fresh factory"
    );
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
        .create_pool(&t.asset, &1_728_000u128, &2u32, &10u64, &0i128);
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
                (
                    pool_id,
                    pool_addr.clone(),
                    t.wasm_hash.clone(),
                    new_wasm_hash.clone()
                )
                    .into_val(&t.env),
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
fn test_upgrade_pool_same_hash_returns_pool_upgrade_failed() {
    let t = setup();
    let pool_id = t
        .client
        .create_pool(&t.asset, &1_728_000u128, &2u32, &10u64, &0i128);

    assert_eq!(
        t.client.try_upgrade_pool(&pool_id, &t.wasm_hash),
        Err(Ok(FactoryError::PoolUpgradeFailed))
    );
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
        .create_pool(&t.asset, &1_728_000u128, &2u32, &10u64, &0i128);

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

#[contract]
struct NonUpgradableContract;

#[contractimpl]
impl NonUpgradableContract {
    pub fn dummy(_env: Env) -> bool {
        true
    }
}

#[test]
fn test_upgrade_pool_non_upgradable_pool_returns_pool_upgrade_failed() {
    let t = setup();
    let non_upgradable_addr = t.env.register(NonUpgradableContract, ());
    let fake_record = PoolRecord {
        address: non_upgradable_addr,
        asset: Address::generate(&t.env),
        credit_rate: 100,
        global_multiplier: 1,
        min_lock_period: 10,
        daily_rate: 100 * 17280,
        wasm_hash: soroban_sdk::BytesN::from_array(&t.env, &[0; 32]),
    };
    t.env.as_contract(&t.factory_addr, || {
        t.env
            .storage()
            .persistent()
            .set(&DataKey::Pool(99), &fake_record);
    });

    let new_wasm_hash = upload_replacement_wasm(&t.env);
    let result = t.client.try_upgrade_pool(&99u32, &new_wasm_hash);
    assert_eq!(result, Err(Ok(FactoryError::PoolUpgradeFailed)));
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

    let asset = register_asset(&env2);
    let result = client2.try_create_pool(&asset, &17_280_000u128, &2u32, &86_400u64, &0i128);
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
    let asset = register_asset(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let id_a = client.create_pool(&asset, &8_640_000u128, &2u32, &100u64, &0i128);
    let id_b = client.create_pool(&asset, &17_280_000u128, &3u32, &200u64, &0i128);
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

    let asset = register_asset(&env);
    let id = client.create_pool(&asset, &4_320_000u128, &2u32, &50u64, &0i128);
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

    let asset_a = register_asset(&env);
    let asset_b = register_asset(&env);

    let id_0 = client.create_pool(&asset_a, &1_728_000u128, &2u32, &10u64, &0i128);
    let id_1 = client.create_pool(&asset_b, &3_456_000u128, &2u32, &20u64, &0i128);
    let id_2 = client.create_pool(&asset_a, &5_184_000u128, &2u32, &30u64, &0i128);

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
    let asset = register_asset(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&asset, &1_728_000u128, &2u32, &10u64, &0i128);
    let unknown = Address::generate(&env);
    let result = client.get_pools_by_asset(&unknown, &0u32, &10u32);
    assert_eq!(result.records.len(), 0);
}

#[test]
fn test_get_pools_by_asset_paginates_large_matching_registry() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    // Create 25 pools all sharing the same asset
    let asset = register_asset(&env);
    for i in 0..25 {
        client.create_pool(
            &asset,
            &(1_728_000 + i as u128 * 17_280),
            &2u32,
            &(10 + i as u64),
            &0i128,
        );
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
fn test_get_pools_by_asset_bounds_scan_for_sparse_matches() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let sparse_asset = register_asset(&env);
    let other_asset = register_asset(&env);
    for i in 0..500 {
        let asset = if i == 499 {
            sparse_asset.clone()
        } else {
            other_asset.clone()
        };
        client.create_pool(
            &asset,
            &(1_728_000 + i as u128 * 17_280),
            &2u32,
            &(10 + i as u64),
            &0i128,
        );
    }

    let page = client.get_pools_by_asset(&sparse_asset, &0u32, &20u32);

    assert_eq!(page.records.len(), 0);
    assert_eq!(page.next_start_id, 50);
    assert_eq!(page.total, 500);
}

#[test]
fn test_get_pools_by_asset_can_page_sparse_matches_to_completion() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let sparse_asset = register_asset(&env);
    let other_asset = register_asset(&env);
    for i in 0..500 {
        let asset = if i == 250 || i == 499 {
            sparse_asset.clone()
        } else {
            other_asset.clone()
        };
        client.create_pool(
            &asset,
            &(1_728_000 + i as u128 * 17_280),
            &2u32,
            &(10 + i as u64),
            &0i128,
        );
    }

    let mut start_id = 0u32;
    let mut found_ids: Vec<u32> = vec![&env];

    loop {
        let page = client.get_pools_by_asset(&sparse_asset, &start_id, &20u32);
        for idx in 0..page.records.len() {
            found_ids.push_back(page.records.get(idx).unwrap().0);
        }
        if page.next_start_id == page.total {
            break;
        }
        start_id = page.next_start_id;
    }

    assert_eq!(found_ids.len(), 2);
    assert_eq!(found_ids.get(0), Some(250));
    assert_eq!(found_ids.get(1), Some(499));
}

#[test]
fn test_get_pools_by_asset_range_custom_scan_limit() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let sparse_asset = register_asset(&env);
    let other_asset = register_asset(&env);
    for i in 0..100 {
        let asset = if i == 90 {
            sparse_asset.clone()
        } else {
            other_asset.clone()
        };
        client.create_pool(
            &asset,
            &(1_728_000 + i as u128 * 17_280),
            &2u32,
            &(10 + i as u64),
            &0i128,
        );
    }

    // With a scan_limit of 50, first call should only scan 0..50
    let page1 = client.get_pools_by_asset_range(&sparse_asset, &0u32, &50u32, &20u32);
    assert_eq!(page1.records.len(), 0);
    assert_eq!(page1.next_start_id, 50);

    // Second call scanning 50..100 should find the match at 90
    let page2 =
        client.get_pools_by_asset_range(&sparse_asset, &page1.next_start_id, &50u32, &20u32);
    assert_eq!(page2.records.len(), 1);
    assert_eq!(page2.records.get(0).unwrap().0, 90);
    assert_eq!(page2.next_start_id, 100);
}

#[test]
fn test_create_pool_emits_pool_crtd_event() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let wasm_hash = upload_farming_pool_wasm(&env);
    let asset = register_asset(&env);
    let factory_addr = env.register(Factory, ());
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    client.create_pool(&asset, &5_184_000u128, &2u32, &30u64, &0i128);
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
    let asset_a = register_asset(&env);
    let asset_b = register_asset(&env);
    let id_a = client.create_pool(&asset_a, &1_728_000u128, &2u32, &10u64, &0i128);
    let id_b = client.create_pool(&asset_b, &3_456_000u128, &2u32, &20u64, &0i128);
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
    let asset = t.asset.clone();
    let args = (&asset, 17_280_000u128, 2u32, 86_400u64, 0i128).into_val(&t.env);
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
        .try_create_pool(&asset, &17_280_000u128, &2u32, &86_400u64, &0i128);

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
        .create_pool(&t.asset, &8_640_000u128, &2u32, &100u64, &0i128);
    assert_eq!(id_a, 0);
    assert_eq!(t.client.pool_count(), 1);

    let id_b = t
        .client
        .create_pool(&t.asset, &17_280_000u128, &2u32, &200u64, &0i128);
    assert_eq!(id_b, 1);
    assert_eq!(t.client.pool_count(), 2);
}

#[test]
fn test_create_pool_returns_typed_error_when_pool_count_overflows() {
    let t = setup();

    t.env.as_contract(&t.factory_addr, || {
        t.env
            .storage()
            .instance()
            .set(&DataKey::PoolCount, &u32::MAX);
    });

    let result = t
        .client
        .try_create_pool(&t.asset, &1_728_000u128, &2u32, &100u64, &0i128);

    assert_eq!(result, Err(Ok(FactoryError::PoolCountOverflow)));
    assert_eq!(t.client.pool_count(), u32::MAX);
    assert!(!t.env.as_contract(&t.factory_addr, || {
        t.env.storage().persistent().has(&DataKey::Pool(u32::MAX))
    }));
}

#[test]
fn test_create_pool_uses_deterministic_pool_addresses() {
    let t = setup();
    let asset = t.asset.clone();
    let expected_before = expected_pool_address(&t.env, &t.factory_addr, 0);
    let expected_again = expected_pool_address(&t.env, &t.factory_addr, 0);

    let id = t
        .client
        .create_pool(&asset, &4_320_000u128, &2u32, &50u64, &0i128);
    let record = t.client.get_pool(&id);

    assert_eq!(id, 0);
    assert_eq!(expected_before, expected_again);
    assert_eq!(record.address, expected_before);
}

#[test]
fn test_create_pool_rejects_zero_daily_rate() {
    let t = setup();
    let asset = t.asset.clone();

    // Below LEDGERS_PER_DAY (17_280), the daily_rate -> credit_rate conversion
    // truncates to zero, which FarmingPool::initialize would reject anyway
    // (credit_rate must be > 0) — create_pool must catch this itself rather
    // than deploying a pool that can never be initialized.
    let result = t
        .client
        .try_create_pool(&asset, &0u128, &2u32, &25u64, &0i128);
    assert_eq!(result, Err(Ok(FactoryError::InvalidCreditRate)));
    assert_eq!(t.client.pool_count(), 0);
}

#[test]
fn test_create_pool_rejects_dust_minimum_stake() {
    let t = setup();
    let asset = t.asset.clone();

    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &2u32, &25u64, &1i128);

    assert_eq!(result, Err(Ok(FactoryError::InvalidMinStakeAmount)));
    assert_eq!(t.client.pool_count(), 0);
}

#[test]
fn test_create_pool_accepts_daily_rate_below_ledgers_per_day_with_ceiling_rounding() {
    let t = setup();
    let asset = t.asset.clone();

    // Before #148 the daily_rate -> credit_rate conversion truncated, so a
    // sub-LEDGERS_PER_DAY daily_rate (17_279) became 0 and was rejected. Now it
    // uses ceiling division, so it rounds up to credit_rate = 1 and the pool is
    // created successfully.
    let pool_id = t
        .client
        .create_pool(&asset, &17_279u128, &2u32, &25u64, &0i128);
    assert_eq!(pool_id, 0);

    let pool_addr = t.client.get_pool(&pool_id).address;
    let pool = FarmingPoolClient::new(&t.env, &pool_addr);
    assert_eq!(pool.credit_rate(), 1);
}

#[test]
fn test_create_pool_rejects_global_multiplier_below_one() {
    let t = setup();
    let asset = t.asset.clone();

    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &0u32, &25u64, &0i128);
    assert_eq!(result, Err(Ok(FactoryError::InvalidGlobalMultiplier)));
}

#[test]
fn test_create_pool_rejects_min_lock_period_out_of_u32_range() {
    let t = setup();
    let asset = t.asset.clone();

    let too_large = (u32::MAX as u64) + 1;
    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &2u32, &too_large, &0i128);
    assert_eq!(result, Err(Ok(FactoryError::MinLockPeriodOutOfRange)));
}

#[test]
fn test_get_pool_bumps_pool_record_ttl() {
    let t = setup();
    let id = t
        .client
        .create_pool(&t.asset, &4_320_000u128, &2u32, &50u64, &0i128);

    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    advance_ledgers(&t.env, TTL_EXTEND_TO - TTL_THRESHOLD + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    assert!(t.client.try_get_pool(&id).is_ok());
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_list_pools_bumps_pool_record_ttl() {
    let t = setup();
    let id = t
        .client
        .create_pool(&t.asset, &4_320_000u128, &2u32, &50u64, &0i128);

    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    advance_ledgers(&t.env, TTL_EXTEND_TO - TTL_THRESHOLD + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    assert!(t.client.try_list_pools(&id, &1u32).is_ok());
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_refresh_pool_ttls_restores_ttl_for_unqueried_pool() {
    let t = setup();
    let id = t
        .client
        .create_pool(&t.asset, &1_728_000u128, &2u32, &50u64, &0i128);

    // Initial TTL after creation
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);

    // Advance ledgers past TTL_EXTEND_TO without ever calling get_pool on this pool
    advance_ledgers(&t.env, TTL_EXTEND_TO + 1);
    assert!(pool_record_ttl(&t.env, &t.factory_addr, id) < TTL_THRESHOLD);

    // Call refresh_pool_ttls to restore TTL without a specific get_pool query
    assert_eq!(t.client.try_refresh_pool_ttls(&id, &1u32), Ok(Ok(())));

    // Verify TTL is restored
    assert_eq!(pool_record_ttl(&t.env, &t.factory_addr, id), TTL_EXTEND_TO);
}

#[test]
fn test_refresh_pool_ttls_stays_permissionless_for_any_caller() {
    // #168: refresh_pool_ttls remains callable by anyone — the keep-alive
    // mechanism must not be gated behind the admin, otherwise a factory whose
    // only lifeline relies on a single admin becomes a single point of failure.
    let t = setup_with_pool_records(3);
    let stranger = Address::generate(&t.env);
    // No admin auth provided at all: assert the refresh still succeeds.
    assert_eq!(t.client.try_refresh_pool_ttls(&0u32, &3u32), Ok(Ok(())));
    assert_eq!(
        pool_record_ttl(&t.env, &t.factory_addr, 1),
        TTL_EXTEND_TO,
        "stranger-triggered keep-alive must still extend TTLs"
    );
    // Sanity: the stranger address is not the admin.
    assert_ne!(t.client.admin(), stranger);
}

#[test]
fn test_refresh_pool_ttls_requires_initialized_factory() {
    // #168: an uninitialized factory has no registry to keep alive, so the
    // permissionless refresh entry point must reject the call with a typed
    // error rather than silently bumping instance state.
    let (_env, client) = setup_uninitialized();
    let result = client.try_refresh_pool_ttls(&0u32, &20u32);
    assert!(matches!(result, Err(Ok(FactoryError::NotInitialized))));
}

#[test]
fn test_create_pool_emits_pool_crtd_event_with_payload() {
    let t = setup();
    let asset = t.asset.clone();
    let expected_address = expected_pool_address(&t.env, &t.factory_addr, 0);
    let id = t
        .client
        .create_pool(&asset, &5_184_000u128, &2u32, &30u64, &0i128);

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
                (
                    id,
                    expected_address,
                    t.admin.clone(),
                    asset,
                    300i128,
                    2u32,
                    30u32,
                    5_184_000u128,
                    t.wasm_hash.clone()
                )
                    .into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_old_admin_cannot_create_pool_after_transfer_but_new_admin_can() {
    let t = setup();
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);

    let old_asset = t.asset.clone();
    let old_args = (&old_asset, 1_728_000u128, 2u32, 10u64, 0i128).into_val(&t.env);
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
        .try_create_pool(&old_asset, &1_728_000u128, &2u32, &10u64, &0i128);

    assert!(
        old_result.is_err(),
        "old admin must not authorize new pool creation"
    );
    assert_eq!(t.client.pool_count(), 0);

    let new_asset = t.asset.clone();
    let new_args = (&new_asset, 3_456_000u128, 2u32, 20u64, 0i128).into_val(&t.env);
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
        .create_pool(&new_asset, &3_456_000u128, &2u32, &20u64, &0i128);

    assert_eq!(new_id, 0);
    assert_eq!(t.client.pool_count(), 1);
}

#[test]
fn test_create_pool_initializes_real_farming_pool_atomically() {
    // Regression test for #79/#80: create_pool must call the deployed pool's
    // own `initialize` in the same transaction as the deploy, so there is no
    // window where an uninitialized pool address is observable on-chain.
    //
    // NOTE: We cannot reuse `MOCK_POOL_WASM` here, because that stub contract does
    // not implement FarmingPool entrypoints.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let factory_addr = env.register(Factory, ());

    let wasm_hash = env.deployer().upload_contract_wasm(farming_pool::WASM);
    let client = FactoryClient::new(&env, &factory_addr);
    client.initialize(&admin, &wasm_hash);

    let asset = register_asset(&env);
    let pool_id = client.create_pool(&asset, &1_728_000u128, &2u32, &10u64, &0i128);
    let record = client.get_pool(&pool_id);

    let pool_client = FarmingPoolClient::new(&env, &record.address);

    // The pool must already be initialized: an initialized-only getter must
    // succeed and return the factory's admin, not panic with NotInitialized.
    assert_eq!(pool_client.admin(), admin);
}

// ── pause_pool_creation tests ──────────────────────────────────────────────────

#[test]
fn test_pause_pool_creation_prevents_create_pool() {
    let t = setup();
    assert!(!t.client.is_pool_creation_paused());

    t.client.pause_pool_creation();
    assert!(t.client.is_pool_creation_paused());

    let asset = t.asset.clone();
    let result = t
        .client
        .try_create_pool(&asset, &1_728_000u128, &2u32, &10u64, &0i128);

    assert!(matches!(result, Err(Ok(FactoryError::PoolCreationPaused))));
}

#[test]
fn test_unpause_pool_creation_allows_create_pool() {
    let t = setup();
    t.client.pause_pool_creation();
    assert!(t.client.is_pool_creation_paused());

    t.client.unpause_pool_creation();
    assert!(!t.client.is_pool_creation_paused());

    let asset = t.asset.clone();
    let pool_id = t
        .client
        .create_pool(&asset, &1_728_000u128, &2u32, &10u64, &0i128);
    assert_eq!(pool_id, 0);
}

#[test]
fn test_pause_pool_creation_requires_admin_auth() {
    let t = setup();
    let user = Address::generate(&t.env);

    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &t.factory_addr,
                fn_name: "pause_pool_creation",
                args: ().into_val(&t.env),
                sub_invokes: &[],
            },
        }])
        .try_pause_pool_creation();

    assert!(
        result.is_err(),
        "non-admin pause_pool_creation must be rejected"
    );
}

#[test]
fn test_admin_transfer_count_increments_on_transfer() {
    let t = setup();
    assert_eq!(t.client.admin_transfer_count(), 0);
    assert_eq!(t.client.get_admin_transfer_count(), 0);

    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(t.client.admin_transfer_count(), 1);
    assert_eq!(t.client.get_admin_transfer_count(), 1);

    let next_admin = Address::generate(&t.env);
    t.client.transfer_admin(&next_admin);
    assert_eq!(t.client.admin_transfer_count(), 2);
    assert_eq!(t.client.get_admin_transfer_count(), 2);

    // Rejected non-admin attempt does not increment
    let unauthorized = Address::generate(&t.env);
    let candidate = Address::generate(&t.env);
    let args = (&candidate,).into_val(&t.env);
    let invoke = MockAuthInvoke {
        contract: &t.factory_addr,
        fn_name: "transfer_admin",
        args,
        sub_invokes: &[],
    };
    let result = t
        .client
        .mock_auths(&[MockAuth {
            address: &unauthorized,
            invoke: &invoke,
        }])
        .try_transfer_admin(&candidate);
    assert!(result.is_err());
    assert_eq!(t.client.admin_transfer_count(), 2);
}

#[test]
fn test_get_pools_by_admin_returns_created_pools() {
    let t = setup();
    assert_eq!(t.client.get_pools_by_admin(&t.admin), vec![&t.env]);

    let asset1 = Address::generate(&t.env);
    let id1 = t
        .client
        .create_pool(&asset1, &1_728_000u128, &2u32, &10u64, &0i128);

    let asset2 = Address::generate(&t.env);
    let id2 = t
        .client
        .create_pool(&asset2, &3_456_000u128, &2u32, &20u64, &0i128);

    let pools = t.client.get_pools_by_admin(&t.admin);
    assert_eq!(pools, vec![&t.env, id1, id2]);
}

#[test]
fn test_get_admin_pool_count_tracks_pools_created_by_admin() {
    let t = setup();
    assert_eq!(t.client.get_admin_pool_count(&t.admin), 0);

    let asset1 = Address::generate(&t.env);
    t.client
        .create_pool(&asset1, &1_728_000u128, &2u32, &10u64, &0i128);
    assert_eq!(t.client.get_admin_pool_count(&t.admin), 1);

    let asset2 = Address::generate(&t.env);
    t.client
        .create_pool(&asset2, &3_456_000u128, &2u32, &20u64, &0i128);
    assert_eq!(t.client.get_admin_pool_count(&t.admin), 2);

    let other_admin = Address::generate(&t.env);
    assert_eq!(t.client.get_admin_pool_count(&other_admin), 0);
}

#[test]
fn test_get_admin_pool_count_uninitialized_returns_not_initialized() {
    let (_env, client) = setup_uninitialized();
    let admin = Address::generate(&_env);
    let result = client.try_get_admin_pool_count(&admin);
    assert!(matches!(result, Err(Ok(FactoryError::NotInitialized))));
}
