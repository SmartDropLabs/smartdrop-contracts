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

// ── migrate_multiplier (#688) ─────────────────────────────────────────────────

use soroban_sdk::symbol_short;

/// 100_000 is the largest amount that stays in the base 100% tier
/// (`get_tier_multiplier` is strictly greater-than), so the credits below are
/// exactly `compute_credits` with no stake-size scaling.
const STAKE_AMOUNT: i128 = 100_000;

/// A user with a flexible stake and a 100% boost allocation, so the global
/// multiplier fully drives their accrual.
fn boosted_staker(t: &TestEnv) -> Address {
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &STAKE_AMOUNT);
    t.client.stake(&user, &STAKE_AMOUNT);
    t.client.set_boost(&user, &100u32);
    user
}

/// Credits for `ledgers` at `multiplier`, at the pool's test credit rate (1_000).
fn credits_at(multiplier: u32, ledgers: u32) -> i128 {
    compute_credits(STAKE_AMOUNT, 100, multiplier, 1_000, ledgers)
}

#[test]
fn test_migrate_multiplier_rebases_a_stale_stake_without_changing_total_credits() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);
    advance_ledgers(&t.env, 10);

    // Stale: the stake still holds the multiplier from before the change.
    assert_eq!(t.client.get_boost_config(&user).unwrap().multiplier, 1);
    let before = t.client.get_credits(&user);

    t.client.migrate_multiplier(&user);

    assert_eq!(t.client.get_boost_config(&user).unwrap().multiplier, 3);
    assert_eq!(t.client.get_credits(&user), before);
}

#[test]
fn test_migrate_multiplier_settles_old_credits_and_accrues_at_the_new_multiplier_after() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);
    t.client.migrate_multiplier(&user);

    // Ten ledgers were earned at multiplier 1 and banked by the migration.
    assert_eq!(t.client.get_stake(&user).unwrap().credits_banked, credits_at(1, 10));

    let settled = t.client.get_credits(&user);
    advance_ledgers(&t.env, 10);

    assert_eq!(t.client.get_credits(&user), settled + credits_at(3, 10));
}

#[test]
fn test_migrating_between_two_changes_credits_each_interval_at_its_own_multiplier() {
    let t = setup();
    let migrated = boosted_staker(&t);
    let unmigrated = boosted_staker(&t);

    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&2u32);
    t.client.migrate_multiplier(&migrated);

    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);
    t.client.migrate_multiplier(&migrated);

    advance_ledgers(&t.env, 10);

    // Migrated at each change: every ten-ledger interval is credited at the
    // multiplier that was live during it.
    assert_eq!(
        t.client.get_credits(&migrated),
        credits_at(1, 10) + credits_at(2, 10) + credits_at(3, 10)
    );
    // Never migrated: accrual only knows about the latest change, so the
    // interval at multiplier 2 is credited at the stale multiplier 1.
    assert_eq!(
        t.client.get_credits(&unmigrated),
        credits_at(1, 20) + credits_at(3, 10)
    );
    assert_ne!(
        t.client.get_credits(&migrated),
        t.client.get_credits(&unmigrated)
    );
}

#[test]
fn test_migrate_multiplier_is_a_no_op_for_an_already_current_stake() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 5);
    let before = t.client.get_stake(&user).unwrap();

    t.client.migrate_multiplier(&user);

    let after = t.client.get_stake(&user).unwrap();
    assert_eq!(after.start_ledger, before.start_ledger);
    assert_eq!(after.credits_banked, before.credits_banked);
    assert_eq!(t.client.get_credits(&user), credits_at(1, 5));
}

#[test]
fn test_migrate_multiplier_twice_changes_nothing_the_second_time() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);
    advance_ledgers(&t.env, 4);

    t.client.migrate_multiplier(&user);
    let once = t.client.get_stake(&user).unwrap();
    let credits_once = t.client.get_credits(&user);

    t.client.migrate_multiplier(&user);
    let twice = t.client.get_stake(&user).unwrap();

    assert_eq!(twice.start_ledger, once.start_ledger);
    assert_eq!(twice.credits_banked, once.credits_banked);
    assert_eq!(twice.multiplier, once.multiplier);
    assert_eq!(t.client.get_credits(&user), credits_once);
}

#[test]
fn test_migrate_multiplier_also_picks_up_a_changed_credit_rate() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_credit_rate(&2_000i128);
    advance_ledgers(&t.env, 10);

    // The multiplier is unchanged but the stake's credit-rate snapshot is stale,
    // so it is not "already current".
    assert_eq!(t.client.get_stake(&user).unwrap().credit_rate, 1_000);

    t.client.migrate_multiplier(&user);

    assert_eq!(t.client.get_stake(&user).unwrap().credit_rate, 2_000);
}

#[test]
fn test_migrate_multiplier_requires_an_active_stake() {
    let t = setup();
    let stranger = Address::generate(&t.env);

    assert_eq!(
        t.client.try_migrate_multiplier(&stranger),
        Err(Ok(PoolError::NoActiveStake))
    );
}

#[test]
fn test_migrate_multiplier_rejects_a_user_with_only_a_locked_position() {
    // A time-locked `Position` has no multiplier snapshot to migrate.
    let t = setup();
    let user = Address::generate(&t.env);
    StellarAssetClient::new(&t.env, &t.token_address).mint(&user, &1_000i128);
    t.client.lock_assets(&user, &1_000i128);

    assert_eq!(
        t.client.try_migrate_multiplier(&user),
        Err(Ok(PoolError::NoActiveStake))
    );
}

#[test]
fn test_migrate_multiplier_on_an_uninitialised_pool_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(FarmingPool, ());
    let client = FarmingPoolClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    assert_eq!(
        client.try_migrate_multiplier(&user),
        Err(Ok(PoolError::NotInitialized))
    );
}

#[test]
fn test_migrate_multiplier_is_admin_only_and_unauthorised_calls_change_nothing() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);

    // No authorisation at all, so the admin's `require_auth` cannot be satisfied.
    t.env.mock_auths(&[]);

    assert!(t.client.try_migrate_multiplier(&user).is_err());
    assert_eq!(t.client.get_boost_config(&user).unwrap().multiplier, 1);
}

#[test]
fn test_migrate_multiplier_is_recorded_in_the_admin_action_log() {
    let t = setup();
    let user = boosted_staker(&t);
    advance_ledgers(&t.env, 10);
    t.client.set_global_multiplier(&3u32);
    let before = t.client.get_admin_action_history(&0u32, &100u32).total;

    t.client.migrate_multiplier(&user);

    let page = t.client.get_admin_action_history(&0u32, &100u32);
    assert_eq!(page.total, before + 1);
    let last = page.events.get(page.total - 1).unwrap();
    assert_eq!(last.action, symbol_short!("mult_mig"));
    assert_eq!(last.admin, t.client.admin());
    assert_eq!(last.ledger, t.env.ledger().sequence());
}

#[test]
fn test_a_no_op_migration_is_not_logged() {
    let t = setup();
    let user = boosted_staker(&t);
    let before = t.client.get_admin_action_history(&0u32, &100u32).total;

    t.client.migrate_multiplier(&user);

    assert_eq!(
        t.client.get_admin_action_history(&0u32, &100u32).total,
        before
    );
}
