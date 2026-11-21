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

#[test]
fn test_take_multiplier_snapshot_captures_state_and_persists_history() {
    let t = setup();
    let user = Address::generate(&t.env);
    let token = StellarAssetClient::new(&t.env, &t.token_address);
    token.mint(&user, &1_000);
    t.client.stake(&user, &1_000);

    let snap1 = t.client.take_multiplier_snapshot(&user);
    assert_eq!(snap1.stake_amount, 1_000);
    assert_eq!(snap1.global_multiplier, 1);

    advance_ledgers(&t.env, 5);
    let snap2 = t.client.take_multiplier_snapshot(&user);
    assert_eq!(snap2.ledger, snap1.ledger + 5);

    let page = t.client.get_multiplier_snapshots(&user, &0, &10);
    assert_eq!(page.total, 2);
    assert_eq!(page.snapshots.len(), 2);
    assert_eq!(page.snapshots.get(0).unwrap().ledger, snap1.ledger);
    assert_eq!(page.snapshots.get(1).unwrap().ledger, snap2.ledger);
}

#[test]
fn test_take_multiplier_snapshot_for_user_with_no_stake_is_zero() {
    let t = setup();
    let user = Address::generate(&t.env);

    let snap = t.client.take_multiplier_snapshot(&user);
    assert_eq!(snap.stake_amount, 0);
    assert_eq!(snap.effective_stake, 0);
}
