#![cfg(test)]
use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
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
fn test_stake_emits_event() {
    let t = setup(2, 1);
    let amount = 1_000i128;

    t.client.stake(&t.user, &amount);

    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("staked").into_val(&t.env)
                ],
                (t.user.clone(), amount).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_total_staked_tracks_locked_and_flexible_positions() {
    let t = setup(2, 1);

    assert_eq!(t.client.total_staked(), 0);
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.total_staked(), 1_000);

    t.client.lock_assets(&t.user, &500);
    assert_eq!(t.client.total_staked(), 1_500);

    t.client.unstake(&t.user);
    assert_eq!(t.client.total_staked(), 500);

    t.client.unlock_assets(&t.user, &500);
    assert_eq!(t.client.total_staked(), 0);
}

// ── total_distributed_credits tests ───────────────────────────────────────────

#[test]
fn test_total_distributed_credits_starts_at_zero() {
    let t = setup(2, 1);
    assert_eq!(t.client.total_distributed_credits(), 0);
}

#[test]
fn test_total_distributed_credits_counts_banked_stake_accrual_on_checkpoint() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.total_distributed_credits(), 0);

    advance_ledgers(&t.env, 10);
    // Read-only views must not commit anything to the aggregate.
    assert_eq!(t.client.get_credits(&t.user), 10_000);
    assert_eq!(t.client.total_distributed_credits(), 0);

    // unstake checkpoints and banks 10_000 credits.
    let banked = t.client.unstake(&t.user);
    assert_eq!(banked, 10_000);
    assert_eq!(t.client.total_distributed_credits(), 10_000);
}

#[test]
fn test_total_distributed_credits_counts_position_accrual_on_unlock() {
    let t = setup_with_lock_period(1, 1, 0);
    assert_eq!(t.client.total_distributed_credits(), 0);

    t.client.lock_assets(&t.user, &1_000);
    assert_eq!(t.client.total_distributed_credits(), 0);

    advance_ledgers(&t.env, 10);
    // Partial unlock checkpoints the position and banks 1_000 * 1 * 10.
    t.client.unlock_assets(&t.user, &500);
    assert_eq!(t.client.total_distributed_credits(), 10_000);
}

#[test]
fn test_total_distributed_credits_accumulates_across_users_and_systems() {
    let t = setup(2, 1);
    let other = Address::generate(&t.env);
    t.token_sac.mint(&other, &1_000_000_000i128);

    // User A flexible stake: 10 ledgers unbooted → 10_000 credits.
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.stake(&t.user, &100); // checkpoints 10_000
    assert_eq!(t.client.total_distributed_credits(), 10_000);

    // User B locked position: 5 more ledgers → 500 credits.
    t.client.lock_assets(&other, &100);
    advance_ledgers(&t.env, 5);
    t.client.unlock_assets(&other, &100); // checkpoints 500
    assert_eq!(t.client.total_distributed_credits(), 10_500);
}

#[test]
fn test_total_credits_earned_tracks_lifetime_credits_across_withdrawals() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);

    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 10_000);
    assert_eq!(t.client.total_credits_earned(&t.user), 0);

    t.client.unstake(&t.user);
    assert_eq!(t.client.total_credits_earned(&t.user), 10_000);

    advance_ledgers(&t.env, 5);
    t.client.stake(&t.user, &500);
    advance_ledgers(&t.env, 5);
    t.client.unstake(&t.user);
    assert_eq!(t.client.total_credits_earned(&t.user), 12_500);
}

#[test]
fn test_total_banked_credits_tracks_current_bank_across_users() {
    let t = setup(2, 1);
    let other = Address::generate(&t.env);
    t.token_sac.mint(&other, &1_000_000_000i128);

    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.total_banked_credits(), 0);

    t.client.unstake(&t.user);
    assert_eq!(t.client.total_banked_credits(), 0);

    t.client.stake(&other, &2_000);
    advance_ledgers(&t.env, 5);
    t.client.unstake(&other);
    assert_eq!(t.client.total_banked_credits(), 0);
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

    let before = t.env.as_contract(&t.contract_id, || {
        t.env
            .storage()
            .persistent()
            .get::<DataKey, UserStake>(&DataKey::UserStake(t.user.clone()))
    });
    let before = before.expect("stake storage must exist before upgrade");
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
    let cfg = t
        .client
        .get_boost_config(&t.user)
        .expect("boost config should default to zero allocation");
    assert_eq!(cfg.allocation_pct, 0);
    assert_eq!(cfg.multiplier, 2);
}

