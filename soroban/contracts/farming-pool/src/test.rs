#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

struct TestEnv {
    env: Env,
    client: FarmingPoolClient<'static>,
    token_address: Address,
}

fn setup() -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_address = asset.address();
    StellarAssetClient::new(&env, &token_address).mint(&admin, &10_000_000_000i128);

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    client.initialize(&admin, &asset.address(), &1u32, &1_000i128, &1u32, &1i128);
    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };

    TestEnv {
        env,
        client,
        token_address,
    }
}

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = current + by;
    });
}

#[test]
fn test_tier_multiplier_thresholds() {
    let t = setup();

    assert_eq!(t.client.get_tier_multiplier(&0), 100);
    assert_eq!(t.client.get_tier_multiplier(&100_000), 100);
    assert_eq!(t.client.get_tier_multiplier(&100_001), 150);
    assert_eq!(t.client.get_tier_multiplier(&1_000_000), 150);
    assert_eq!(t.client.get_tier_multiplier(&1_000_001), 200);
    assert_eq!(
        t.client.try_get_tier_multiplier(&-1),
        Err(Ok(PoolError::InvalidAmount))
    );
}

#[test]
fn test_tier_multiplier_scales_accrued_credits() {
    let t = setup();
    let small = Address::generate(&t.env);
    let large = Address::generate(&t.env);
    let token = StellarAssetClient::new(&t.env, &t.token_address);
    token.mint(&small, &100_000);
    token.mint(&large, &2_000_000);

    t.client.stake(&small, &100_000);
    t.client.stake(&large, &2_000_000);
    advance_ledgers(&t.env, 10);

    // small: 100_000 * 1_000 * 10 * 100/100 = 1_000_000_000
    // large: 2_000_000 * 1_000 * 10 * 200/100 = 40_000_000_000
    assert_eq!(t.client.get_credits(&small), 1_000_000_000);
    assert_eq!(t.client.get_credits(&large), 40_000_000_000);
}

// ── #697 — export_multiplier ─────────────────────────────────────────────────

#[test]
fn test_export_multiplier_for_user_without_stake_returns_defaults() {
    let t = setup();
    let user = Address::generate(&t.env);

    let data = t.client.export_multiplier(&user);

    assert_eq!(data.user, user);
    assert_eq!(data.schema_version, 1);
    assert_eq!(data.exported_at_ledger, t.env.ledger().sequence());
    assert_eq!(data.global_multiplier, 1);
    assert_eq!(data.stake_multiplier, 1);
    assert_eq!(data.allocation_pct, 0);
    assert_eq!(data.tier_multiplier_pct, 100);
    assert_eq!(data.stake_amount, 0);
    assert_eq!(data.effective_stake, 0);
    assert_eq!(data.credit_rate, 1_000);
    assert_eq!(data.banked_credits, 0);
    assert!(!data.has_active_stake);
}

#[test]
fn test_export_multiplier_reflects_stake_boost_and_tier() {
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &2_000_000);

    t.client.set_global_multiplier(&5);
    t.client.stake(&user, &2_000_000);
    t.client.set_boost(&user, &50);

    let data = t.client.export_multiplier(&user);

    assert!(data.has_active_stake);
    assert_eq!(data.stake_amount, 2_000_000);
    assert_eq!(data.allocation_pct, 50);
    assert_eq!(data.global_multiplier, 5);
    assert_eq!(data.stake_multiplier, 5);
    assert_eq!(data.tier_multiplier_pct, 200);
    // 2_000_000 * (100 - 50 + 50 * 5) / 100
    assert_eq!(data.effective_stake, 6_000_000);
}

#[test]
fn test_export_multiplier_keeps_stake_snapshot_after_global_change() {
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &10_000);

    t.client.set_global_multiplier(&5);
    t.client.stake(&user, &10_000);
    t.client.set_global_multiplier(&7);

    let data = t.client.export_multiplier(&user);

    assert_eq!(data.global_multiplier, 7);
    assert_eq!(data.stake_multiplier, 5);
}

#[test]
fn test_export_multiplier_matches_multiplier_breakdown() {
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &500_000);

    t.client.stake(&user, &500_000);
    t.client.set_boost(&user, &40);
    advance_ledgers(&t.env, 12);

    let data = t.client.export_multiplier(&user);
    let breakdown = t.client.get_multiplier_breakdown(&user);

    assert_eq!(data.stake_amount, breakdown.stake_amount);
    assert_eq!(data.allocation_pct, breakdown.allocation_pct);
    assert_eq!(data.global_multiplier, breakdown.global_multiplier);
    assert_eq!(data.effective_stake, breakdown.effective_stake);
    assert_eq!(data.credit_rate, breakdown.credit_rate);
    assert_eq!(data.banked_credits, breakdown.banked_credits);
}

#[test]
fn test_export_multiplier_covers_locked_position() {
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &5_000);

    t.client.lock_assets(&user, &5_000);

    let data = t.client.export_multiplier(&user);

    assert!(data.has_active_stake);
    assert_eq!(data.stake_amount, 5_000);
    assert_eq!(data.stake_multiplier, data.global_multiplier);
    assert_eq!(data.tier_multiplier_pct, 100);
}

#[test]
fn test_export_multiplier_is_read_only_and_repeatable() {
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &20_000);
    t.client.stake(&user, &20_000);

    let first = t.client.export_multiplier(&user);
    let second = t.client.export_multiplier(&user);

    assert_eq!(first, second);
    assert_eq!(t.client.get_credits(&user), 0);
}

#[test]
fn test_export_multiplier_rejects_uninitialized_pool() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    assert_eq!(
        client.try_export_multiplier(&user),
        Err(Ok(PoolError::NotInitialized))
    );
}
