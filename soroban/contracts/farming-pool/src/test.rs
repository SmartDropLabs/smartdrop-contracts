#![cfg(test)]
use super::*;
use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Events, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, BytesN, Env, IntoVal, Symbol, Val,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: FarmingPoolClient<'static>,
    contract_id: Address,
    token: TokenClient<'static>,
    token_sac: StellarAssetClient<'static>,
    admin: Address,
    user: Address,
}

fn upload_upgrade_target_wasm(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(ADD_I32_WASM)
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

fn setup(global_multiplier: u32, credit_rate: i128) -> TestEnv {
    setup_with_lock_period(global_multiplier, credit_rate, 0)
}

fn setup_uninitialized() -> (Env, FarmingPoolClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let user = Address::generate(&env);
    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };
    (env, client, user)
}

fn setup_with_lock_period(
    global_multiplier: u32,
    credit_rate: i128,
    min_lock_period: u32,
) -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    // Deploy a Stellar Asset Contract for the stake token.
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&user, &1_000_000_000i128);

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    let min_stake_amount = 100;
    client.initialize(
        &admin,
        &asset.address(),
        &global_multiplier,
        &credit_rate,
        &min_lock_period,
        &min_stake_amount,
    );

    let token = TokenClient::new(&env, &asset.address());

    // Transmute lifetime to 'static so the struct can own client & token.
    // SAFETY: env owns the contract and token registrations; they live as long as env.
    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };
    let token = unsafe { core::mem::transmute::<TokenClient<'_>, TokenClient<'static>>(token) };
    let token_sac = unsafe {
        core::mem::transmute::<StellarAssetClient<'_>, StellarAssetClient<'static>>(token_sac)
    };

    TestEnv {
        env,
        client,
        contract_id,
        token,
        token_sac,
        admin,
        user,
    }
}

fn setup_without_mocked_auth() -> (Env, Address, FarmingPoolClient<'static>, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    client.initialize(&admin, &asset.address(), &2u32, &1i128, &0u32, &1_i128);

    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };

    (env, contract_id, client, admin, user)
}

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|l| l.sequence_number = current + by);
}

#[test]
fn test_stake_uninitialized_returns_not_initialized() {
    let (_env, client, user) = setup_uninitialized();
    match client.try_stake(&user, &100i128) {
        Err(Ok(PoolError::NotInitialized)) => {}
        _ => panic!("expected PoolError::NotInitialized"),
    }
}

#[test]
fn test_pause_uninitialized_returns_not_initialized() {
    let (_env, client, _user) = setup_uninitialized();
    match client.try_pause() {
        Err(Ok(PoolError::NotInitialized)) => {}
        _ => panic!("expected PoolError::NotInitialized"),
    }
}

#[test]
fn test_schema_version_defaults_to_current_release() {
    let t = setup(2, 1);
    assert_eq!(t.client.schema_version(), SCHEMA_VERSION);
}

#[test]
fn test_migrate_placeholder_requires_admin_and_stamps_current_version() {
    let t = setup(2, 1);
    assert_eq!(t.client.migrate(), SCHEMA_VERSION);
    assert_eq!(t.client.schema_version(), SCHEMA_VERSION);
}

#[test]
fn test_upgrade_preserves_stake_storage_and_enables_new_wasm() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);

    let before = t.client.get_stake(&t.user).expect("stake must exist");
    let new_wasm_hash = upload_upgrade_target_wasm(&t.env);

    t.client.upgrade(&new_wasm_hash);

    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("upgraded").into_val(&t.env)
                ],
                new_wasm_hash.clone().into_val(&t.env),
            )
        ]
    );

    let stored = t.env.as_contract(&t.contract_id, || {
        t.env
            .storage()
            .persistent()
            .get::<DataKey, UserStake>(&DataKey::UserStake(t.user.clone()))
    });
    let stored = stored.expect("stake storage must survive wasm upgrade");
    assert_eq!(stored.amount, before.amount);
    assert_eq!(stored.start_ledger, before.start_ledger);
    assert_eq!(stored.credits_banked, before.credits_banked);
    assert_eq!(stored.credit_rate, before.credit_rate);

    let args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&t.env, 2i32.into_val(&t.env), 40i32.into_val(&t.env)];
    let sum: i32 = t
        .env
        .invoke_contract(&t.contract_id, &Symbol::new(&t.env, "add"), args);
    assert_eq!(sum, 42);
}

#[test]
fn test_upgrade_requires_admin_auth() {
    let (env, contract_id, client, admin, user) = setup_without_mocked_auth();
    let new_wasm_hash = upload_upgrade_target_wasm(&env);

    let result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "upgrade",
                args: (&new_wasm_hash,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_upgrade(&new_wasm_hash);

    assert!(result.is_err(), "non-admin upgrade must be rejected");
    assert_eq!(client.admin(), admin);
}

#[test]
fn test_admin_uninitialized_returns_not_initialized() {
    let (_env, client, _user) = setup_uninitialized();
    match client.try_admin() {
        Err(Ok(PoolError::NotInitialized)) => {}
        _ => panic!("expected PoolError::NotInitialized"),
    }
}

#[test]
fn test_transfer_admin_uninitialized_returns_not_initialized() {
    let (env, client, _user) = setup_uninitialized();
    let new_admin = Address::generate(&env);
    match client.try_transfer_admin(&new_admin) {
        Err(Ok(PoolError::NotInitialized)) => {}
        _ => panic!("expected PoolError::NotInitialized"),
    }
}

// ── Boost calculation unit tests ──────────────────────────────────────────────

#[test]
fn test_effective_stake_no_boost() {
    // Without boost, effective stake equals staked amount (allocation_pct = 0 → multiplier has no effect).
    let stake = compute_total_stake(1_000, 0, 5);
    assert_eq!(stake, 1_000);
}

#[test]
fn test_effective_stake_full_allocation_2x() {
    // 100% allocation at 2× multiplier: virtual_stake = 1000 * 2 = 2000, principal = 0.
    let stake = compute_total_stake(1_000, 100, 2);
    assert_eq!(stake, 2_000);
}

#[test]
fn test_effective_stake_half_allocation_2x() {
    // 50% allocation at 2×: principal = 500, virtual = 500*2 = 1000. total = 1500.
    let stake = compute_total_stake(1_000, 50, 2);
    assert_eq!(stake, 1_500);
}

#[test]
fn test_effective_stake_25pct_allocation_3x() {
    // 25% allocation at 3×: boosted = 250, principal = 750, virtual = 750. total = 1500.
    let stake = compute_total_stake(1_000, 25, 3);
    assert_eq!(stake, 1_500);
}

#[test]
fn test_effective_stake_1pct_allocation_10x() {
    // Minimal allocation at high multiplier.
    // boosted = 10, principal = 990, virtual = 100. total = 1090.
    let stake = compute_total_stake(1_000, 1, 10);
    assert_eq!(stake, 1_090);
}

// ── Boost system integration tests ───────────────────────────────────────────

#[test]
fn test_set_boost_and_get_config() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    let cfg = t
        .client
        .get_boost_config(&t.user)
        .expect("boost config should be set");
    assert_eq!(cfg.allocation_pct, 50);
    assert_eq!(cfg.multiplier, 2);
}

#[test]
fn test_get_boost_config_none_before_set() {
    let t = setup(2, 1);
    assert!(t.client.get_boost_config(&t.user).is_none());
}

#[test]
fn test_credits_without_boost_accrue_at_face_value() {
    // credit_rate = 1, no boost → credits = amount * ledgers
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 1_000 * 10);
}

#[test]
fn test_credits_with_50pct_boost_2x_multiplier() {
    // effective_stake = 1500, credit_rate = 1, ledgers = 10 → 15000 credits
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 1_500 * 10);
}

#[test]
fn test_credits_with_100pct_boost_2x_multiplier() {
    // effective_stake = 2000, 10 ledgers → 20000 credits
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &100u32);
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 2_000 * 10);
}

#[test]
fn test_boost_update_preserves_previously_earned_credits() {
    // Stake, earn 5 ledgers unbooted, then set 50% boost, earn 5 more.
    // First 5: credits = 1000 * 5 = 5000 (no boost)
    // Next 5: credits = 1500 * 5 = 7500 (50% boost, 2×)
    // Total: 12500
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 5);
    t.client.set_boost(&t.user, &50u32); // checkpoints 5000 credits
    advance_ledgers(&t.env, 5);
    assert_eq!(t.client.get_credits(&t.user), 12_500);
}

#[test]
fn test_boost_can_be_updated_repeatedly_without_losing_credits() {
    // 10 ledgers at 50% boost (effective 1500), then 10 at 100% (effective 2000).
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);
    t.client.set_boost(&t.user, &100u32); // checkpoints 15000
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 15_000 + 20_000);
}

#[test]
fn test_set_boost_rejects_zero_allocation() {
    // Soroban host wraps contract panics in HostError; use try_ client variants to inspect them.
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    assert!(t.client.try_set_boost(&t.user, &0u32).is_err());
}

#[test]
fn test_set_boost_rejects_over_100_allocation() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    assert!(t.client.try_set_boost(&t.user, &101u32).is_err());
}

#[test]
fn test_admin_sets_global_multiplier() {
    let t = setup(2, 1);
    t.client.set_global_multiplier(&3u32);
    // Boost config for a user should reflect new multiplier.
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    let cfg = t.client.get_boost_config(&t.user).unwrap();
    assert_eq!(cfg.multiplier, 3);
}

#[test]
fn test_set_credit_rate_updates_public_getters() {
    let t = setup_with_lock_period(2, 1, 12);
    t.client.set_credit_rate(&4i128);
    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("rate_set").into_val(&t.env)
                ],
                (1i128, 4i128).into_val(&t.env),
            )
        ]
    );

    t.client.set_min_lock_period(&25u32);
    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("lock_set").into_val(&t.env)
                ],
                (12u32, 25u32).into_val(&t.env),
            )
        ]
    );

    assert_eq!(t.client.credit_rate(), 4);
    assert_eq!(t.client.get_credit_rate(), 4);
    assert_eq!(t.client.min_lock_period(), 25);
    assert_eq!(t.client.get_min_lock_period(), 25);
}

