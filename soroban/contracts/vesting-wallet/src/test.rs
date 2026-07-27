#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: VestingWalletClient<'static>,
    token: TokenClient<'static>,
    admin: Address,
    beneficiary: Address,
    /// Ledger sequence at test setup time (used for computing schedule offsets).
    start: u32,
}

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|l| l.sequence_number = current + by);
}

/// Create a vesting schedule:
/// - cliff offset from start (0 = no cliff)
/// - vesting period length in ledgers
/// - total_amount tokens locked
/// - revocable flag
fn setup_schedule(cliff_offset: u32, period: u32, total: i128, revocable: bool) -> TestEnv {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&admin, &total);

    let start = env.ledger().sequence();
    let cliff = start + cliff_offset;
    let end = cliff + period; // end is always > cliff

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    client.initialize(
        &beneficiary,
        &asset.address(),
        &total,
        &start,
        &cliff,
        &end,
        &revocable,
        &admin,
    );

    let token = TokenClient::new(&env, &asset.address());

    let client = unsafe {
        core::mem::transmute::<VestingWalletClient<'_>, VestingWalletClient<'static>>(client)
    };
    let token = unsafe { core::mem::transmute::<TokenClient<'_>, TokenClient<'static>>(token) };

    TestEnv {
        env,
        client,
        token,
        admin,
        beneficiary,
        start,
    }
}

// Convenience wrappers.
fn setup(cliff_offset: u32, period: u32, total: i128) -> TestEnv {
    setup_schedule(cliff_offset, period, total, false)
}

fn setup_revocable(cliff_offset: u32, period: u32, total: i128) -> TestEnv {
    setup_schedule(cliff_offset, period, total, true)
}

// ── Initialisation tests ──────────────────────────────────────────────────────

#[test]
fn test_double_initialize_returns_error() {
    let t = setup(0, 100, 1_000);
    let another_beneficiary = Address::generate(&t.env);
    let another_admin = Address::generate(&t.env);
    // The token address is irrelevant here: AlreadyInitialized is returned before any transfer.
    let dummy_token = Address::generate(&t.env);
    let result = t.client.try_initialize(
        &another_beneficiary,
        &dummy_token,
        &500i128,
        &t.start,
        &t.start,
        &(t.start + 100),
        &false,
        &another_admin,
    );
    assert!(matches!(result, Err(Ok(VestingError::AlreadyInitialized))));
}

// ── vested_amount / releasable tests ─────────────────────────────────────────

#[test]
fn test_cliff_not_reached_releasable_is_zero() {
    // Cliff at +100 ledgers, period 100 (end at +200), advance only 50.
    let t = setup(100, 100, 1_000);
    advance_ledgers(&t.env, 50);
    assert_eq!(t.client.releasable(), 0);
    assert_eq!(t.client.vested_amount(), 0);
}

#[test]
fn test_at_cliff_ledger_vesting_begins() {
    // No cliff (cliff_offset = 0), period = 200, total = 1000.
    // At exactly cliff (= start), 0 * total / period = 0 vested (start == cliff, current == start).
    // Advance to cliff+1 to see first accrual.
    let t = setup(0, 200, 1_000);
    advance_ledgers(&t.env, 1);
    // vested = 1000 * 1 / 200 = 5
    assert_eq!(t.client.vested_amount(), 5);
}

#[test]
fn test_linear_vesting_midpoint() {
    // No cliff, period = 200, total = 1000. At ledger 100 (midpoint): vested = 500.
    let t = setup(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    assert_eq!(t.client.vested_amount(), 500);
    assert_eq!(t.client.releasable(), 500);
}

#[test]
fn test_past_end_full_amount_releasable() {
    // Period = 100, advance 300 (well past end). Full amount is releasable.
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 300);
    assert_eq!(t.client.vested_amount(), 1_000);
    assert_eq!(t.client.releasable(), 1_000);
}

#[test]
fn test_vested_amount_with_cliff() {
    // Cliff at +100, period = 100 (end at cliff+100 = start+200), total = 1000.
    // At ledger start+50: below cliff → vested = 0.
    // At ledger start+100 (exactly cliff): vested = 1000 * 100 / 200 = 500.
    // At ledger start+150: vested = 1000 * 150 / 200 = 750.
    let t = setup(100, 100, 1_000);

    advance_ledgers(&t.env, 50);
    assert_eq!(t.client.vested_amount(), 0);

    advance_ledgers(&t.env, 50); // now at start + 100 = cliff
    assert_eq!(t.client.vested_amount(), 500);

    advance_ledgers(&t.env, 50); // now at start + 150
    assert_eq!(t.client.vested_amount(), 750);
}

// ── release tests ─────────────────────────────────────────────────────────────

