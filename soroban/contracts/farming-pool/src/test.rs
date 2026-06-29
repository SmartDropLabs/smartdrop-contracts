#![cfg(test)]
use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, Env, IntoVal,
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
    client.initialize(
        &admin,
        &asset.address(),
        &global_multiplier,
        &credit_rate,
        &min_lock_period,
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
    client.initialize(&admin, &asset.address(), &2u32, &1i128, &0u32);

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
    assert!(
        !t.env.events().all().events().is_empty(),
        "lock event not emitted"
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
    assert!(t.client.get_user_position(&t.user).is_none());
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 0);
}

#[test]
fn test_unlock_assets_partial_keeps_remaining_position() {
    let t = setup(1, 1);
    let initial_balance = t.token.balance(&t.user);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 10);
    t.client.unlock_assets(&t.user, &400); // partial unlock
    let pos = t
        .client
        .get_user_position(&t.user)
        .expect("position should still exist");
    assert_eq!(pos.amount, 600);
    // 1000 * 10 = 10000 credits banked during checkpoint
    assert_eq!(pos.total_credits, 10_000);
    assert_eq!(t.token.balance(&t.user), initial_balance - 600);
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
    assert!(
        !t.env.events().all().events().is_empty(),
        "unlock event not emitted"
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
fn test_calculate_credits_zero_without_position() {
    let t = setup(1, 1);
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 0);
}

#[test]
fn test_calculate_credits_accrues_over_time() {
    // credit_rate = 2, amount = 500, ledgers = 20 → credits = 500 * 2 * 20 = 20000
    let t = setup(1, 2);
    t.client.lock_assets(&t.user, &500);
    advance_ledgers(&t.env, 20);
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 20_000);
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
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 25_000);
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
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 13_000);
}

// ── get_user_position tests ───────────────────────────────────────────────────

#[test]
fn test_get_user_position_none_before_lock() {
    let t = setup(1, 1);
    assert!(t.client.get_user_position(&t.user).unwrap().is_none());
}

#[test]
fn test_get_user_position_returns_correct_fields() {
    let t = setup(1, 1);
    let start = t.env.ledger().sequence();
    t.client.lock_assets(&t.user, &750);
    let pos = t.client.get_user_position(&t.user).unwrap().unwrap();
    assert_eq!(pos.amount, 750);
    assert_eq!(pos.lock_ledger, start);
    assert_eq!(pos.checkpoint_ledger, start);
    assert_eq!(pos.total_credits, 0);
}

#[test]
fn test_get_user_position_none_after_full_unlock() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    advance_ledgers(&t.env, 5);
    t.client.unlock_assets(&t.user, &1_000);
    assert!(t.client.get_user_position(&t.user).unwrap().is_none());
}

// ── pause / unpause tests ─────────────────────────────────────────────────────

#[test]
fn test_pool_not_paused_initially() {
    let t = setup(1, 1);
    assert!(!t.client.is_paused().unwrap());
}

#[test]
fn test_pause_blocks_lock_assets() {
    let t = setup(1, 1);
    t.client.pause();
    assert!(t.client.is_paused().unwrap());
    assert!(t.client.try_lock_assets(&t.user, &100i128).is_err());
}

#[test]
fn test_pause_blocks_unlock_assets() {
    let t = setup(1, 1);
    t.client.lock_assets(&t.user, &1_000);
    t.client.pause();
    assert!(t.client.try_unlock_assets(&t.user, &1_000).is_err());
}

#[test]
fn test_unpause_restores_operations() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    assert!(!t.client.is_paused().unwrap());
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
    assert!(t.client.try_stake(&t.user, &100i128).is_err());
}

#[test]
fn test_unpause_restores_stake() {
    let t = setup(1, 1);
    t.client.pause();
    t.client.unpause();
    t.client.stake(&t.user, &500);
    assert_eq!(t.client.get_stake(&t.user).unwrap().unwrap().amount, 500);
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
    assert!(t.client.get_stake(&t.user).unwrap().is_none());
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
        t.client.get_boost_config(&t.user).unwrap().unwrap().allocation_pct,
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
    assert_eq!(t.client.get_boost_config(&t.user).unwrap().unwrap().multiplier, 3);
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
    assert_eq!(t.client.calculate_credits(&t.user).unwrap(), 10_000); // 1000 * 10
    assert_eq!(t.client.calculate_credits(&user2).unwrap(), 20_000); // 2000 * 10
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
        .unwrap()
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
        t.client.get_user_position(&t.user).unwrap().is_none(),
        "position should be cleared"
    );
    assert!(
        t.client.get_stake(&t.user).unwrap().is_none(),
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
    assert_eq!(t.client.get_stake(&t.user).unwrap().unwrap().amount, 1_000);

    // User can lock when whitelist is not enabled
    let user2 = Address::generate(&t.env);
    t.token_sac.mint(&user2, &500_000i128);
    t.client.lock_assets(&user2, &500);
    assert_eq!(t.client.get_user_position(&user2).unwrap().unwrap().amount, 500);
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
    assert_eq!(t.client.get_stake(&t.user).unwrap().unwrap().amount, 1_000);
    assert_eq!(t.client.get_user_position(&t.user).unwrap().unwrap().amount, 500);

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
    assert_eq!(t.client.get_stake(&t.user).unwrap().unwrap().amount, 1_000);
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

    assert_eq!(t.client.get_stake(&user1).unwrap().unwrap().amount, 100);
    assert_eq!(t.client.get_stake(&user2).unwrap().unwrap().amount, 100);
    assert_eq!(t.client.get_stake(&user3).unwrap().unwrap().amount, 100);
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