#[test]
fn test_set_credit_rate_rejects_zero_with_typed_error() {
    let t = setup(2, 1);
    let result = t.client.try_set_credit_rate(&0i128);
    assert!(matches!(result, Err(Ok(PoolError::InvalidCreditRate))));
}

// ── #89: value ceilings on set_global_multiplier / set_credit_rate ──────────

#[test]
fn test_set_credit_rate_rejects_above_ceiling() {
    let t = setup(2, 1);
    let result = t.client.try_set_credit_rate(&(MAX_CREDIT_RATE + 1));
    assert!(matches!(result, Err(Ok(PoolError::InvalidCreditRate))));
}

#[test]
fn test_set_credit_rate_accepts_exactly_the_ceiling() {
    let t = setup(2, 1);
    t.client.set_credit_rate(&MAX_CREDIT_RATE);
    assert_eq!(t.client.credit_rate(), MAX_CREDIT_RATE);
}

#[test]
fn test_set_credit_rate_requires_admin_auth() {
    let (env, contract_id, client, _admin, user) = setup_without_mocked_auth();

    let result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_credit_rate",
                args: (&5i128,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_credit_rate(&5i128);

    assert!(
        result.is_err(),
        "non-admin set_credit_rate must be rejected"
    );
}

#[test]
fn test_set_min_lock_period_requires_admin_auth() {
    let (env, contract_id, client, _admin, user) = setup_without_mocked_auth();

    let result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_min_lock_period",
                args: (&9u32,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_min_lock_period(&9u32);

    assert!(
        result.is_err(),
        "non-admin set_min_lock_period must be rejected"
    );
}

#[test]
fn test_credit_rate_change_does_not_retroactively_alter_staked_credits() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);

    t.client.set_credit_rate(&3i128);
    assert_eq!(t.client.get_credits(&t.user), 10_000);

    t.client.stake(&t.user, &100); // checkpoints under the old rate (top-up must clear min_stake_amount)
    advance_ledgers(&t.env, 5);
    assert_eq!(t.client.get_credits(&t.user), 26_500);
}

#[test]
fn test_credit_rate_change_does_not_retroactively_alter_locked_credits() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);

    t.client.set_credit_rate(&3i128);
    assert_eq!(t.client.calculate_credits(&t.user), 10_000);

    t.client.lock_assets(&t.user, &100); // checkpoints under the old rate (top-up must clear min_stake_amount)
    advance_ledgers(&t.env, 5);
    assert_eq!(t.client.calculate_credits(&t.user), 26_500);
}

#[test]
fn test_admin_multiplier_change_applies_from_next_checkpoint() {
    // 10 ledgers at 50% boost @ 2×, then user checkpoints (banking 2× credits),
    // then admin bumps to 3×, then 10 more ledgers at 50% @ 3×.
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);
    // User checkpoints at 2× before admin changes the multiplier.
    // effective_stake = 1500 → 15000 banked.
    t.client.set_boost(&t.user, &50u32);
    t.client.set_global_multiplier(&3u32);
    advance_ledgers(&t.env, 10);
    // Next 10 ledgers: effective_stake = 2000 (50% @ 3×) → 20000
    assert_eq!(t.client.get_credits(&t.user), 35_000);
}

#[test]
fn test_admin_multiplier_rejects_zero() {
    // Updated for #89: the old bare `assert!` (matched via `should_panic`)
    // was replaced with a typed `PoolError::InvalidGlobalMultiplier` return,
    // so this now asserts via `try_set_global_multiplier` like the rest of
    // the typed-error suite instead of panic-message matching.
    let t = setup(2, 1);
    let result = t.client.try_set_global_multiplier(&0u32);
    assert!(matches!(
        result,
        Err(Ok(PoolError::InvalidGlobalMultiplier))
    ));
}

#[test]
fn test_set_global_multiplier_rejects_above_ceiling() {
    let t = setup(2, 1);
    let result = t
        .client
        .try_set_global_multiplier(&(MAX_GLOBAL_MULTIPLIER + 1));
    assert!(matches!(
        result,
        Err(Ok(PoolError::InvalidGlobalMultiplier))
    ));
}

#[test]
fn test_set_global_multiplier_accepts_exactly_the_ceiling() {
    let t = setup(2, 1);
    t.client.set_global_multiplier(&MAX_GLOBAL_MULTIPLIER);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    let cfg = t.client.get_boost_config(&t.user).unwrap();
    assert_eq!(cfg.multiplier, MAX_GLOBAL_MULTIPLIER);
}

#[test]
fn test_initialize_rejects_global_multiplier_above_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);

    let result = client.try_initialize(
        &admin,
        &asset.address(),
        &(MAX_GLOBAL_MULTIPLIER + 1),
        &1i128,
        &0u32,
        &1i128,
    );
    assert!(matches!(
        result,
        Err(Ok(PoolError::InvalidGlobalMultiplier))
    ));
}

#[test]
fn test_initialize_rejects_credit_rate_above_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);

    let result = client.try_initialize(
        &admin,
        &asset.address(),
        &2u32,
        &(MAX_CREDIT_RATE + 1),
        &0u32,
        &1i128,
    );
    assert!(matches!(result, Err(Ok(PoolError::InvalidCreditRate))));
}

/// Ties the #89 fix to its own derivation: at both ceilings simultaneously
/// (`MAX_GLOBAL_MULTIPLIER`, `MAX_CREDIT_RATE`), full boost allocation (100%,
/// `compute_total_stake`'s worst case), the derivation's `amount_max`, and
/// its `elapsed_max` (~10 years of ledgers), the boost-path credit preview
/// (`get_credits`, which exercises the exact `compute_total_stake` /
/// `compute_credits` chain the derivation bounds) must not overflow/panic and
/// must equal the worst-case product computed by the same formula. This is a
/// single deterministic boundary point, not a fuzz/property suite (#75/#76
/// remain out of scope).
#[test]
fn test_compute_credits_no_overflow_at_ceilings() {
    // Matches the doc comment on MAX_GLOBAL_MULTIPLIER/MAX_CREDIT_RATE.
    const AMOUNT_MAX: i128 = 1_000_000_000_000_000_000; // 10^18
    const ELAPSED_MAX: u32 = 63_072_000; // ~10 years at 5s/ledger

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&user, &AMOUNT_MAX);

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &asset.address(),
        &MAX_GLOBAL_MULTIPLIER,
        &MAX_CREDIT_RATE,
        &0u32,
        &1i128,
    );

    client.stake(&user, &AMOUNT_MAX);
    client.set_boost(&user, &100u32);
    advance_ledgers(&env, ELAPSED_MAX);

    // No panic/overflow trap here is itself the assertion; also check the
    // value is exactly the worst-case product the derivation computed.
    let credits = client.get_credits(&user);
    let expected =
        AMOUNT_MAX * MAX_GLOBAL_MULTIPLIER as i128 * MAX_CREDIT_RATE * ELAPSED_MAX as i128;
    assert_eq!(credits, expected);
}

#[test]
fn test_unstake_returns_tokens_and_credits() {
    let t = setup(2, 1);
    let initial_balance = t.token.balance(&t.user);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);
    let credits = t.client.unstake(&t.user);
    assert_eq!(credits, 15_000); // 1500 * 10
    assert_eq!(t.token.balance(&t.user), initial_balance);
    assert!(t.client.get_stake(&t.user).is_none());
}

#[test]
fn test_additional_stake_checkpoints_credits() {
    // Stake 1000, earn 10 ledgers (= 10000 credits), then stake 500 more.
    // After checkpoint: banked = 10000, amount = 1500.
    // Earn 10 more ledgers with 0 boost: 1500 * 10 = 15000.
    // Total: 25000.
    let t = setup(1, 1); // multiplier=1 so no boost effect here
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.stake(&t.user, &500); // triggers checkpoint
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 25_000);
}

#[test]
fn test_get_credits_zero_without_stake() {
    let t = setup(2, 1);
    assert_eq!(t.client.get_credits(&t.user), 0);
}

// ── lock_assets tests ─────────────────────────────────────────────────────────

#[test]
fn test_admin_getter_returns_current_admin() {
    let t = setup(2, 1);
    assert_eq!(t.client.admin(), t.admin);
}

#[test]
fn test_transfer_admin_changes_admin() {
    let t = setup(2, 1);
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(t.client.admin(), new_admin);
}

#[test]
fn test_transfer_admin_emits_event() {
    let t = setup(2, 1);
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("adm_xfr").into_val(&t.env)
                ],
                (t.admin.clone(), new_admin.clone()).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_transfer_admin_requires_current_admin_auth() {
    let (env, contract_id, client, admin, user) = setup_without_mocked_auth();
    let new_admin = Address::generate(&env);
    let result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "transfer_admin",
                args: (&new_admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_transfer_admin(&new_admin);
    assert!(result.is_err(), "non-admin transfer_admin must be rejected");
    assert_eq!(client.admin(), admin);
}

#[test]
fn test_old_admin_loses_privileges_after_transfer() {
    let (env, contract_id, client, old_admin, _user) = setup_without_mocked_auth();
    let new_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "transfer_admin",
                args: (&new_admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .transfer_admin(&new_admin);

    let old_pause = client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "pause",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_pause();
    assert!(old_pause.is_err(), "old admin must not be able to pause");

    let old_multiplier = client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "set_global_multiplier",
                args: (&3u32,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_set_global_multiplier(&3u32);
    assert!(
        old_multiplier.is_err(),
        "old admin must not be able to set global multiplier"
    );

    client
        .mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "pause",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .pause();
    assert!(client.is_paused(), "new admin should be able to pause");
}

#[test]
fn test_lock_assets_creates_position() {
    let t = setup(1, 1);
    let initial_balance = t.token.balance(&t.user);
    t.client.lock_assets(&t.user, &500);
    let pos = t
        .client
        .get_user_position(&t.user)
        .expect("position should exist");
    assert_eq!(pos.amount, 500);
    assert_eq!(pos.total_credits, 0);
    assert_eq!(t.token.balance(&t.user), initial_balance - 500);
    assert_eq!(t.token.balance(&t.contract_id), 500);
}

#[test]
fn test_lock_assets_additional_lock_checkpoints_credits() {
    // Lock 1000, advance 10 ledgers (10000 credits), then lock 500 more.
    // After checkpoint: banked = 10000, amount = 1500.
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.lock_assets(&t.user, &500); // triggers checkpoint
    let pos = t
        .client
        .get_user_position(&t.user)
        .expect("position should exist");
    assert_eq!(pos.amount, 1_500);
    assert_eq!(pos.total_credits, 10_000); // 1000 * 10
}

#[test]
fn test_multiple_locks_credit_only_new_amount_for_later_ledgers() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &100);
    advance_ledgers(&t.env, 1_000);

    t.client.lock_assets(&t.user, &100);
    advance_ledgers(&t.env, 1_000);

    assert_eq!(t.client.calculate_credits(&t.user), 300_000);
}

#[test]
fn test_lock_assets_rejects_zero_amount() {
    let t = setup(1, 1);
    assert!(t.client.try_lock_assets(&t.user, &0i128).is_err());
}

#[test]
fn test_lock_assets_rejects_negative_amount() {
    let t = setup(1, 1);
    assert!(t.client.try_lock_assets(&t.user, &-1i128).is_err());
}

#[test]
fn test_lock_assets_rejects_insufficient_balance() {
    let t = setup(1, 1);
    // User only has 1_000_000_000 tokens; try to lock more.
    assert!(t
        .client
        .try_lock_assets(&t.user, &2_000_000_000i128)
        .is_err());
}

#[test]
fn test_lock_assets_emits_event() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);

    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("locked").into_val(&t.env)
                ],
                (t.user.clone(), 1_000i128).into_val(&t.env),
            )
        ]
    );
}

