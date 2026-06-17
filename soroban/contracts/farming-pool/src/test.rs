#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: FarmingPoolClient<'static>,
    token: TokenClient<'static>,
    admin: Address,
    user: Address,
}

fn setup(global_multiplier: u32, credit_rate: i128) -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user  = Address::generate(&env);

    // Deploy a Stellar Asset Contract for the stake token.
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&user, &1_000_000_000i128);

    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    client.initialize(&admin, &asset.address(), &global_multiplier, &credit_rate);

    let token = TokenClient::new(&env, &asset.address());

    // Transmute lifetime to 'static so the struct can own client & token.
    // SAFETY: env owns the contract and token registrations; they live as long as env.
    let client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(client)
    };
    let token = unsafe {
        core::mem::transmute::<TokenClient<'_>, TokenClient<'static>>(token)
    };

    TestEnv { env, client, token, admin, user }
}

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|l| l.sequence_number = current + by);
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

// ── Contract integration tests ────────────────────────────────────────────────

#[test]
fn test_set_boost_and_get_config() {
    let t = setup(2, 1);
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);

    let cfg = t.client.get_boost_config(&t.user).expect("boost config should be set");
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
    // Next 5:  credits = 1500 * 5 = 7500 (50% boost, 2×)
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
#[should_panic(expected = "multiplier must be >= 1")]
fn test_admin_multiplier_rejects_zero() {
    let t = setup(2, 1);
    t.client.set_global_multiplier(&0u32);
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