#[test]
fn test_total_credits_tracks_cumulative_accrual() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.total_credits(), 0);

    advance_ledgers(&t.env, 10);
    t.client.set_boost(&t.user, &50u32);
    assert_eq!(t.client.total_credits(), 10_000);

    advance_ledgers(&t.env, 10);
    t.client.set_boost(&t.user, &50u32);
    assert_eq!(t.client.total_credits(), 25_000);
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
fn test_set_boost_rejects_without_active_stake() {
    let t = setup(2, 1);
    let res = t.client.try_set_boost(&t.user, &50u32);
    assert_eq!(res, Err(Ok(PoolError::NoActiveStake)));
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
                (1i128, 4i128, 0u32).into_val(&t.env),
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
fn test_min_lock_period_seconds() {
    // 12 ledgers * ~5s/ledger = 60 seconds (#166).
    let t = setup_with_lock_period(2, 1, 12);
    assert_eq!(t.client.min_lock_period_seconds(), 60);
    assert_eq!(t.client.get_min_lock_period_seconds(), 60);

    // 30 ledgers -> 150 seconds; also confirms it tracks set_min_lock_period.
    t.client.set_min_lock_period(&25u32);
    assert_eq!(t.client.min_lock_period_seconds(), 125);
    assert_eq!(t.client.get_min_lock_period_seconds(), 125);
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
    // User checkpoints to adopt the new global multiplier
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);
    // Next 10 ledgers: effective_stake = 2000 (50% @ 3×) → 20000 -> total 35,000
    assert_eq!(t.client.get_credits(&t.user), 35_000);
}

#[test]
fn test_admin_multiplier_change_applies_to_existing_stake_without_manual_checkpoint() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);

    t.client.set_global_multiplier(&3u32);
    advance_ledgers(&t.env, 10);

    assert_eq!(t.client.get_credits(&t.user), 35_000);
}

#[test]
fn test_get_credits_matches_checkpoint_accrual_after_multiplier_change() {
    // Regression for #223: get_credits and checkpoint must use the same
    // multiplier source, so an un-checkpointed read equals exactly what the
    // next checkpointing operation banks.
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 10);

    t.client.set_global_multiplier(&3u32);
    advance_ledgers(&t.env, 10);

    // Read-only view: must not mutate stake state.
    let viewed = t.client.get_credits(&t.user);
    assert_eq!(viewed, 35_000);

    // unstake checkpoints → banked credits must equal the viewed total.
    let banked = t.client.unstake(&t.user);
    assert_eq!(banked, viewed);

    // The aggregate counter must agree with the banked amount too.
    assert_eq!(t.client.total_distributed_credits(), banked);
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
fn test_flash_stake_unstake_in_same_ledger_yields_no_credits() {
    // Regression for #169: stake has no lock period, so a user CAN immediately
    // unstake — but an immediate round-trip in the same ledger must earn no
    // credits, i.e. flash-staking provides no reward and no leverage.
    let t = setup(2, 1);
    let initial_balance = t.token.balance(&t.user);

    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &100u32);
    let credits = t.client.unstake(&t.user);

    assert_eq!(credits, 0, "flash staking must not mint credits");
    assert_eq!(
        t.token.balance(&t.user),
        initial_balance,
        "principal fully returned"
    );
    assert!(t.client.get_stake(&t.user).is_none());
    assert_eq!(t.client.get_credits(&t.user), 0);
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
    assert_eq!(t.client.get_position_credits(&t.user), 0);
    assert_eq!(t.client.get_stake_credits(&t.user), 0);
}