// ── unlock_assets tests ───────────────────────────────────────────────────────

#[test]
fn test_unlock_assets_full_returns_tokens_and_credits() {
    let t = setup(1, 1);
    let initial_balance = t.token.balance(&t.user);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&t.user, &1_000);
    // All tokens returned, position removed, credits = 1000 * 10.
    assert_eq!(t.token.balance(&t.user), initial_balance);
    assert_eq!(t.token.balance(&t.contract_id), 0);
    assert!(t.client.get_user_position(&t.user).is_none());
    assert_eq!(t.client.calculate_credits(&t.user), 0);
}

#[test]
fn test_unlock_assets_partial_keeps_remaining_position() {
    let t = setup(1, 1);
    let initial_balance = t.token.balance(&t.user);
    t.client.lock_assets(&t.user, &500);
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&t.user, &200); // partial unlock

    let pos = t
        .client
        .get_user_position(&t.user)
        .expect("position should still exist");
    assert_eq!(pos.amount, 300);
    // 500 * 10 = 5000 credits banked during checkpoint.
    assert_eq!(pos.total_credits, 5_000);
    assert_eq!(t.token.balance(&t.user), initial_balance - 300);
    assert_eq!(t.token.balance(&t.contract_id), 300);
}

// ── unlock_assets split-invariance (#123) ─────────────────────────────────────
//
// #75 covers *when* checkpoints happen (time-invariance); this covers a
// distinct axis: whether a withdrawal's final outcome is invariant to *how*
// its amount is partitioned across multiple unlock_assets calls.
// checkpoint_position runs on every call and folds already-accrued credits
// into total_credits — a genuinely different code path per partial call —
// so this isn't guaranteed by construction, only by checkpoint_position's
// formula being linear in `amount`.

#[test]
fn test_unlock_assets_final_outcome_is_invariant_to_how_the_withdrawal_is_split() {
    // Each partition sums to 1_000, varying both the *count* and *sizing* of
    // partial calls — [1] is a single full unlock, [2] two equal halves,
    // [3] three uneven pieces, [4] a pathological near-all-at-once split
    // that would catch an off-by-one on the first/last piece specifically.
    let partitions: [&[i128]; 4] = [&[1_000], &[500, 500], &[300, 200, 500], &[1, 1, 998]];

    // Every call after the first happens with zero further ledger advance,
    // so only the first checkpoint in a partition ever has elapsed > 0 —
    // isolating split-invariance from the already-covered time axis.
    const ELAPSED_LEDGERS: u32 = 10;
    // amount(1_000) * credit_rate(1, from setup(1, 1)) * ELAPSED_LEDGERS.
    const EXPECTED_TOTAL_CREDITS: i128 = 1_000 * ELAPSED_LEDGERS as i128;

    for partition in partitions {
        let t = setup(1, 1);
        let initial_balance = t.token.balance(&t.user);

        t.client.lock_assets(&t.user, &1_000);
        advance_ledgers(&t.env, ELAPSED_LEDGERS);

        let mut last_part = 0i128;
        for &part in partition {
            t.client.unlock_assets(&t.user, &part);
            last_part = part;
        }
        // The test env only retains the most recent invocation's events, so
        // this must be captured immediately after the last unlock — any
        // further contract/token call below would overwrite it.
        let final_unlock_events = t.env.events().all().filter_by_contract(&t.contract_id);

        assert!(
            t.client.get_user_position(&t.user).is_none(),
            "position must be fully cleared regardless of partition {partition:?}",
        );
        assert_eq!(
            t.client.calculate_credits(&t.user),
            0,
            "no position left means no further accruing credits, for partition {partition:?}",
        );
        assert_eq!(
            t.token.balance(&t.user),
            initial_balance,
            "the full locked amount must come back regardless of partition {partition:?}",
        );
        assert_eq!(t.token.balance(&t.contract_id), 0);

        // This is exactly the last unlock's event — and since every call
        // after the first has zero elapsed ledgers, its total_credits is
        // the same cumulative value the *first* checkpoint alone produced,
        // regardless of how many pieces the withdrawal was split into.
        assert_eq!(
            final_unlock_events,
            soroban_sdk::vec![
                &t.env,
                (
                    t.contract_id.clone(),
                    soroban_sdk::vec![
                        &t.env,
                        soroban_sdk::symbol_short!("pool").into_val(&t.env),
                        soroban_sdk::symbol_short!("unlocked").into_val(&t.env)
                    ],
                    (t.user.clone(), last_part, EXPECTED_TOTAL_CREDITS).into_val(&t.env),
                )
            ],
            "final cumulative total_credits must be identical across partitions {partition:?}",
        );
    }
}

#[test]
fn test_unlock_assets_split_across_min_lock_period_boundary_reaches_same_final_state_as_single_unlock(
) {
    // Compare: (A) unlock everything in one call the moment the position
    // matures, vs. (B) an early partial attempt that's correctly rejected
    // before maturity, followed by completing the withdrawal (split across
    // two calls) once matured. Both must reach an identical final state.
    const MIN_LOCK_PERIOD: u32 = 10;

    let a = setup_with_lock_period(1, 1, MIN_LOCK_PERIOD);
    let a_initial_balance = a.token.balance(&a.user);
    a.client.lock_assets(&a.user, &1_000);
    advance_ledgers(&a.env, MIN_LOCK_PERIOD);
    a.client.unlock_assets(&a.user, &1_000);

    let b = setup_with_lock_period(1, 1, MIN_LOCK_PERIOD);
    let b_initial_balance = b.token.balance(&b.user);
    b.client.lock_assets(&b.user, &1_000);
    advance_ledgers(&b.env, MIN_LOCK_PERIOD - 2); // before maturity
    assert!(
        b.client.try_unlock_assets(&b.user, &500).is_err(),
        "an unlock attempted before the min lock period elapses must be rejected"
    );
    advance_ledgers(&b.env, 2); // now exactly at maturity
    b.client.unlock_assets(&b.user, &400);
    b.client.unlock_assets(&b.user, &600);

    assert!(a.client.get_user_position(&a.user).is_none());
    assert!(b.client.get_user_position(&b.user).is_none());
    assert_eq!(a.client.calculate_credits(&a.user), 0);
    assert_eq!(b.client.calculate_credits(&b.user), 0);
    assert_eq!(
        a.token.balance(&a.user) - a_initial_balance,
        b.token.balance(&b.user) - b_initial_balance,
    );
    assert_eq!(
        a.token.balance(&a.contract_id),
        b.token.balance(&b.contract_id)
    );
}

#[test]
fn test_unlock_assets_rejects_zero_amount() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    assert!(t.client.try_unlock_assets(&t.user, &0i128).is_err());
}

#[test]
fn test_unlock_assets_rejects_more_than_locked() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    assert!(t.client.try_unlock_assets(&t.user, &1_001i128).is_err());
}

#[test]
fn test_unlock_assets_rejects_when_no_position() {
    let t = setup(1, 1);
    assert!(t.client.try_unlock_assets(&t.user, &100i128).is_err());
}

#[test]
fn test_unlock_assets_emits_event() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 5);
    t.client.unlock_assets(&t.user, &1_000);

    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("unlocked").into_val(&t.env)
                ],
                (t.user.clone(), 1_000i128, 5_000i128).into_val(&t.env),
            )
        ]
    );
}

// ── minimum lock period tests ─────────────────────────────────────────────────

#[test]
fn test_unlock_blocked_before_min_lock_period() {
    let t = setup_with_lock_period(1, 1, 100);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 50); // only 50 of 100 ledgers elapsed
    assert!(t.client.try_unlock_assets(&t.user, &1_000).is_err());
}

#[test]
fn test_unlock_allowed_after_min_lock_period() {
    let t = setup_with_lock_period(1, 1, 100);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 100); // Should succeed at exactly the boundary.
                                  // Should succeed — no panic.
    t.client.unlock_assets(&t.user, &1_000);
    assert!(t.client.get_user_position(&t.user).is_none());
}

#[test]
fn test_unlock_allowed_well_past_min_lock_period() {
    let t = setup_with_lock_period(1, 1, 10);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 500);
    t.client.unlock_assets(&t.user, &1_000);
    assert!(t.client.get_user_position(&t.user).is_none());
}

// ── calculate_credits tests ───────────────────────────────────────────────────

