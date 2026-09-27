#![cfg(test)]

use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, IntoVal,
};

// The contract crate is `#![no_std]`; the event assertions below collect into
// `std` collections, so the shim is declared here (same as factory's tests).
extern crate std;

// ── Test helpers ──────────────────────────────────────────────────────────────

struct TestEnv {
    env: Env,
    client: VestingWalletClient<'static>,
    contract_id: Address,
    token: TokenClient<'static>,
    token_address: Address,
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
        contract_id,
        token,
        token_address: asset.address(),
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
fn test_third_party_can_front_run_initialize() {
    // This documents the standalone deployment limitation: initialize does not
    // know which account deployed the contract, so the first valid call wins.
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    let attacker = Address::generate(&env);
    let intended_admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&attacker, &100i128);

    client.initialize(
        &beneficiary,
        &asset.address(),
        &100i128,
        &0u32,
        &0u32,
        &100u32,
        &false,
        &attacker,
    );

    let result = client.try_initialize(
        &beneficiary,
        &asset.address(),
        &100i128,
        &0u32,
        &0u32,
        &100u32,
        &false,
        &intended_admin,
    );
    assert!(matches!(result, Err(Ok(VestingError::AlreadyInitialized))));
}

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

#[test]
fn test_token_getter_returns_initialized_token_address() {
    let t = setup(0, 100, 1_000);
    assert_eq!(t.client.token(), t.token_address);
}

#[test]
#[should_panic(expected = "start must be in the future")]
fn test_initialize_rejects_start_ledger_in_the_past() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&admin, &100i128);

    advance_ledgers(&env, 10);
    let current = env.ledger().sequence();
    let past_start = current - 1;

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    client.initialize(
        &beneficiary,
        &asset.address(),
        &100i128,
        &past_start,
        &current,
        &(current + 100),
        &false,
        &admin,
    );
}

#[test]
fn test_initialize_rejects_total_amount_above_compute_vested_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = Address::generate(&env);

    let result = client.try_initialize(
        &beneficiary,
        &token,
        &(MAX_TOTAL_AMOUNT + 1),
        &0u32,
        &0u32,
        &u32::MAX,
        &false,
        &admin,
    );
    assert!(matches!(result, Err(Ok(VestingError::TotalAmountTooLarge))));
}

#[test]
fn test_compute_vested_is_safe_at_maximum_duration_and_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&admin, &MAX_TOTAL_AMOUNT);

    client.initialize(
        &beneficiary,
        &asset.address(),
        &MAX_TOTAL_AMOUNT,
        &0u32,
        &0u32,
        &u32::MAX,
        &false,
        &admin,
    );
    env.ledger()
        .with_mut(|ledger| ledger.sequence_number = 2_000_000_000);

    let vested = client.vested_amount();
    assert!(vested > 0);
    assert!(vested <= MAX_TOTAL_AMOUNT);
}

#[test]
fn test_emergency_withdraw_returns_raw_balance_to_admin() {
    let t = setup(0, 100, 1_000);
    assert_eq!(t.token.balance(&t.admin), 0);

    let returned = t.client.emergency_withdraw();

    assert_eq!(returned, 1_000);
    assert_eq!(t.token.balance(&t.admin), 1_000);
    assert_eq!(t.token.balance(&t.contract_id), 0);
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

// #408 — indexers reconstruct vesting progress purely from these events, so
// the payload is the audit trail: it must name the beneficiary, the amount
// released by *this* call, and the cumulative total released afterwards.
// Asserting only that "an event exists" would let any of those three values
// regress (or the topic rename) without a single test failing, which is exactly
// the failure mode the issue describes.
#[test]
fn test_release_event_payload_identifies_beneficiary_and_amounts() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    let released_now = t.client.release(); // half of 1_000 is vested at 50/100

    assert_eq!(
        t.env.events().all(),
        vec![
            &t.env,
            (
                t.contract_id.clone(),
                vec![
                    &t.env,
                    symbol_short!("vest").into_val(&t.env),
                    symbol_short!("released").into_val(&t.env),
                ],
                (t.beneficiary.clone(), released_now, released_now).into_val(&t.env),
            )
        ]
    );
}