#[test]
fn test_release_transfers_correct_amount() {
    let t = setup(0, 200, 1_000);
    advance_ledgers(&t.env, 100); // 50% vested = 500

    let amount = t.client.release();
    assert_eq!(amount, 500);
    assert_eq!(t.token.balance(&t.beneficiary), 500);
    assert_eq!(t.client.released_amount(), 500);
    assert_eq!(t.client.releasable(), 0);
}

#[test]
fn test_release_twice_respects_already_released() {
    let t = setup(0, 200, 1_000);
    advance_ledgers(&t.env, 100); // 500 vested

    t.client.release(); // release 500

    advance_ledgers(&t.env, 50); // 750 vested now

    let second = t.client.release();
    assert_eq!(second, 250); // only the newly vested portion
    assert_eq!(t.token.balance(&t.beneficiary), 750);
    assert_eq!(t.client.released_amount(), 750);
}

#[test]
fn test_release_nothing_before_cliff() {
    let t = setup(100, 100, 1_000);
    advance_ledgers(&t.env, 50); // before cliff

    let amount = t.client.release();
    assert_eq!(amount, 0);
    assert_eq!(t.token.balance(&t.beneficiary), 0);
}

#[test]
fn test_release_full_amount_after_end() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 200); // past end

    let amount = t.client.release();
    assert_eq!(amount, 1_000);
    assert_eq!(t.token.balance(&t.beneficiary), 1_000);
    assert_eq!(t.client.releasable(), 0);
}

#[test]
fn test_release_emits_event() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    t.client.release();
    assert!(
        !t.env.events().all().events().is_empty(),
        "event not emitted"
    );
}

// ── revoke tests ──────────────────────────────────────────────────────────────

#[test]
fn test_revoke_when_not_revocable_returns_error() {
    let t = setup(0, 200, 1_000); // revocable = false
    advance_ledgers(&t.env, 100);
    assert!(matches!(
        t.client.try_revoke(),
        Err(Ok(VestingError::NotRevocable))
    ));
}

#[test]
fn test_revoke_twice_returns_already_revoked() {
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    t.client.revoke();
    assert!(matches!(
        t.client.try_revoke(),
        Err(Ok(VestingError::AlreadyRevoked))
    ));
}

#[test]
fn test_revoke_sends_unvested_to_admin() {
    // No cliff, period = 200, total = 1000. Revoke at ledger 100 (50% vested).
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);

    t.client.revoke();

    // Admin should receive the 500 unvested tokens.
    assert_eq!(t.token.balance(&t.admin), 500);
}

#[test]
fn test_revoke_midway_beneficiary_keeps_vested_portion() {
    // No cliff, period = 200, total = 1000.
    // At ledger 100: vested = 500, unvested = 500.
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);

    t.client.revoke();

    // Admin received unvested portion.
    assert_eq!(t.token.balance(&t.admin), 500);

    // Beneficiary can still claim the 500 that were vested.
    assert_eq!(t.client.releasable(), 500);
    let claimed = t.client.release();
    assert_eq!(claimed, 500);
    assert_eq!(t.token.balance(&t.beneficiary), 500);
}

#[test]
fn test_revoke_after_partial_release_only_returns_unvested() {
    // At ledger 100: 500 vested. Beneficiary releases all 500.
    // Revoke at the same ledger: vested_at_revoke = 500, unvested = 500.
    // Admin gets back the 500 unvested tokens. Beneficiary releasable = 0.
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100); // 500 vested

    t.client.release(); // beneficiary claims all 500; released_amount = 500

    t.client.revoke(); // vested frozen at 500; admin receives 500 unvested

    assert_eq!(t.token.balance(&t.admin), 500);
    assert_eq!(t.token.balance(&t.beneficiary), 500);

    // releasable = vested_frozen(500) - released(500) = 0
    assert_eq!(t.client.releasable(), 0);
}

#[test]
fn test_vested_amount_frozen_after_revoke() {
    // Vested at revoke = 500. After more ledgers pass, vested_amount stays at 500.
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    t.client.revoke();

    advance_ledgers(&t.env, 100); // would have vested more without revocation
    assert_eq!(t.client.vested_amount(), 500);
}

#[test]
fn test_revoke_emits_event() {
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    t.client.revoke();
    assert!(
        !t.env.events().all().events().is_empty(),
        "event not emitted"
    );
}

// ── released_amount test ──────────────────────────────────────────────────────

#[test]
fn test_released_amount_starts_at_zero() {
    let t = setup(0, 100, 1_000);
    assert_eq!(t.client.released_amount(), 0);
}

#[test]
fn test_released_amount_tracks_cumulative_releases() {
    let t = setup(0, 400, 1_000);
    advance_ledgers(&t.env, 100); // 25% vested = 250
    t.client.release();
    advance_ledgers(&t.env, 100); // 50% vested = 500
    t.client.release();

    assert_eq!(t.client.released_amount(), 500);
    assert_eq!(t.token.balance(&t.beneficiary), 500);
}