#[test]
fn test_min_lock_period_change_does_not_affect_existing_position_unlock_ledger() {
    let t = setup_with_lock_period(1, 1, 100);
    t.client.lock_assets(&t.user, &1_000);
    let position = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(position.unlock_ledger, position.lock_ledger + 100);

    t.client.set_min_lock_period(&5u32);
    advance_ledgers(&t.env, 50);
    assert!(t.client.try_unlock_assets(&t.user, &1_000).is_err());

    advance_ledgers(&t.env, 50);
    t.client.unlock_assets(&t.user, &1_000);
    assert!(t.client.get_user_position(&t.user).is_none());
}

#[test]
fn test_new_positions_use_updated_min_lock_period() {
    let t = setup_with_lock_period(1, 1, 100);
    t.client.set_min_lock_period(&5u32);
    t.client.lock_assets(&t.user, &1_000);

    let position = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(position.unlock_ledger, position.lock_ledger + 5);
}

#[test]
fn test_calculate_credits_zero_without_position() {
    let t = setup(1, 1);
    assert_eq!(t.client.calculate_credits(&t.user), 0);
}

#[test]
fn test_calculate_credits_accrues_over_time() {
    // credit_rate = 2, amount = 500, ledgers = 20 → credits = 500 * 2 * 20 = 20000
    let t = setup(1, 2);
    t.client.lock_assets(&t.user, &500);
    advance_ledgers(&t.env, 20);
    assert_eq!(t.client.calculate_credits(&t.user), 20_000);
}

#[test]
fn test_calculate_credits_includes_banked_plus_accruing() {
    // Lock, advance 10 (banked = 10000 at second lock), add more, advance 10 more.
    // Second period: (1000 + 500) * 1 * 10 = 15000. Total = 25000.
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.lock_assets(&t.user, &500); // banks 10000
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.calculate_credits(&t.user), 25_000);
}

#[test]
fn test_calculate_credits_reflects_partial_unlock_checkpoint() {
    // Lock 1000, advance 10 → 10000. Unlock 400 (banks 10000). Remaining 600 accrues.
    // Advance 5 more: 600 * 1 * 5 = 3000. Total banked+accruing = 10000 + 3000 = 13000.
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&t.user, &400); // banks 10000 into pos.total_credits
    advance_ledgers(&t.env, 5);
    assert_eq!(t.client.calculate_credits(&t.user), 13_000);
}

// ── get_user_position tests ───────────────────────────────────────────────────

#[test]
fn test_get_user_position_none_before_lock() {
    let t = setup(1, 1);
    assert!(t.client.get_user_position(&t.user).is_none());
}

#[test]
fn test_get_user_position_returns_correct_fields() {
    let t = setup(1, 1);
    let start = t.env.ledger().sequence();
    t.client.lock_assets(&t.user, &750);
    let pos = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(pos.amount, 750);
    assert_eq!(pos.lock_ledger, start);
    assert_eq!(pos.unlock_ledger, start);
    assert_eq!(pos.checkpoint_ledger, start);
    assert_eq!(pos.total_credits, 0);
    assert_eq!(pos.credit_rate, 1);
}

#[test]
fn test_get_user_position_none_after_full_unlock() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 5);
    t.client.unlock_assets(&t.user, &1_000);
    assert!(t.client.get_user_position(&t.user).is_none());
}

// ── pause / unpause tests ─────────────────────────────────────────────────────

#[test]
fn test_pool_not_paused_initially() {
    let t = setup(1, 1);
    assert!(!t.client.is_paused());
}

#[test]
fn test_pause_blocks_lock_assets() {
    let t = setup(1, 1);
    t.client.pause();
    assert!(t.client.is_paused());
    assert!(t.client.try_lock_assets(&t.user, &100i128).is_err());
    assert!(t.client.is_paused());

    match t.client.try_lock_assets(&t.user, &100i128) {
        Err(Ok(PoolError::Paused)) => {}
        other => panic!("expected PoolError::Paused, got: {:?}", other),
    }
}

#[test]
fn test_pause_blocks_unlock_assets() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    t.client.pause();

    match t.client.try_unlock_assets(&t.user, &1_000) {
        Err(Ok(PoolError::Paused)) => {}
        other => panic!("expected PoolError::Paused, got: {:?}", other),
    }
}

#[test]
fn test_unpause_restores_operations() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    assert!(!t.client.is_paused());
    // Lock and unlock should work again.
    t.client.lock_assets(&t.user, &500);
    t.client.unlock_assets(&t.user, &500);
}

#[test]
fn test_pause_emits_event() {
    let t = setup(1, 1);
    t.client.pause();
    assert!(
        !t.env.events().all().events().is_empty(),
        "pause event not emitted"
    );
}

#[test]
fn test_unpause_emits_event() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    assert!(
        !t.env.events().all().events().is_empty(),
        "unpause event not emitted"
    );
}

#[test]
fn test_pause_blocks_stake() {
    let t = setup(1, 1);
    t.client.pause();

    match t.client.try_stake(&t.user, &100i128) {
        Err(Ok(PoolError::Paused)) => {}
        other => panic!("expected PoolError::Paused, got: {:?}", other),
    }
}

#[test]
fn test_unpause_restores_stake() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    t.client.stake(&t.user, &500);
    assert_eq!(t.client.get_stake(&t.user).unwrap().amount, 500);
}

#[test]
fn test_pause_blocks_unstake() {
    let t = setup(1, 1);
    t.client.stake(&t.user, &1_000);
    t.client.pause();
    assert!(t.client.try_unstake(&t.user).is_err());
}

#[test]
fn test_unpause_restores_unstake() {
    let t = setup(1, 1);
    t.client.stake(&t.user, &1_000);
    t.client.pause();
    t.client.unpause();
    t.client.unstake(&t.user);
    assert!(t.client.get_stake(&t.user).is_none());
}

#[test]
fn test_pause_blocks_set_boost() {
    let t = setup(1, 1);
    t.client.stake(&t.user, &1_000);
    t.client.pause();
    assert!(t.client.try_set_boost(&t.user, &50u32).is_err());
}

#[test]
fn test_unpause_restores_set_boost() {
    let t = setup(1, 1);
    t.client.stake(&t.user, &1_000);
    t.client.pause();
    t.client.unpause();
    t.client.set_boost(&t.user, &50u32);
    assert_eq!(
        t.client.get_boost_config(&t.user).unwrap().allocation_pct,
        50
    );
}

#[test]
fn test_set_global_multiplier_callable_while_paused() {
    let t = setup(1, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    t.client.pause();
    t.client.set_global_multiplier(&3u32);
    assert_eq!(t.client.get_boost_config(&t.user).unwrap().multiplier, 3);
}

// ── multi-user isolation ──────────────────────────────────────────────────────

#[test]
fn test_multiple_users_independent_positions() {
    let t = setup(1, 1);
    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &500_000i128);
    t.client.lock_assets(&t.user, &1_000);
    t.client.lock_assets(&user2, &2_000);
    advance_ledgers(&t.env, 10);
    // Each user's credits are independent.
    assert_eq!(t.client.calculate_credits(&t.user), 10_000); // 1000 * 10
    assert_eq!(t.client.calculate_credits(&user2), 20_000); // 2000 * 10
}

#[test]
fn test_one_user_unlock_does_not_affect_another() {
    let t = setup(1, 1);
    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &500_000i128);
    t.client.lock_assets(&t.user, &1_000);
    t.client.lock_assets(&user2, &2_000);
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&t.user, &1_000);
    // user2's position is untouched.
    let pos2 = t
        .client
        .get_user_position(&user2)
        .expect("user2 position should exist");
    assert_eq!(pos2.amount, 2_000);
}

// ── emergency_withdraw tests ──────────────────────────────────────────────────

#[test]
fn test_emergency_withdraw_while_paused() {
    let t = setup(1, 1);
    let initial_balance = t.token.balance(&t.user);
    // Lock 500, stake 300, advance 10 ledgers so credits accrue.
    t.client.lock_assets(&t.user, &500);
    t.client.stake(&t.user, &300);
    advance_ledgers(&t.env, 10);
    // Trigger credit checkpoints: second lock banks 500*1*10=5_000 into pos.total_credits;
    // second stake banks 300*1*10=3_000 into stake.credits_banked.
    t.client.lock_assets(&t.user, &100);
    t.client.stake(&t.user, &100);
    t.client.pause();
    let returned = t.client.emergency_withdraw(&t.user);
    // 600 locked + 400 staked = 1_000 total tokens returned.
    assert_eq!(returned, 1_000);
    assert_eq!(t.token.balance(&t.user), initial_balance);
    assert!(
        t.client.get_user_position(&t.user).is_none(),
        "position should be cleared"
    );
    assert!(
        t.client.get_stake(&t.user).is_none(),
        "stake should be cleared"
    );
    // 5_000 (lock credits) + 3_000 (stake credits) preserved.
    assert_eq!(t.client.get_banked_credits(&t.user), 8_000);
}

#[test]
fn test_emergency_withdraw_while_unpaused_returns_not_paused() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    let result = t.client.try_emergency_withdraw(&t.user);
    assert!(matches!(result, Err(Ok(PoolError::NotPaused))));
}

// ── Whitelist system tests ───────────────────────────────────────────────────

#[test]
fn test_whitelist_disabled_by_default_allows_all() {
    let t = setup(2, 1);
    // User can stake when whitelist is not enabled (disabled by default)
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.get_stake(&t.user).unwrap().amount, 1_000);

    // User can lock when whitelist is not enabled
    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &500_000i128);
    t.client.lock_assets(&user2, &500);
    assert_eq!(t.client.get_user_position(&user2).unwrap().amount, 500);
}

#[test]
fn test_whitelist_blocks_unstaked_locked_unapproved() {
    let t = setup(2, 1);
    t.client.enable_whitelist();

    // Unapproved user cannot stake
    let result_stake = t.client.try_stake(&t.user, &1_000);
    assert!(matches!(result_stake, Err(Ok(PoolError::NotWhitelisted))));

    // Unapproved user cannot lock
    let result_lock = t.client.try_lock_assets(&t.user, &500);
    assert!(matches!(result_lock, Err(Ok(PoolError::NotWhitelisted))));
}