#[test]
fn test_release_event_with_cumulative_total() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    t.client.release(); // 500 released

    advance_ledgers(&t.env, 25);
    t.client.release(); // 250 releasable, cumulative total 750

    // The second event must report this call's 250 *and* the running 750, so an
    // indexer can track progress from the events alone.
    let events = t.env.events().all();
    let released_events: Vec<_> = events
        .events()
        .iter()
        .filter(|(_, topics, _)| {
            topics
                == &vec![
                    &t.env,
                    symbol_short!("vest").into_val(&t.env),
                    symbol_short!("released").into_val(&t.env),
                ]
        })
        .collect();

    assert_eq!(released_events.len(), 2);
    let (_, _, second_payload) = released_events[1];
    assert_eq!(
        second_payload,
        (t.beneficiary.clone(), 250i128, 750i128).into_val(&t.env),
        "cumulative released total must be carried in the event"
    );
}

#[test]
fn test_release_with_nothing_releasable_emits_no_event() {
    let t = setup(0, 100, 1_000);
    // Before the cliff nothing is releasable, so there is no transfer to
    // report: an event here would tell indexers a release happened when it
    // did not.
    let amount = t.client.release();

    assert_eq!(amount, 0);
    let released_events = t.env.events().all().events().iter().filter(|(_, topics, _)| {
        topics
            == &vec![
                &t.env,
                symbol_short!("vest").into_val(&t.env),
                symbol_short!("released").into_val(&t.env),
            ]
    });
    assert_eq!(released_events.count(), 0);
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
fn test_revoked_reflects_revocation_state() {
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    assert!(!t.client.revoked());

    t.client.revoke();
    assert!(t.client.revoked());
}

#[test]
fn test_revoked_uninitialized_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert!(matches!(
        client.try_revoked(),
        Err(Ok(VestingError::NotInitialized))
    ));
}

#[test]
fn test_beneficiary_getter_returns_configured_address() {
    let t = setup(50, 200, 1_000);
    assert_eq!(t.client.beneficiary(), t.beneficiary);
}

#[test]
fn test_beneficiary_getter_tracks_transfer_beneficiary() {
    let t = setup(50, 200, 1_000);
    let new_beneficiary = Address::generate(&t.env);

    t.client.transfer_beneficiary(&new_beneficiary);

    assert_eq!(t.client.beneficiary(), new_beneficiary);
}

#[test]
fn test_beneficiary_uninitialized_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert!(matches!(
        client.try_beneficiary(),
        Err(Ok(VestingError::NotInitialized))
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

// ── get_vesting_schedule tests ────────────────────────────────────────────────

#[test]
fn test_get_vesting_schedule_returns_all_parameters() {
    let t = setup_schedule(50, 200, 1_000, true);

    let schedule = t.client.get_vesting_schedule();

    assert_eq!(schedule.beneficiary, t.beneficiary);
    assert_eq!(schedule.token, t.token_address);
    assert_eq!(schedule.total_amount, 1_000);
    assert_eq!(schedule.start_ledger, t.start);
    assert_eq!(schedule.cliff_ledger, t.start + 50);
    assert_eq!(schedule.end_ledger, t.start + 250);
    assert!(schedule.revocable);
}

#[test]
fn test_get_vesting_schedule_reflects_transferred_beneficiary() {
    let t = setup(0, 100, 1_000);

    let new_beneficiary = Address::generate(&t.env);
    t.client.transfer_beneficiary(&new_beneficiary);

    let schedule = t.client.get_vesting_schedule();
    assert_eq!(schedule.beneficiary, new_beneficiary);
    assert_eq!(schedule.total_amount, 1_000);
    assert_eq!(schedule.start_ledger, t.start);
    assert_eq!(schedule.end_ledger, t.start + 100);
    assert!(!schedule.revocable);
}

#[test]
fn test_get_vesting_schedule_uninitialized_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert!(matches!(
        client.try_get_vesting_schedule(),
        Err(Ok(VestingError::NotInitialized))
    ));
}

#[test]
fn test_release_count_increments_on_release() {
    let t = setup(50, 200, 1_000);

    // Initially 0
    assert_eq!(t.client.release_count(), 0);
    assert_eq!(t.client.get_release_count(), 0);

    // Release before cliff (0 tokens releasable) does not increment count
    assert_eq!(t.client.release(), 0);
    assert_eq!(t.client.release_count(), 0);

    // Advance past cliff and release tokens
    advance_ledgers(&t.env, 100);
    let released1 = t.client.release();
    assert!(released1 > 0);
    assert_eq!(t.client.release_count(), 1);
    assert_eq!(t.client.get_release_count(), 1);

    // Immediate subsequent release (0 tokens) does not increment
    assert_eq!(t.client.release(), 0);
    assert_eq!(t.client.release_count(), 1);

    // Advance to end and release remainder
    advance_ledgers(&t.env, 200);
    let released2 = t.client.release();
    assert!(released2 > 0);
    assert_eq!(t.client.release_count(), 2);
    assert_eq!(t.client.get_release_count(), 2);
}

#[test]
fn test_release_requires_beneficiary_auth() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    t.env.mock_auths(&[MockAuth {
        address: &t.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &t.contract_id,
            fn_name: "release",
            args: vec![&t.env],
            sub_invokes: &[],
        },
    }]);

    let released = t.client.release();
    assert!(released > 0);
}