#[test]
fn test_get_position_credits_and_get_stake_credits_distinguish_systems() {
    let t = setup(2, 1);

    // Flexible stake in UserStake system (1000 tokens, 50% boost at 2x -> effective 1500)
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);

    // Time-locked position in Position system (500 tokens, credit rate 1)
    t.client.lock_assets(&t.user, &500);

    advance_ledgers(&t.env, 10);

    // Position credits = 500 * 1 * 10 = 5_000
    assert_eq!(t.client.get_position_credits(&t.user), 5_000);
    assert_eq!(t.client.calculate_credits(&t.user), 5_000);

    // Stake credits = 1500 * 1 * 10 = 15_000
    assert_eq!(t.client.get_stake_credits(&t.user), 15_000);

    // Combined get_credits = 5_000 + 15_000 = 20_000
    assert_eq!(t.client.get_credits(&t.user), 20_000);
}

// ── lock_assets tests ─────────────────────────────────────────────────────────

#[test]
fn test_admin_getter_returns_current_admin() {
    let t = setup(2, 1);
    assert_eq!(t.client.admin(), t.admin);
}

#[test]
fn test_propose_admin_does_not_change_admin_until_accepted() {
    let (env, contract_id, client, old_admin, _user) = setup_without_mocked_auth();
    let new_admin = Address::generate(&env);

    client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&new_admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&new_admin);
    assert_eq!(client.admin(), old_admin);

    client
        .mock_auths(&[MockAuth {
            address: &new_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .accept_admin();
    assert_eq!(client.admin(), new_admin);
}

#[test]
fn test_accept_admin_requires_proposed_address_auth() {
    let (env, contract_id, client, old_admin, user) = setup_without_mocked_auth();
    let new_admin = Address::generate(&env);
    client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "propose_admin",
                args: (&new_admin,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .propose_admin(&new_admin);

    let current_admin_result = client
        .mock_auths(&[MockAuth {
            address: &old_admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_accept_admin();
    assert!(
        current_admin_result.is_err(),
        "current admin may not accept"
    );

    let third_party_result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "accept_admin",
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_accept_admin();
    assert!(third_party_result.is_err(), "third parties may not accept");
    assert_eq!(client.admin(), old_admin);
}

#[test]
fn test_propose_admin_can_be_overwritten_or_cancelled_before_acceptance() {
    let t = setup(2, 1);
    let first_proposal = Address::generate(&t.env);
    let second_proposal = Address::generate(&t.env);

    t.client.propose_admin(&first_proposal);
    t.client.propose_admin(&second_proposal);
    t.client.accept_admin();
    assert_eq!(t.client.admin(), second_proposal);

    t.client.propose_admin(&second_proposal);
    let result = t.client.try_accept_admin();
    assert!(matches!(result, Err(Ok(PoolError::NoPendingAdmin))));
    assert_eq!(t.client.admin(), second_proposal);
}

#[test]
fn test_propose_admin_emits_event() {
    let t = setup(2, 1);
    let new_admin = Address::generate(&t.env);
    t.client.propose_admin(&new_admin);
    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("adm_prop").into_val(&t.env)
                ],
                (t.admin.clone(), new_admin).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_transfer_admin_changes_admin() {
    let t = setup(2, 1);
    let new_admin = Address::generate(&t.env);
    t.client.transfer_admin(&new_admin);
    assert_eq!(t.client.admin(), new_admin);
}

#[test]
fn test_set_global_multiplier_emits_old_and_new() {
    let t = setup(2, 1);

    // Pool was initialized with global_multiplier = 2.
    t.client.set_global_multiplier(&5);

    assert_eq!(
        t.env.events().all(),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("boost").into_val(&t.env),
                    soroban_sdk::symbol_short!("mult_set").into_val(&t.env)
                ],
                (2u32, 5u32).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_set_global_multiplier_event_reports_previous_value() {
    let t = setup(2, 1);

    t.client.set_global_multiplier(&5);
    t.client.set_global_multiplier(&3);

    // The most recent event pairs the just-superseded value (5) with the new
    // one (3), not the pool's original multiplier.
    let events = t.env.events().all();
    let (contract, topics, data) = events.last().unwrap();
    assert_eq!(contract, t.contract_id);
    assert_eq!(
        topics,
        soroban_sdk::vec![
            &t.env,
            soroban_sdk::symbol_short!("boost").into_val(&t.env),
            soroban_sdk::symbol_short!("mult_set").into_val(&t.env)
        ]
    );
    assert_eq!(data, (5u32, 3u32).into_val(&t.env));
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
fn test_has_position_returns_false_without_position() {
    let t = setup(1, 1);
    assert!(!t.client.has_position(&t.user));
}

#[test]
fn test_has_position_returns_true_after_lock() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &500);
    assert!(t.client.has_position(&t.user));
}

#[test]
fn test_has_position_returns_false_after_full_unlock() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &500);
    advance_ledgers(&t.env, 100);
    t.client.unlock_assets(&t.user, &500);
    assert!(!t.client.has_position(&t.user));
}

#[test]
fn test_lock_assets_additional_lock_checkpoints_credits() {
    // Lock 1000, advance 10 ledgers (10000 credits), then lock 500 more.
    // After checkpoint: banked = 10000, amount = 1500.
    // Earn 10 more ledgers with 0 boost: 1500 * 10 = 15000.
    // Total: 25000.
    let t = setup(1, 1); // multiplier=1 so no boost effect here
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.lock_assets(&t.user, &500); // banks 10000
    advance_ledgers(&t.env, 10);
    assert_eq!(t.client.get_credits(&t.user), 25_000);
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
                (t.user.clone(), 1_000i128, 1_000i128).into_val(&t.env),
            )
        ]
    );

    // Top up with another 500 tokens; event should include 500 as lock amount and 1500 as total position.
    // Soroban test env only retains the most recent invocation's events.
    t.client.lock_assets(&t.user, &500);
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
                (t.user.clone(), 500i128, 1_500i128).into_val(&t.env),
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
        if partition.len() == 1 {
            assert_eq!(
                final_unlock_events,
                soroban_sdk::vec![
                    &t.env,
                    (
                        t.contract_id.clone(),
                        soroban_sdk::vec![
                            &t.env,
                            soroban_sdk::symbol_short!("pool").into_val(&t.env),
                            soroban_sdk::symbol_short!("chkpt").into_val(&t.env)
                        ],
                        (
                            t.user.clone(),
                            EXPECTED_TOTAL_CREDITS,
                            EXPECTED_TOTAL_CREDITS
                        )
                            .into_val(&t.env),
                    ),
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
        } else {
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
fn test_lock_assets_topup_after_maturity_extends_unlock_ledger() {
    let t = setup_with_lock_period(1, 1, 100);
    t.client.lock_assets(&t.user, &100);
    advance_ledgers(&t.env, 100);

    t.client.lock_assets(&t.user, &500);
    let position = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(position.unlock_ledger, 200);
    assert!(t.client.try_unlock_assets(&t.user, &600).is_err());

    advance_ledgers(&t.env, 99);
    assert!(t.client.try_unlock_assets(&t.user, &600).is_err());
    advance_ledgers(&t.env, 1);
    t.client.unlock_assets(&t.user, &600);
    assert!(t.client.get_user_position(&t.user).is_none());
}

#[test]
fn test_lock_assets_topup_does_not_shorten_existing_unlock_ledger() {
    let t = setup_with_lock_period(1, 1, 1_000);
    t.client.lock_assets(&t.user, &1_000);
    let original_position = t.client.get_user_position(&t.user).unwrap();

    advance_ledgers(&t.env, 10);
    t.client.set_min_lock_period(&5u32);
    t.client.lock_assets(&t.user, &100);

    let topped_up_position = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(
        topped_up_position.unlock_ledger,
        original_position.unlock_ledger
    );
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
fn test_get_user_position_returns_accrued_credits() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 5);

    let pos = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(pos.total_credits, 5_000);
    assert_eq!(pos.amount, 1_000);
}

#[test]
fn test_get_stake_returns_accrued_credits() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    advance_ledgers(&t.env, 5);

    let stake = t.client.get_stake(&t.user).unwrap();
    assert_eq!(stake.credits_banked, 7_500);
    assert_eq!(stake.amount, 1_000);
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
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("paused").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_unpause_emits_event() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("unpaused").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_pause_staking_emits_event() {
    let t = setup(1, 1);
    t.client.pause_staking();
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("stg_pause").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_pause_withdrawals_emits_event() {
    let t = setup(1, 1);
    t.client.pause_withdrawals();
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("wd_pause").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_unpause_staking_emits_event() {
    let t = setup(1, 1);
    t.client.pause_staking();
    t.client.unpause_staking();
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("stg_unps").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_unpause_withdrawals_emits_event() {
    let t = setup(1, 1);
    t.client.pause_withdrawals();
    t.client.unpause_withdrawals();
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        soroban_sdk::vec![
            &t.env,
            (
                t.contract_id.clone(),
                soroban_sdk::vec![
                    &t.env,
                    soroban_sdk::symbol_short!("pool").into_val(&t.env),
                    soroban_sdk::symbol_short!("wd_unps").into_val(&t.env)
                ],
                ().into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_pause_staking_blocks_new_stakes_but_allows_withdrawals() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.pause_staking();

    assert!(t.client.try_stake(&t.user, &100i128).is_err());
    assert!(t.client.try_lock_assets(&t.user, &100i128).is_err());
    t.client.unstake(&t.user);
}

#[test]
fn test_pause_withdrawals_blocks_unstake_but_allows_new_stakes() {
    let t = setup(2, 1);
    t.client.pause_withdrawals();
    t.client.stake(&t.user, &1_000);

    assert!(t.client.try_unstake(&t.user).is_err());
    assert!(t.client.try_unlock_assets(&t.user, &100i128).is_err());
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
    // 5_000 (lock credits) + 3_000 (stake credits) preserved as a combined total.
    assert_eq!(t.client.get_banked_credits(&t.user), 8_000);
    // Individual histories must not be merged into a single figure (#145): the
    // lock/unlock position and boost stake credits remain separately retrievable.
    let split = t.client.get_banked_credits_split(&t.user);
    assert_eq!(split.position_credits, 5_000);
    assert_eq!(split.stake_credits, 3_000);
}

#[test]
fn test_emergency_withdraw_while_unpaused_returns_not_paused() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    let result = t.client.try_emergency_withdraw(&t.user);
    assert!(matches!(result, Err(Ok(PoolError::NotPaused))));
}

// ── #130: multiplier-lock invariant tests ───────────────────────────────────

/// Asserts that `set_global_multiplier` has zero effect on the lock/position accrual path.
///
/// Design Invariant:
/// Boost allocations and the global multiplier apply ONLY to staked positions (`UserStake`),
/// never to locked positions (`Position`). Mid-lock admin changes to `global_multiplier` must
/// have zero measurable impact on accrued or accruing locked credits.
#[test]
fn test_global_multiplier_change_does_not_affect_locked_position_credits() {
    let t = setup(1, 1);

    // Lock 1,000 tokens and stake 1,000 tokens for the same user.
    t.client.lock_assets(&t.user, &1_000);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &100u32); // 100% boost allocation on stake side

    // Advance 10 ledgers under multiplier = 1.
    advance_ledgers(&t.env, 10);

    // Admin updates global multiplier from 1 to 5 mid-lock.
    t.client.set_global_multiplier(&5u32);

    // Advance another 10 ledgers under multiplier = 5. Total elapsed = 20 ledgers.
    advance_ledgers(&t.env, 10);

    // Lock side: credits = amount * credit_rate * total_elapsed = 1000 * 1 * 20 = 20,000.
    // The multiplier change to 5 must have ZERO effect on locked position credit accrual.
    assert_eq!(t.client.calculate_credits(&t.user), 20_000);

    // Stake side: uncheckpointed window correctly tracks pre/post multiplier change.
    // Pre-change (ledgers 0-10, mult=1, 100% boost -> effective 1000): 1000 * 1 * 10 = 10,000.
    // Post-change (ledgers 10-20, mult=5, 100% boost -> effective 5000): 5000 * 1 * 10 = 50,000.
    // Total stake credits = 60,000.
    // Total credits = 20,000 (lock) + 60,000 (stake) = 80,000.
    assert_eq!(t.client.get_credits(&t.user), 80_000);

    // Unlock assets and verify position credits bank cleanly.
    t.client.unlock_assets(&t.user, &1_000);
    assert_eq!(t.client.calculate_credits(&t.user), 0);
}

/// Asserts that `set_boost` (user allocation percentage) has zero effect on locked position credits.
#[test]
fn test_set_boost_has_zero_effect_on_locked_position_credits() {
    let t = setup(2, 1); // multiplier = 2

    // User locks 1,000 tokens without any stake.
    t.client.lock_assets(&t.user, &1_000);

    // User sets boost allocation to 100%.
    t.client.set_boost(&t.user, &100u32);

    advance_ledgers(&t.env, 10);

    // Lock credits must equal 1000 * 1 * 10 = 10,000 regardless of boost setting or 2x multiplier.
    assert_eq!(t.client.calculate_credits(&t.user), 10_000);
}

#[test]
fn test_emergency_withdraw_requires_user_auth() {
    let (env, contract_id, client, _admin, user) = setup_without_mocked_auth();

    // Attacker cannot withdraw user's funds without user's auth
    let attacker = Address::generate(&env);
    let unauth_result = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "emergency_withdraw",
                args: (&user,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_emergency_withdraw(&user);
    assert!(
        unauth_result.is_err(),
        "third party cannot trigger emergency withdraw without user auth"
    );
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
fn test_whitelist_count_reflects_adds_and_removes() {
    let t = setup(2, 1);
    assert_eq!(t.client.whitelist_count(), 0);
    assert_eq!(t.client.get_whitelist_count(), 0);

    let user1 = Address::generate(&t.env);
    let user2 = Address::generate(&t.env);

    t.client.add_to_whitelist(&user1);
    assert_eq!(t.client.whitelist_count(), 1);

    t.client.add_to_whitelist(&user2);
    assert_eq!(t.client.whitelist_count(), 2);

    // Re-adding an existing entry must not double-count.
    t.client.add_to_whitelist(&user1);
    assert_eq!(t.client.whitelist_count(), 2);

    t.client.remove_from_whitelist(&user1);
    assert_eq!(t.client.whitelist_count(), 1);

    // Removing a non-member is a no-op for the count.
    t.client.remove_from_whitelist(&Address::generate(&t.env));
    assert_eq!(t.client.whitelist_count(), 1);

    t.client.remove_from_whitelist(&user2);
    assert_eq!(t.client.whitelist_count(), 0);
}

#[test]
fn test_whitelist_count_matches_get_whitelisted_users_total() {
    let t = setup(2, 1);

    let mut users = soroban_sdk::Vec::new(&t.env);
    for _ in 0..5 {
        users.push_back(Address::generate(&t.env));
    }
    t.client.batch_add_to_whitelist(&users);

    let listed = t.client.get_whitelisted_users(&0u32, &100u32);
    assert_eq!(t.client.whitelist_count(), listed.total);
    assert_eq!(t.client.whitelist_count(), 5);
}

#[test]
fn test_whitelist_count_uninitialized_returns_not_initialized() {
    let (_env, client, _admin) = setup_uninitialized();
    assert!(matches!(
        client.try_whitelist_count(),
        Err(Ok(PoolError::NotInitialized))
    ));
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
fn test_batch_remove_from_whitelist() {
    let t = setup(2, 1);
    t.client.enable_whitelist();

    let user1 = Address::generate(&t.env);
    let user2 = Address::generate(&t.env);
    let user3 = Address::generate(&t.env);

    let mut users = soroban_sdk::Vec::new(&t.env);
    users.push_back(user1.clone());
    users.push_back(user2.clone());
    users.push_back(user3.clone());

    t.client.batch_add_to_whitelist(&users);
    assert!(t.client.is_whitelisted(&user1));
    assert!(t.client.is_whitelisted(&user2));
    assert!(t.client.is_whitelisted(&user3));

    // Batch remove exactly mirrors the batch add (#167): all three gone at once.
    t.client.batch_remove_from_whitelist(&users);
    assert!(!t.client.is_whitelisted(&user1));
    assert!(!t.client.is_whitelisted(&user2));
    assert!(!t.client.is_whitelisted(&user3));

    // Removed users can no longer stake.
    let result_stake = t.client.try_stake(&user1, &100);
    assert!(matches!(result_stake, Err(Ok(PoolError::NotWhitelisted))));
}

#[test]
#[should_panic(expected = "max 50 addresses per call")]
fn test_batch_remove_from_whitelist_exceeds_limit() {
    let t = setup(2, 1);
    let mut users = soroban_sdk::Vec::new(&t.env);
    for _ in 0..51 {
        users.push_back(Address::generate(&t.env));
    }
    t.client.batch_remove_from_whitelist(&users);
}

#[test]
fn test_batch_remove_from_whitelist_requires_admin_auth() {
    let (env, contract_id, client, _admin, user) = setup_without_mocked_auth();

    let users = soroban_sdk::vec![&env, user.clone()];
    let result = client
        .mock_auths(&[MockAuth {
            address: &user,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "batch_remove_from_whitelist",
                args: (users.clone(),).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_batch_remove_from_whitelist(&users);

    assert!(
        result.is_err(),
        "non-admin batch_remove_from_whitelist must be rejected"
    );
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
                multiplier: read_global_multiplier(env),
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
    // no unstake applied.
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

#[test]
fn test_staked_user_count_increments_and_decrements_correctly() {
    let t = setup(1, 10);
    assert_eq!(t.client.staked_user_count(), 0);
    assert_eq!(t.client.get_staked_user_count(), 0);

    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &10_000);

    // User 1 stakes: count becomes 1
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.staked_user_count(), 1);

    // User 1 stakes more: count stays 1
    t.client.stake(&t.user, &500);
    assert_eq!(t.client.staked_user_count(), 1);

    // User 2 locks position: count becomes 2
    t.client.lock_assets(&user2, &2_000);
    assert_eq!(t.client.staked_user_count(), 2);

    // User 1 unstakes completely: count becomes 1
    t.client.unstake(&t.user);
    assert_eq!(t.client.staked_user_count(), 1);

    // User 2 unlocks position completely: count becomes 0
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&user2, &2_000);
    assert_eq!(t.client.staked_user_count(), 0);
}

#[test]
fn test_lock_assets_top_up_extends_unlock_ledger() {
    let t = setup_with_lock_period(1, 10, 10);
    let start_ledger = t.env.ledger().sequence();

    // Initial lock of 1,000 for 10 ledgers
    t.client.lock_assets(&t.user, &1_000);
    let pos1 = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(pos1.unlock_ledger, start_ledger + 10);

    // Advance ledgers by 5
    advance_ledgers(&t.env, 5);

    // Top-up lock of 500: fresh lock period extends unlock_ledger to start_ledger + 5 + 10 = start_ledger + 15
    t.client.lock_assets(&t.user, &500);
    let pos2 = t.client.get_user_position(&t.user).unwrap();
    assert_eq!(pos2.amount, 1_500);
    assert_eq!(pos2.unlock_ledger, start_ledger + 15);

    // Trying to unlock at ledger start_ledger + 12 should fail
    advance_ledgers(&t.env, 7); // now sequence is start_ledger + 12
    assert!(
        t.client.try_unlock_assets(&t.user, &1_500).is_err(),
        "unlock before extended lock period must fail"
    );

    // Advancing past start_ledger + 15 allows full unlock
    advance_ledgers(&t.env, 3); // now sequence is start_ledger + 15
    t.client.unlock_assets(&t.user, &1_500);
}

#[test]
fn test_lock_count_increments_on_every_lock_operation() {
    let t = setup(1, 10);
    assert_eq!(t.client.lock_count(), 0);
    assert_eq!(t.client.get_lock_count(), 0);

    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &10_000);

    // Flexible staking does not affect lock_count
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.lock_count(), 0);
    t.client.unstake(&t.user);
    assert_eq!(t.client.lock_count(), 0);

    // User 1 locks: lock_count becomes 1
    t.client.lock_assets(&t.user, &1_000);
    assert_eq!(t.client.lock_count(), 1);
    assert_eq!(t.client.get_lock_count(), 1);

    // User 2 locks: lock_count becomes 2
    t.client.lock_assets(&user2, &2_000);
    assert_eq!(t.client.lock_count(), 2);

    // User 1 top-up (additional lock operation): lock_count becomes 3
    t.client.lock_assets(&t.user, &500);
    assert_eq!(t.client.lock_count(), 3);

    // Unlocking assets does not decrement lock_count
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&user2, &2_000);
    assert_eq!(t.client.lock_count(), 3);
}

#[test]
fn test_unstake_count_increments_on_every_unstake_operation() {
    let t = setup(1, 10);
    assert_eq!(t.client.unstake_count(), 0);
    assert_eq!(t.client.get_unstake_count(), 0);

    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &10_000);

    // Staking does not affect unstake_count
    t.client.stake(&t.user, &1_000);
    t.client.stake(&user2, &2_000);
    assert_eq!(t.client.unstake_count(), 0);

    // Lock/unlock operations do not affect unstake_count
    let user3 = Address::generate(&t.env);
    t.token_sac.mint(&user3, &10_000);
    t.client.lock_assets(&user3, &1_000);
    assert_eq!(t.client.unstake_count(), 0);

    // User 1 unstakes: unstake_count becomes 1
    t.client.unstake(&t.user);
    assert_eq!(t.client.unstake_count(), 1);
    assert_eq!(t.client.get_unstake_count(), 1);

    // User 2 unstakes: unstake_count becomes 2
    t.client.unstake(&user2);
    assert_eq!(t.client.unstake_count(), 2);
    assert_eq!(t.client.get_unstake_count(), 2);

    // Unlocking assets does not increment unstake_count
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&user3, &1_000);
    assert_eq!(t.client.unstake_count(), 2);
}

#[test]
fn test_checkpoint_emits_chkpt_event() {
    let t = setup(1, 10);
    t.client.stake(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.stake(&t.user, &500);

    let events = t.env.events().all().filter_by_contract(&t.contract_id);
    assert_ne!(events, soroban_sdk::vec![&t.env]);
}

#[test]
fn test_active_stake_count_lifecycle() {
    let t = setup(1, 10);
    assert_eq!(t.client.active_stake_count(), 0);
    assert_eq!(t.client.get_active_stake_count(), 0);

    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &10_000);

    // User 1 stakes: active_stake_count becomes 1
    t.client.stake(&t.user, &1_000);
    assert_eq!(t.client.active_stake_count(), 1);
    assert_eq!(t.client.get_active_stake_count(), 1);

    // User 1 stakes more (top up): active_stake_count remains 1
    t.client.stake(&t.user, &500);
    assert_eq!(t.client.active_stake_count(), 1);

    // User 2 stakes: active_stake_count becomes 2
    t.client.stake(&user2, &2_000);
    assert_eq!(t.client.active_stake_count(), 2);

    // User 3 locks position: locked position does not increment active_stake_count
    let user3 = Address::generate(&t.env);
    t.token_sac.mint(&user3, &10_000);
    t.client.lock_assets(&user3, &1_000);
    assert_eq!(t.client.active_stake_count(), 2);

    // User 1 unstakes: active_stake_count becomes 1
    t.client.unstake(&t.user);
    assert_eq!(t.client.active_stake_count(), 1);

    // Pool pauses, User 2 emergency withdraws: active_stake_count becomes 0
    t.client.pause();
    t.client.emergency_withdraw(&user2);
    assert_eq!(t.client.active_stake_count(), 0);
    assert_eq!(t.client.get_active_stake_count(), 0);
}

#[test]
fn test_credit_rate_change_count_tracking() {
    let t = setup(1, 10);
    assert_eq!(t.client.credit_rate_change_count(), 0);
    assert_eq!(t.client.get_credit_rate_change_count(), 0);

    // First rate change
    t.client.set_credit_rate(&5i128);
    assert_eq!(t.client.credit_rate_change_count(), 1);
    assert_eq!(t.client.get_credit_rate_change_count(), 1);

    // Second rate change
    t.client.set_credit_rate(&10i128);
    assert_eq!(t.client.credit_rate_change_count(), 2);
    assert_eq!(t.client.get_credit_rate_change_count(), 2);

    // Invalid rate change does not increment count
    assert!(t.client.try_set_credit_rate(&0i128).is_err());
    assert_eq!(t.client.credit_rate_change_count(), 2);
    assert_eq!(t.client.get_credit_rate_change_count(), 2);
}

#[test]
fn test_migrate_schema_version_framework() {
    let t = setup(1, 10);
    let prev = t.client.migrate();
    assert_eq!(prev, 1);
}