#[test]
fn test_user_added_and_removed_from_whitelist() {
    let t = setup(2, 1);
    t.client.enable_whitelist();

    // Initially not whitelisted
    assert!(!t.client.is_whitelisted(&t.user));

    // Add user to whitelist
    t.client.add_to_whitelist(&t.user);
    assert!(t.client.is_whitelisted(&t.user));

    // Now user can stake and lock
    t.client.stake(&t.user, &1_000);
    t.client.lock_assets(&t.user, &500);
    assert_eq!(t.client.get_stake(&t.user).unwrap().amount, 1_000);
    assert_eq!(t.client.get_user_position(&t.user).unwrap().amount, 500);

    // Remove user from whitelist
    t.client.remove_from_whitelist(&t.user);
    assert!(!t.client.is_whitelisted(&t.user));

    // Removed user cannot stake additional tokens or lock additional tokens
    let result_stake = t.client.try_stake(&t.user, &500);
    assert!(matches!(result_stake, Err(Ok(PoolError::NotWhitelisted))));
}

#[test]
fn test_disable_whitelist_restores_open_access() {
    let t = setup(2, 1);
    t.client.enable_whitelist();

    // Blocked initially
    assert!(t.client.try_stake(&t.user, &1_000).is_err());

    // Disable whitelist
    t.client.disable_whitelist();

    // Stake succeeds now
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.get_stake(&t.user).unwrap().amount, 1_000);
}

#[test]
fn test_batch_add_to_whitelist() {
    let t = setup(2, 1);
    t.client.enable_whitelist();

    let user1 = Address::generate(&t.env);
    let user2 = Address::generate(&t.env);
    let user3 = Address::generate(&t.env);

    t.token_sac.mint(&user1, &500_000i128);
    t.token_sac.mint(&user2, &500_000i128);
    t.token_sac.mint(&user3, &500_000i128);

    let mut users = soroban_sdk::Vec::new(&t.env);
    users.push_back(user1.clone());
    users.push_back(user2.clone());
    users.push_back(user3.clone());

    t.client.batch_add_to_whitelist(&users);

    assert!(t.client.is_whitelisted(&user1));
    assert!(t.client.is_whitelisted(&user2));
    assert!(t.client.is_whitelisted(&user3));

    // They can all stake
    t.client.stake(&user1, &100);
    t.client.stake(&user2, &100);
    t.client.stake(&user3, &100);

    assert_eq!(t.client.get_stake(&user1).unwrap().amount, 100);
    assert_eq!(t.client.get_stake(&user2).unwrap().amount, 100);
    assert_eq!(t.client.get_stake(&user3).unwrap().amount, 100);
}

#[test]
#[should_panic(expected = "max 50 addresses per call")]
fn test_batch_add_to_whitelist_exceeds_limit() {
    let t = setup(2, 1);
    let mut users = soroban_sdk::Vec::new(&t.env);
    for _ in 0..51 {
        users.push_back(Address::generate(&t.env));
    }
    t.client.batch_add_to_whitelist(&users);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_set_min_stake_amount_lock_assets() {
    let t = setup(1, 1);

    t.client.lock_assets(&t.user, &10i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_set_min_stake_amount_stake() {
    let t = setup(1, 1);

    t.client.stake(&t.user, &50);
}

#[test]
fn test_set_min_stake_amount_lock_assets_pass() {
    let t = setup(1, 1);

    t.client.lock_assets(&t.user, &100i128);
}

#[test]
fn test_set_min_stake_amount() {
    let t = setup(1, 1);

    let min_stake = t.client.get_min_stake_amount();
    assert_eq!(min_stake, 100);

    let amount = 200_i128;

    t.client.set_min_stake_amount(&amount);
    let min_stake = t.client.get_min_stake_amount();
    assert_eq!(min_stake, amount);
}
// ── lock_assets checks-effects-interactions (#69) ─────────────────────────────
//
// `stake_token` is an admin-supplied address, not necessarily a trusted
// Stellar Asset Contract — a non-standard `transfer` implementation could
// attempt to call back into FarmingPool before returning. Empirically
// (verified against this project's soroban-env-host 25.0.1), Soroban's host
// already rejects same-contract reentrancy outright — `ContractReentryMode`
// defaults to `Prohibited`, so any attempt to call back into a contract
// that's already on the invocation's call stack traps with "Contract
// re-entry is not allowed" *before* any of our code (fixed or not) runs.
// These tests exercise both realistic reentrant-token shapes and confirm
// the CEI reordering fix doesn't change correct-path behavior.

use crate::mock_reentrant_token::{
    MockNaiveReentrantToken, MockNaiveReentrantTokenClient, MockReentrantToken,
    MockReentrantTokenClient,
};

extern crate std;

#[test]
fn test_lock_assets_reentrant_transfer_is_rejected_and_final_state_is_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    // Succeeds fully: the mock token catches the rejected reentry gracefully
    // (via try_invoke_contract) rather than trapping the whole call.
    client.lock_assets(&user, &500i128);

    // The reentrant get_user_position call — attempted mid-transfer, before
    // lock_assets would have returned — was rejected by the host.
    assert!(token_client.reentry_was_rejected());

    // And with set_position now happening before the transfer, the position
    // this call was computing is correctly persisted once it completes.
    let position = client.get_user_position(&user).unwrap();
    assert_eq!(position.amount, 500);
}

#[test]
fn test_lock_assets_reverts_entirely_if_stake_token_naively_reenters() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockNaiveReentrantToken, ());
    let token_client = MockNaiveReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    // The naive mock token doesn't catch the host's rejection, so the
    // reentrant call traps — and with it, the entire lock_assets invocation,
    // including our set_position write. Assert the whole call aborts rather
    // than silently succeeding or leaving a partial state behind.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.lock_assets(&user, &500i128);
    }));
    assert!(
        result.is_err(),
        "lock_assets should trap when stake_token attempts reentrancy"
    );

    // Soroban's per-invocation atomicity means the trap rolled back
    // everything, including the effects-first set_position write — no
    // partial position was left behind.
    assert!(client.get_user_position(&user).is_none());
}

// ── stake / unstake / unlock_assets / emergency_withdraw reentrancy
// coverage (#128) ──────────────────────────────────────────────────────────
//
// `lock_assets` was the only function with MockReentrantToken/
// MockNaiveReentrantToken coverage, despite `stake`, `unstake`,
// `unlock_assets`, and `emergency_withdraw` sharing the identical threat
// model documented above `lock_assets` (an admin-supplied, not necessarily
// trusted `stake_token`) — and despite being exactly the functions #70,
// #71, and #72 flag for transfer-before-state-update ordering, which makes
// this coverage doubly valuable as a regression suite once those land.
//
// Pre-existing Position/UserStake state some of these tests need is seeded
// directly via `set_position`/`set_user_stake` under `env.as_contract`
// rather than by first calling `lock_assets`/`stake` normally: `stake_token`
// is fixed at `initialize` and has no setter, so a token already configured
// to reenter would also reenter during that setup call. For the graceful
// mock that's merely redundant; for the naive mock it's fatal — that setup
// call would itself trap, long before the function actually under test ever
// runs.

fn seed_position(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        let current = env.ledger().sequence();
        set_position(
            env,
            user,
            &Position {
                amount,
                lock_ledger: current,
                unlock_ledger: current,
                checkpoint_ledger: current,
                total_credits: 0,
                credit_rate: read_credit_rate(env),
            },
        );
    });
}

fn seed_user_stake(env: &Env, contract_id: &Address, user: &Address, amount: i128) {
    env.as_contract(contract_id, || {
        let current = env.ledger().sequence();
        set_user_stake(
            env,
            user,
            &UserStake {
                amount,
                start_ledger: current,
                credits_banked: 0,
                credit_rate: read_credit_rate(env),
            },
        );
    });
}

// ── stake (top-up branch) ──────────────────────────────────────────────────

#[test]
fn test_stake_topup_reentrant_transfer_is_rejected_and_final_state_is_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    // Pre-existing stake to top up — the branch with more state a reentrant
    // call could observe mid-update, per the issue's own callout.
    seed_user_stake(&env, &farming_pool_id, &user, 500i128);

    // Reenter the same state-mutating function under test, not just a
    // read-only getter — the more realistic worst-case reentry attempt.
    let reentrant_args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&env, user.clone().into_val(&env), 200i128.into_val(&env)];
    token_client.configure_reentrant_call(&Symbol::new(&env, "stake"), &reentrant_args);

    client.stake(&user, &200i128);

    assert!(token_client.reentry_was_rejected());

    let stake = client.get_stake(&user).unwrap();
    assert_eq!(stake.amount, 700);
}

#[test]
fn test_stake_reverts_entirely_if_stake_token_naively_reenters() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockNaiveReentrantToken, ());
    let token_client = MockNaiveReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_user_stake(&env, &farming_pool_id, &user, 500i128);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.stake(&user, &200i128);
    }));
    assert!(
        result.is_err(),
        "stake should trap when stake_token attempts reentrancy"
    );

    // Trap rolled back the whole call, including set_user_stake — the
    // seeded stake is untouched, no top-up applied.
    let stake = client.get_stake(&user).unwrap();
    assert_eq!(stake.amount, 500);
}

// ── unstake ─────────────────────────────────────────────────────────────────

#[test]
fn test_unstake_reentrant_transfer_is_rejected_and_final_state_is_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_user_stake(&env, &farming_pool_id, &user, 500i128);

    let reentrant_args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&env, user.clone().into_val(&env)];
    token_client.configure_reentrant_call(&Symbol::new(&env, "unstake"), &reentrant_args);

    client.unstake(&user);

    assert!(token_client.reentry_was_rejected());
    assert!(client.get_stake(&user).is_none());
}

#[test]
fn test_unstake_reverts_entirely_if_stake_token_naively_reenters() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockNaiveReentrantToken, ());
    let token_client = MockNaiveReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_user_stake(&env, &farming_pool_id, &user, 500i128);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.unstake(&user);
    }));
    assert!(
        result.is_err(),
        "unstake should trap when stake_token attempts reentrancy"
    );

    // Trap rolled back the whole call — the seeded stake is still present,
    // unstake never actually completed.
    let stake = client.get_stake(&user).unwrap();
    assert_eq!(stake.amount, 500);
}