// ── admin access-control regression tests (#406) ──────────────────────────────
//
// The reported `clawback` entry point does not exist in this contract, but
// the underlying concern — "no access control on sensitive operations" — is
// worth pinning down. These tests authorize only a NON-admin address and
// assert that every admin-gated entry point rejects the call, so any future
// entry point added without `require_auth()` fails here.

#[test]
fn test_admin_only_entry_points_require_admin_auth() {
    let t = setup_revocable(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    let stranger = Address::generate(&t.env);

    // Authorize the stranger for a harmless read only: no admin-gated entry
    // point has a matching authorization, so `require_auth()` must fail.
    t.env.mock_auths(&[]);

    assert!(
        t.client.try_revoke().is_err(),
        "revoke must require the stored admin's authorization"
    );
    assert!(
        t.client.try_emergency_withdraw().is_err(),
        "emergency_withdraw must require the stored admin's authorization"
    );
    assert!(
        t.client.try_transfer_beneficiary(&stranger).is_err(),
        "transfer_beneficiary must require the stored admin's authorization"
    );
    assert!(
        t.client.try_transfer_admin(&stranger).is_err(),
        "transfer_admin must require the current admin's authorization"
    );

    // Nothing moved: the rejected calls must not have changed any state.
    assert_eq!(t.client.beneficiary(), t.beneficiary);
    assert_eq!(t.client.admin(), t.admin);
    assert!(!t.client.revoked());
    assert_eq!(t.client.released_amount(), 0);
    assert_eq!(
        t.token.balance(&t.contract_id),
        1_000,
        "vested tokens must still be held after rejected admin calls"
    );

    // The admin's own authorization still works, proving the rejections above
    // were about authorization and not a broken fixture.
    t.env.mock_all_auths();
    t.client.revoke();
    assert!(t.client.revoked());
}

// ── beneficiary validation tests (#405) ───────────────────────────────────────

#[test]
fn test_initialize_rejects_the_zero_beneficiary() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    token_sac.mint(&admin, &1_000i128);

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    let start = env.ledger().sequence();
    let result = client.try_initialize(
        &Address::default(), // #405: unusable beneficiary
        &asset.address(),
        &1_000i128,
        &start,
        &start,
        &start + 200,
        &false,
        &admin,
    );

    assert!(
        matches!(result, Err(Ok(VestingError::InvalidInput))),
        "initialize must reject the zero beneficiary address"
    );
}

#[test]
fn test_transfer_beneficiary_rejects_the_zero_address() {
    let t = setup(0, 200, 1_000);
    let good = Address::generate(&t.env);

    let rejected = t
        .client
        .try_transfer_beneficiary(&Address::default());
    assert!(
        matches!(rejected, Err(Ok(VestingError::InvalidInput))),
        "transfer_beneficiary must reject the zero address, which could strand the funds"
    );

    // A real address still works, and the rejected call changed nothing.
    t.client.transfer_beneficiary(&good);
    assert_eq!(t.client.beneficiary(), good);
}