// ── unlock_assets ───────────────────────────────────────────────────────────

#[test]
fn test_unlock_assets_reentrant_transfer_is_rejected_and_final_state_is_correct() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_position(&env, &farming_pool_id, &user, 500i128);

    let reentrant_args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&env, user.clone().into_val(&env), 200i128.into_val(&env)];
    token_client.configure_reentrant_call(&Symbol::new(&env, "unlock_assets"), &reentrant_args);

    client.unlock_assets(&user, &200i128);

    assert!(token_client.reentry_was_rejected());

    let position = client.get_user_position(&user).unwrap();
    assert_eq!(position.amount, 300);
}

#[test]
fn test_unlock_assets_reverts_entirely_if_stake_token_naively_reenters() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockNaiveReentrantToken, ());
    let token_client = MockNaiveReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_position(&env, &farming_pool_id, &user, 500i128);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.unlock_assets(&user, &200i128);
    }));
    assert!(
        result.is_err(),
        "unlock_assets should trap when stake_token attempts reentrancy"
    );

    // Trap rolled back the whole call — the seeded position is untouched,
    // no partial unlock was applied.
    let position = client.get_user_position(&user).unwrap();
    assert_eq!(position.amount, 500);
}

// ── emergency_withdraw ───────────────────────────────────────────────────────
//
// Makes two separate token.transfer calls in sequence when a user has both
// a Position and a UserStake — the two tests below target reentry during
// the first (Position-only) and second (Position + UserStake) transfer
// respectively, since `reentry_was_rejected()` reflects the most recent
// `transfer` call.

#[test]
fn test_emergency_withdraw_reentrant_transfer_during_position_payout_is_rejected_and_final_state_is_correct(
) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_position(&env, &farming_pool_id, &user, 500i128);
    client.pause();

    let reentrant_args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&env, user.clone().into_val(&env)];
    token_client
        .configure_reentrant_call(&Symbol::new(&env, "emergency_withdraw"), &reentrant_args);

    let returned = client.emergency_withdraw(&user);

    assert!(token_client.reentry_was_rejected());
    assert_eq!(returned, 500);
    assert!(client.get_user_position(&user).is_none());
}

#[test]
fn test_emergency_withdraw_reentrant_transfer_during_stake_payout_is_rejected_and_final_state_is_correct(
) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockReentrantToken, ());
    let token_client = MockReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_position(&env, &farming_pool_id, &user, 500i128);
    seed_user_stake(&env, &farming_pool_id, &user, 300i128);
    client.pause();

    let reentrant_args: soroban_sdk::Vec<Val> =
        soroban_sdk::vec![&env, user.clone().into_val(&env)];
    token_client
        .configure_reentrant_call(&Symbol::new(&env, "emergency_withdraw"), &reentrant_args);

    let returned = client.emergency_withdraw(&user);

    assert!(token_client.reentry_was_rejected());
    assert_eq!(returned, 800);
    assert!(client.get_user_position(&user).is_none());
    assert!(client.get_stake(&user).is_none());
}

#[test]
fn test_emergency_withdraw_reverts_entirely_if_stake_token_naively_reenters() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let farming_pool_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &farming_pool_id);

    let token_id = env.register(MockNaiveReentrantToken, ());
    let token_client = MockNaiveReentrantTokenClient::new(&env, &token_id);
    token_client.configure(&farming_pool_id, &user);

    client.initialize(&admin, &token_id, &2u32, &100i128, &0u32, &0i128);

    seed_position(&env, &farming_pool_id, &user, 500i128);
    seed_user_stake(&env, &farming_pool_id, &user, 300i128);
    client.pause();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.emergency_withdraw(&user);
    }));
    assert!(
        result.is_err(),
        "emergency_withdraw should trap when stake_token attempts reentrancy"
    );

    // Trap rolled back everything — both seeded records are untouched, and
    // neither of emergency_withdraw's two transfers actually completed.
    let position = client.get_user_position(&user).unwrap();
    assert_eq!(position.amount, 500);
    let stake = client.get_stake(&user).unwrap();
    assert_eq!(stake.amount, 300);
}

// ── Credit-accrual invariants, property-based (#75) ──────────────────────────
//
// Shared notation used by every derivation below. Writing
// `B(a, p) = floor(a * p / 100)` for the boosted portion computed by
// `compute_total_stake`, that function reduces algebraically to
//
//     S(a, p, m) = (a - B(a, p)) + B(a, p) * m
//                = a + B(a, p) * (m - 1)
//
// and `compute_credits` is `S(a, p, m) * r * elapsed`. `B` is the *only*
// nonlinearity in the whole accrual path — everything else is integer
// multiplication and addition, which are exact.
//
// Where truncation could bite, and by how much. For a partition
// `A = a_1 + ... + a_n`, write `a_i * p / 100 = q_i + f_i` with `f_i` in
// `[0, 1)`. Then `sum_i B(a_i, p) = sum_i q_i` while
// `B(A, p) = floor(sum_i (q_i + f_i)) = sum_i q_i + floor(sum_i f_i)`, so
//
//     B(A, p) - sum_i B(a_i, p) = floor(sum_i f_i)  in  [0, n - 1]
//
// (upper bound `n - 1` because each `f_i <= 99/100`, so `sum f_i < n`; tight at
// `p = 99, a_i = 1, n = 100`). An implementation that banked each deposit's
// boosted portion *separately* would therefore lose up to
// `(n - 1) * (m - 1) * r * elapsed` credits relative to one that aggregates
// first. That is the loss issue #75 anticipated.
//
// It is not reachable by splitting a deposit, because `stake` performs
// `existing.amount += amount` *before* `B` is next evaluated, and every call
// site of `compute_total_stake` passes the cumulative `stake.amount`. `B` is
// only ever applied to the aggregate. Properties A and B below are therefore
// *strict equalities*, not tolerance-bounded comparisons: an epsilon of
// `(n - 1) * (m - 1) * r * elapsed` would be blind to a regression of exactly
// that size, which is the regression these tests exist to catch.
//
// Preconditions shared by A and B, outside which they are false *by design*:
//   * `credit_rate` fixed. `checkpoint` snapshots the rate into `UserStake`, so
//     a mid-flight change is priced from the next checkpoint onward — see
//     `test_credit_rate_change_does_not_retroactively_alter_staked_credits`.
//   * `global_multiplier` fixed. See the "Property E" note at the end of this
//     section: unlike the rate, the multiplier is *not* snapshotted.
//   * `allocation_pct` fixed and set before the first stake, so that
//     `checkpoint`'s `get_user_boost` read is the same value at every call.

use proptest::prelude::*;

/// Case count for every property in this section. Deliberately modest: each
/// case registers a Stellar Asset Contract plus a pool instance and drives
/// several contract invocations, so cost per case is milliseconds rather than
/// microseconds. 64 is enough to explore the truncating region of `B` densely
/// while keeping the whole section well under a second — see #75's acceptance
/// criterion on CI runtime.
const PROP_CASES: u32 = 64;

/// Upper bound on a single generated deposit.
///
/// Chosen to stay far inside #89's overflow budget rather than to probe it:
/// with `multiplier <= MAX_GLOBAL_MULTIPLIER`, `credit_rate <= MAX_CREDIT_RATE`,
/// at most `MAX_DEPOSITS` deposits and a horizon under 10^5 ledgers, the worst
/// case is `6e12 * 1e3 * 1e8 * 1e5 ~= 4.5e28`, about ten orders of magnitude
/// below `i128::MAX ~= 1.7e38`. Overflow boundaries are #76's subject, not
/// this file's — these properties must fail on *accounting* errors only.
const MAX_DEPOSIT: i128 = 1_000_000_000_000;

const MAX_DEPOSITS: usize = 6;
const MAX_CHECKPOINTS: usize = 8;

/// Largest gap, in ledgers, generated between two timeline steps.
const MAX_GAP: u32 = 5_000;

/// A pool configured for property testing.
///
/// Differs from [`setup`] in three ways that matter:
///  * `capture_snapshot_at_drop: false` — soroban writes a
///    `test_snapshots/<test>.N.json` file for every `Env` it drops
///    (soroban-sdk `src/env.rs`, `impl Drop for Env`). With one `Env` per
///    proptest case per path that would be hundreds of JSON files per run.
///  * `min_stake_amount = 1` — the loosest legal setting (`get_min_stake_amount`
///    itself defaults to 1), which is also the most adversarial: it lets a
///    deposit be split into parts as small as a single unit, maximising the
///    number of independent `floor` roundings a per-deposit implementation
///    would incur.
///  * a much larger mint, so a six-deposit schedule at `MAX_DEPOSIT` each is
///    always affordable.
fn setup_for_props(global_multiplier: u32, credit_rate: i128) -> TestEnv {
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&user, &i128::from(u64::MAX));

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &asset.address(),
        &global_multiplier,
        &credit_rate,
        &0u32,
        &1i128,
    );

    let token = TokenClient::new(&env, &asset.address());

    // SAFETY: as in `setup_with_lock_period` — `env` owns the contract and
    // token registrations, so they outlive the returned struct.
    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };
    let token = unsafe { core::mem::transmute::<TokenClient<'_>, TokenClient<'static>>(token) };
    let token_sac = unsafe {
        core::mem::transmute::<StellarAssetClient<'_>, StellarAssetClient<'static>>(token_sac)
    };

    TestEnv {
        env,
        client,
        contract_id,
        token,
        token_sac,
        admin,
        user,
    }
}

/// One event on a generated timeline: `gap` ledgers elapse, then `action`.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// Stake this amount.
    Deposit(i128),
    /// Re-apply the *same* `allocation_pct`, which forces `checkpoint` to run
    /// (`set_boost` checkpoints before writing) without altering any input to
    /// the accrual formula. The purest available no-op checkpoint.
    Checkpoint,
}

type Step = (u32, Action);

/// Independent oracle for the credits a schedule should produce.
///
/// Written in the *reduced* form `S = a + B(a, p) * (m - 1)` derived at the top
/// of this section, rather than by calling `compute_total_stake`, so that these
/// properties are oracle tests rather than merely differential ones: two paths
/// through the contract cannot agree with each other *and* with this expression
/// while both being wrong in the same way.
///
/// The oracle deliberately keeps the `floor` — it is not a rational-arithmetic
/// reference. Its job is to pin the *aggregation structure* (`B` applied once
/// per checkpoint to the running total, credits summed over intervals), which
/// is exactly what deposit granularity and checkpoint frequency could perturb.
fn oracle_total_stake(amount: i128, allocation_pct: u32, multiplier: u32) -> i128 {
    amount + (amount * allocation_pct as i128 / 100) * (multiplier as i128 - 1)
}

/// Credits accrued by `steps` followed by `tail_gap` further ledgers, given a
/// boost set before the first deposit and a fixed multiplier and rate.
///
/// `sum_j S(A_j, p, m) * r * (t_{j+1} - t_j)` where `A_j` is the amount held
/// during interval `j`. `Checkpoint` steps contribute nothing beyond their gap,
/// which is precisely Property B's claim.
fn oracle_credits(
    steps: &[Step],
    allocation_pct: u32,
    multiplier: u32,
    credit_rate: i128,
    tail_gap: u32,
) -> i128 {
    let mut held = 0i128;
    let mut total = 0i128;
    for &(gap, action) in steps {
        // `gap` ledgers elapse while `held` is staked. Before the first
        // deposit `held` is 0 and the interval contributes nothing.
        total += oracle_total_stake(held, allocation_pct, multiplier) * credit_rate * gap as i128;
        if let Action::Deposit(amount) = action {
            held += amount;
        }
    }
    total += oracle_total_stake(held, allocation_pct, multiplier) * credit_rate * tail_gap as i128;
    total
}

/// Split `amount` into `weights.len()` positive parts in the given proportions.
///
/// Every part is at least 1 (the pool's `min_stake_amount`), and the parts sum
/// to exactly `amount`, so the split changes deposit *granularity* and nothing
/// else. The caller must ensure `1 <= weights.len() <= amount`.
fn partition(amount: i128, weights: &[u32]) -> std::vec::Vec<i128> {
    let n = weights.len() as i128;
    debug_assert!(n >= 1 && n <= amount);
    let weight_sum: i128 = weights.iter().map(|&w| i128::from(w)).sum();

    let mut assigned = 0i128;
    let mut parts = std::vec::Vec::with_capacity(weights.len());
    for (i, &w) in weights.iter().enumerate().take(weights.len() - 1) {
        // Leave at least 1 unit for each part still to come.
        let reserved = n - 1 - i as i128;
        let part = (amount * i128::from(w) / weight_sum).clamp(1, amount - assigned - reserved);
        parts.push(part);
        assigned += part;
    }
    parts.push(amount - assigned);
    parts
}

// ── Generators ───────────────────────────────────────────────────────────────
//
// Every range below is the *reachable* range for that input, so the properties
// never assert anything about a state the contract would have rejected:
//
//   multiplier     1..=MAX_GLOBAL_MULTIPLIER  (lib.rs, `set_global_multiplier`)
//   credit_rate    1..=MAX_CREDIT_RATE        (lib.rs, `set_credit_rate`)
//   allocation_pct 1..=100                    (lib.rs, `set_boost`)
//   deposit        >= min_stake_amount, > 0   (lib.rs, `stake`)
//
// Both ceilings are #89's landed constants, imported rather than re-stated, so
// this file tracks any future change to them automatically.

/// `2..=MAX_GLOBAL_MULTIPLIER`, not `1..=`.
///
/// At `multiplier == 1`, `S(a, p, m) = a` identically and the `floor` in `B`
/// cannot affect the result at all — a split-invariance property would pass
/// vacuously. Properties A and B exist to exercise the truncating branch, so
/// they exclude the one value that switches it off. Property D, which makes no
/// claim about truncation, keeps the full `1..=` range.
fn boosting_multiplier() -> impl Strategy<Value = u32> {
    2..=MAX_GLOBAL_MULTIPLIER
}

fn any_multiplier() -> impl Strategy<Value = u32> {
    1..=MAX_GLOBAL_MULTIPLIER
}

fn any_credit_rate() -> impl Strategy<Value = i128> {
    1..=MAX_CREDIT_RATE
}

fn any_allocation_pct() -> impl Strategy<Value = u32> {
    1..=100u32
}

fn any_deposit() -> impl Strategy<Value = i128> {
    1..=MAX_DEPOSIT
}

fn any_gap() -> impl Strategy<Value = u32> {
    0..=MAX_GAP
}

/// A timeline mixing deposits and no-op checkpoints.
fn any_steps() -> impl Strategy<Value = std::vec::Vec<Step>> {
    let action = prop_oneof![
        3 => any_deposit().prop_map(Action::Deposit),
        1 => Just(Action::Checkpoint),
    ];
    proptest::collection::vec((any_gap(), action), 1..=(MAX_DEPOSITS + MAX_CHECKPOINTS))
}

/// A deposits-only timeline, paired with per-deposit split weights.
fn any_deposit_schedule() -> impl Strategy<Value = std::vec::Vec<(u32, i128, std::vec::Vec<u32>)>> {
    proptest::collection::vec(
        (
            any_gap(),
            any_deposit(),
            proptest::collection::vec(1..=1_000u32, 1..=4),
        ),
        1..=MAX_DEPOSITS,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(PROP_CASES))]

    /// PROPERTY A — total credits do not depend on deposit granularity.
    ///
    /// Claim: for a fixed schedule of deposits `(t_j, amt_j)`, subdividing any
    /// `amt_j` into parts staked at the *same* ledger `t_j` leaves total
    /// credits unchanged at every later ledger. Tolerance: **exactly zero**.
    ///
    /// Derivation. Two facts about a second `stake` call at the same ledger:
    ///
    ///  1. It checkpoints with `elapsed = current - stake.start_ledger = 0`,
    ///     so `credits_banked += S(...) * r * 0 = 0` and `start_ledger` is
    ///     rewritten to the value it already held. The bank is untouched, and
    ///     — crucially — `B` never sees the individual part.
    ///  2. `existing.amount += amount` aggregates *before* `B` is next
    ///     evaluated, at the following checkpoint or read.
    ///
    /// So after subdividing, `UserStake` is field-for-field identical:
    /// `{amount, start_ledger, credits_banked, credit_rate}` all match. By
    /// induction over the schedule every subsequent checkpoint observes
    /// identical state, hence identical credits. No rounding is introduced,
    /// which is why this is `==` and not `<= epsilon`.
    ///
    /// What it catches: a refactor that banked each deposit's boosted portion
    /// separately would lose `floor(sum_i f_i) * (m - 1) * r * elapsed`
    /// credits, up to `(n - 1) * (m - 1) * r * elapsed` — the bound derived at
    /// the top of this section. Strict equality detects that at `n = 2`.
    #[test]
    fn prop_credits_are_independent_of_deposit_granularity(
        multiplier in boosting_multiplier(),
        credit_rate in any_credit_rate(),
        allocation_pct in any_allocation_pct(),
        schedule in any_deposit_schedule(),
        tail_gap in 1..=MAX_GAP,
    ) {
        // Coarse path: one `stake` call per scheduled deposit.
        let coarse = {
            let t = setup_for_props(multiplier, credit_rate);
            t.client.set_boost(&t.user, &allocation_pct);
            for &(gap, amount, _) in &schedule {
                advance_ledgers(&t.env, gap);
                t.client.stake(&t.user, &amount);
            }
            advance_ledgers(&t.env, tail_gap);
            t.client.get_credits(&t.user)
        };

        // Fine path: identical timeline, but each deposit arrives as several
        // same-ledger calls summing to the same amount.
        let fine = {
            let t = setup_for_props(multiplier, credit_rate);
            t.client.set_boost(&t.user, &allocation_pct);
            for (gap, amount, weights) in &schedule {
                advance_ledgers(&t.env, *gap);
                // `partition` needs at least one unit per part.
                let parts = (weights.len() as i128).min(*amount).max(1) as usize;
                for part in partition(*amount, &weights[..parts]) {
                    t.client.stake(&t.user, &part);
                }
            }
            advance_ledgers(&t.env, tail_gap);
            t.client.get_credits(&t.user)
        };

        prop_assert_eq!(
            coarse, fine,
            "deposit granularity changed total credits (m={}, r={}, pct={})",
            multiplier, credit_rate, allocation_pct
        );

        // ...and both agree with the independent closed form, so the property
        // cannot be satisfied by two identically-wrong paths.
        let steps: std::vec::Vec<Step> = schedule
            .iter()
            .map(|&(gap, amount, _)| (gap, Action::Deposit(amount)))
            .collect();
        prop_assert_eq!(
            coarse,
            oracle_credits(&steps, allocation_pct, multiplier, credit_rate, tail_gap),
            "credits diverged from the closed-form oracle"
        );
    }

    /// PROPERTY B — total credits do not depend on checkpoint frequency.
    ///
    /// Claim: with `amount`, `allocation_pct`, `multiplier` and `credit_rate`
    /// fixed, inserting arbitrarily many extra no-op checkpoints at arbitrary
    /// ledgers leaves total credits unchanged. Tolerance: **exactly zero**.
    ///
    /// Derivation. Chop `[t_0, T]` at `t_0 < t_1 < ... < t_k`. Because
    /// `amount`, `p` and `m` are unchanged at every checkpoint, `S(A, p, m)`
    /// is the *same integer* `S` at each one, so the banked total is
    ///
    ///     sum_j S * r * (t_{j+1} - t_j) = S * r * sum_j (t_{j+1} - t_j)
    ///                                   = S * r * T
    ///
    /// Integer multiplication distributes exactly over integer addition, so
    /// partitioning *time* introduces no rounding whatsoever. Structurally:
    /// the only `floor` in the accrual path is `B(amount, pct)`, a function of
    /// amount and percentage alone — it is entirely independent of `elapsed`.
    /// This is why the property needs no epsilon even though #75 expected one.
    ///
    /// Both paths walk the identical ledger timeline; they differ only in
    /// whether the `Checkpoint` steps actually invoke the contract. Deposits
    /// are interleaved so the property is tested across changing amounts, not
    /// only for a single static stake.
    #[test]
    fn prop_credits_are_independent_of_checkpoint_frequency(
        multiplier in boosting_multiplier(),
        credit_rate in any_credit_rate(),
        allocation_pct in any_allocation_pct(),
        steps in any_steps(),
        tail_gap in 1..=MAX_GAP,
    ) {
        // Sparse path: checkpoint steps advance the ledger but do nothing.
        let sparse = {
            let t = setup_for_props(multiplier, credit_rate);
            t.client.set_boost(&t.user, &allocation_pct);
            for &(gap, action) in &steps {
                advance_ledgers(&t.env, gap);
                if let Action::Deposit(amount) = action {
                    t.client.stake(&t.user, &amount);
                }
            }
            advance_ledgers(&t.env, tail_gap);
            t.client.get_credits(&t.user)
        };

        // Dense path: same timeline, checkpoint steps actually checkpoint.
        let dense = {
            let t = setup_for_props(multiplier, credit_rate);
            t.client.set_boost(&t.user, &allocation_pct);
            for &(gap, action) in &steps {
                advance_ledgers(&t.env, gap);
                match action {
                    Action::Deposit(amount) => t.client.stake(&t.user, &amount),
                    // Re-applying the same pct checkpoints without changing
                    // any input to the accrual formula.
                    Action::Checkpoint => t.client.set_boost(&t.user, &allocation_pct),
                }
            }
            advance_ledgers(&t.env, tail_gap);
            t.client.get_credits(&t.user)
        };

        prop_assert_eq!(
            sparse, dense,
            "checkpoint frequency changed total credits (m={}, r={}, pct={})",
            multiplier, credit_rate, allocation_pct
        );

        prop_assert_eq!(
            sparse,
            oracle_credits(&steps, allocation_pct, multiplier, credit_rate, tail_gap),
            "credits diverged from the closed-form oracle"
        );
    }

    /// PROPERTY C — credits are strictly increasing in elapsed ledgers.
    ///
    /// Claim: for fixed stake, multiplier, rate and allocation, advancing the
    /// ledger by `d >= 1` strictly increases the reported credits; advancing by
    /// zero leaves them unchanged.
    ///
    /// Derivation. `get_credits = credits_banked + S(A, p, m) * r * elapsed`,
    /// and both factors of the slope are bounded below by 1:
    ///
    ///   * `S = A + B(A, p) * (m - 1) >= A >= 1`, since `stake` asserts
    ///     `amount > 0`, `B >= 0` and `m >= 1`;
    ///   * `r >= 1`, since `credit_rate` is constrained to
    ///     `1..=MAX_CREDIT_RATE`.
    ///
    /// So `delta = S * r * d >= 1 * 1 * 1 = 1 > 0`. Unlike A and B this needs
    /// no truncation argument at all: `elapsed` appears only as a multiplicand,
    /// never inside a division, so there is no rounding to reason about. The
    /// same argument gives the lock/`Position` path (`amount * rate * elapsed`)
    /// a slope of `amount * rate >= 1`.
    ///
    /// Crossing a checkpoint is also safe: at the instant of banking,
    /// `banked' = banked + S * r * elapsed` while `elapsed' = 0`, so the
    /// observable is continuous and the sequence stays monotone.
    #[test]
    fn prop_credits_are_strictly_increasing_in_elapsed_ledgers(
        multiplier in any_multiplier(),
        credit_rate in any_credit_rate(),
        allocation_pct in any_allocation_pct(),
        amount in any_deposit(),
        advances in proptest::collection::vec(1..=MAX_GAP, 1..=8),
    ) {
        let t = setup_for_props(multiplier, credit_rate);
        t.client.set_boost(&t.user, &allocation_pct);
        t.client.stake(&t.user, &amount);
        t.client.lock_assets(&t.user, &amount);

        let mut previous_staked = t.client.get_credits(&t.user);
        let mut previous_locked = t.client.calculate_credits(&t.user);

        // Advancing by zero must be a no-op for both credit systems.
        advance_ledgers(&t.env, 0);
        prop_assert_eq!(t.client.get_credits(&t.user), previous_staked);
        prop_assert_eq!(t.client.calculate_credits(&t.user), previous_locked);

        for delta in advances {
            advance_ledgers(&t.env, delta);

            let staked = t.client.get_credits(&t.user);
            prop_assert!(
                staked > previous_staked,
                "get_credits did not increase over {} ledgers: {} -> {}",
                delta, previous_staked, staked
            );

            let locked = t.client.calculate_credits(&t.user);
            prop_assert!(
                locked > previous_locked,
                "calculate_credits did not increase over {} ledgers: {} -> {}",
                delta, previous_locked, locked
            );

            previous_staked = staked;
            previous_locked = locked;
        }
    }

    /// PROPERTY D — credits are never negative, for any reachable input.
    ///
    /// Claim: `get_credits` and `calculate_credits` return a non-negative
    /// value for every input the contract's own validation admits.
    ///
    /// Derivation. `get_credits` is a sum of products of provably non-negative
    /// factors:
    ///
    ///   * `credits_banked` starts at 0 and only ever `+=` a `compute_credits`
    ///     result, so by induction it is non-negative;
    ///   * `principal = A - B(A, p) >= 0`, because `p <= 100` forces
    ///     `B(A, p) = floor(A * p / 100) <= A`;
    ///   * `virtual_stake = B(A, p) * m >= 0` since `m >= 1`;
    ///   * `r > 0` and `elapsed >= 0` (`saturating_sub` floors it at zero).
    ///
    /// "Reachable input" is what makes this a real test rather than a check on
    /// an unreachable state, so the generators mirror the contract's own
    /// constraints exactly: `amount > 0` and `>= min_stake_amount`
    /// (`stake`), `allocation_pct in 1..=100` (`set_boost`), `multiplier in
    /// 1..=MAX_GLOBAL_MULTIPLIER` and `credit_rate in 1..=MAX_CREDIT_RATE`
    /// (#89). Whitelisting is left disabled, its default. `multiplier == 1`
    /// and `allocation_pct == 100` are both kept in range: the latter is the
    /// `S = A * m` extreme, where `principal` reaches exactly zero.
    ///
    /// The zero-credit boundaries are covered too — a user with no position at
    /// all, and a user who has fully unstaked — since "never negative" has to
    /// hold at 0, not just above it.
    #[test]
    fn prop_credits_are_never_negative(
        multiplier in any_multiplier(),
        credit_rate in any_credit_rate(),
        allocation_pct in any_allocation_pct(),
        amount in any_deposit(),
        elapsed in 0..=MAX_GAP,
    ) {
        let t = setup_for_props(multiplier, credit_rate);
        let stranger = Address::generate(&t.env);

        // No stake and no locked position.
        prop_assert_eq!(t.client.get_credits(&stranger), 0);
        prop_assert_eq!(t.client.calculate_credits(&stranger), 0);

        t.client.set_boost(&t.user, &allocation_pct);
        t.client.stake(&t.user, &amount);
        t.client.lock_assets(&t.user, &amount);
        advance_ledgers(&t.env, elapsed);

        prop_assert!(t.client.get_credits(&t.user) >= 0);
        prop_assert!(t.client.calculate_credits(&t.user) >= 0);
        prop_assert!(t.client.get_banked_credits(&t.user) >= 0);

        // Unstaking banks and clears the record; the getter must return to 0
        // rather than to a negative residue.
        prop_assert!(t.client.unstake(&t.user) >= 0);
        prop_assert_eq!(t.client.get_credits(&t.user), 0);

        // Same for the lock system, which reports credits at withdrawal time.
        t.client.unlock_assets(&t.user, &amount);
        prop_assert_eq!(t.client.calculate_credits(&t.user), 0);
    }
}

// ── PROPERTY E — NOT IMPLEMENTED: needs maintainer confirmation (#75) ────────
//
// The obvious fourth invariant — "total credits are independent of checkpoint
// frequency across a *multiplier* change", the analogue of
// `test_credit_rate_change_does_not_retroactively_alter_staked_credits` — is
// deliberately absent, because the contract's intended semantics here are not
// settled and a property test either way would encode a guess as a passing
// test.
//
// The observation. `checkpoint` treats its two admin-controlled inputs
// asymmetrically:
//
//   * `credit_rate` is *snapshotted* into `UserStake.credit_rate` and the
//     already-elapsed segment is priced at the snapshot, so a mid-flight
//     `set_credit_rate` is not retroactive. That is asserted by
//     `test_credit_rate_change_does_not_retroactively_alter_staked_credits`.
//   * `global_multiplier` is *re-read* by `checkpoint` via
//     `read_global_multiplier` and applied to the already-elapsed segment. It
//     is not snapshotted anywhere, and `set_global_multiplier` cannot
//     checkpoint other users.
//
// The consequence is directly observable. With `multiplier = 2`,
// `allocation_pct = 100`, `amount = 1_000` and ten ledgers elapsed,
// `get_credits` reports 20_000. Calling `set_global_multiplier(3)` and
// advancing *zero* ledgers changes that reading to 30_000: the past segment
// was repriced. A user who happened to checkpoint before the change keeps
// 20_000 for that segment, so total credits *do* depend on checkpoint
// frequency whenever a multiplier change is in flight.
//
// Why no test is attached. `test_admin_multiplier_change_applies_from_next_
// checkpoint` is named as though snapshot semantics hold, but it inserts a
// no-op `set_boost` checkpoint immediately before bumping the multiplier, so
// it never exercises the mid-segment case. Nothing in the suite or the
// contract settles which behaviour is intended.
//
// QUESTION FOR THE MAINTAINER: should `global_multiplier` be snapshotted into
// `UserStake` alongside `credit_rate`, so that a multiplier change is priced
// from the next checkpoint onward and matches both the rate's documented
// semantics and the sibling test's name? Or is retroactive repricing of open
// segments the intended behaviour? Once that is answered, this becomes a
// strict-equality property in the same shape as Property B (with `multiplier`
// varying mid-timeline) or an explicit regression test pinning the retroactive
// behaviour — but not before.
