#![cfg(test)]

use super::*;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    vec,
    xdr::{ContractEventBody, ScVal},
    Address, Env, IntoVal, TryFromVal,
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

#[test]
fn test_release_rejects_when_wallet_balance_is_insufficient() {
    let t = setup(0, 10, 1_000);
    advance_ledgers(&t.env, 5);

    // Simulate an underfunded wallet (for example, tokens recovered by an
    // administrator). The release must fail before updating ReleasedAmount.
    t.token.transfer(&t.contract_id, &t.admin, &600i128);

    assert_eq!(
        t.client.try_release(),
        Err(Ok(VestingError::InsufficientBalance))
    );
    assert_eq!(t.client.released_amount(), 0);
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

    // The longest schedule `initialize` accepts (#684), at the largest amount
    // the formula supports. Just before the end the elapsed portion is at its
    // largest, so `total * elapsed` is the biggest product the formula can see.
    client.initialize(
        &beneficiary,
        &asset.address(),
        &MAX_TOTAL_AMOUNT,
        &0u32,
        &0u32,
        &MAX_VESTING_DURATION_LEDGERS,
        &false,
        &admin,
    );
    env.ledger()
        .with_mut(|ledger| ledger.sequence_number = MAX_VESTING_DURATION_LEDGERS - 1);

    let vested = client.vested_amount();
    assert!(vested > 0);
    assert!(vested <= MAX_TOTAL_AMOUNT);
}

// ── Schedule length limit (#684) ──────────────────────────────────────────────

/// A deployed but uninitialised wallet whose `admin` already holds `funds` tokens,
/// for tests that call `initialize` with hand-picked ledgers.
struct FundedWallet {
    env: Env,
    contract_id: Address,
    token_address: Address,
    admin: Address,
    beneficiary: Address,
}

fn funded_uninitialised_wallet(funds: i128) -> FundedWallet {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    StellarAssetClient::new(&env, &asset.address()).mint(&admin, &funds);
    let contract_id = env.register(VestingWallet, ());
    FundedWallet {
        env,
        contract_id,
        token_address: asset.address(),
        admin,
        beneficiary,
    }
}

#[test]
fn test_max_vesting_duration_is_one_year_of_ledgers() {
    assert_eq!(MAX_VESTING_DURATION_SECS, 31_536_000);
    assert_eq!(MAX_VESTING_DURATION_LEDGERS, 6_307_200);
}

#[test]
fn test_invalid_schedule_error_code_is_stable() {
    assert_eq!(VestingError::InvalidSchedule as u32, 12);
}

#[test]
fn test_initialize_accepts_a_schedule_of_exactly_the_maximum_duration() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);
    let start = w.env.ledger().sequence();

    client.initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &start,
        &start,
        &(start + MAX_VESTING_DURATION_LEDGERS),
        &false,
        &w.admin,
    );

    let schedule = client.get_vesting_schedule();
    assert_eq!(
        schedule.end_ledger - schedule.start_ledger,
        MAX_VESTING_DURATION_LEDGERS
    );
}

#[test]
fn test_initialize_rejects_a_schedule_one_ledger_over_the_maximum() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);
    let start = w.env.ledger().sequence();

    let result = client.try_initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &start,
        &start,
        &(start + MAX_VESTING_DURATION_LEDGERS + 1),
        &false,
        &w.admin,
    );

    assert!(matches!(result, Err(Ok(VestingError::InvalidSchedule))));
}

#[test]
fn test_initialize_rejects_a_schedule_that_used_to_be_accepted_at_u32_max_duration() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);

    let result = client.try_initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &0u32,
        &0u32,
        &u32::MAX,
        &false,
        &w.admin,
    );

    assert!(matches!(result, Err(Ok(VestingError::InvalidSchedule))));
}

#[test]
fn test_the_limit_is_measured_from_start_so_a_late_cliff_does_not_extend_it() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);
    let start = w.env.ledger().sequence();

    // The cliff sits at the maximum duration and the end one ledger later:
    // only one ledger of linear vesting remains, but the schedule as a whole
    // is one ledger over the limit.
    let result = client.try_initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &start,
        &(start + MAX_VESTING_DURATION_LEDGERS),
        &(start + MAX_VESTING_DURATION_LEDGERS + 1),
        &false,
        &w.admin,
    );

    assert!(matches!(result, Err(Ok(VestingError::InvalidSchedule))));
}

#[test]
fn test_a_rejected_schedule_leaves_the_wallet_uninitialised_and_funds_untouched() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);
    let token = TokenClient::new(&w.env, &w.token_address);
    let start = w.env.ledger().sequence();

    let rejected = client.try_initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &start,
        &start,
        &(start + MAX_VESTING_DURATION_LEDGERS + 1),
        &false,
        &w.admin,
    );
    assert!(matches!(rejected, Err(Ok(VestingError::InvalidSchedule))));

    // Nothing was stored and no tokens moved.
    assert!(matches!(
        client.try_beneficiary(),
        Err(Ok(VestingError::NotInitialized))
    ));
    assert_eq!(token.balance(&w.admin), 1_000);
    assert_eq!(token.balance(&w.contract_id), 0);

    // The wallet is still free to be initialised with a valid schedule.
    client.initialize(
        &w.beneficiary,
        &w.token_address,
        &1_000i128,
        &start,
        &start,
        &(start + MAX_VESTING_DURATION_LEDGERS),
        &false,
        &w.admin,
    );
    assert_eq!(token.balance(&w.contract_id), 1_000);
}

#[test]
fn test_the_amount_ceiling_error_takes_precedence_over_the_duration_limit() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);

    // Both limits are broken: the amount is above the ceiling and the duration
    // is above the maximum. The amount error is reported, as before.
    let result = client.try_initialize(
        &w.beneficiary,
        &w.token_address,
        &(MAX_TOTAL_AMOUNT + 1),
        &0u32,
        &0u32,
        &u32::MAX,
        &false,
        &w.admin,
    );

    assert!(matches!(result, Err(Ok(VestingError::TotalAmountTooLarge))));
}

#[test]
fn test_import_vesting_enforces_the_same_duration_limit() {
    let w = funded_uninitialised_wallet(1_000);
    let client = VestingWalletClient::new(&w.env, &w.contract_id);
    let start = w.env.ledger().sequence();

    let too_long = VestingSchedule {
        beneficiary: w.beneficiary.clone(),
        token: w.token_address.clone(),
        total_amount: 1_000,
        start_ledger: start,
        cliff_ledger: start,
        end_ledger: start + MAX_VESTING_DURATION_LEDGERS + 1,
        revocable: false,
    };
    let rejected = client.try_import_vesting(&too_long, &w.token_address, &w.admin);
    assert!(matches!(rejected, Err(Ok(VestingError::InvalidSchedule))));

    let at_limit = VestingSchedule {
        end_ledger: start + MAX_VESTING_DURATION_LEDGERS,
        ..too_long
    };
    client.import_vesting(&at_limit, &w.token_address, &w.admin);
    assert_eq!(
        client.get_vesting_schedule().end_ledger,
        start + MAX_VESTING_DURATION_LEDGERS
    );
}

#[test]
fn test_ordinary_schedules_are_unaffected_by_the_limit() {
    // Short schedules, with and without a cliff, still initialise and vest.
    let t = setup(10, 90, 1_000);
    advance_ledgers(&t.env, 50);

    assert_eq!(t.client.vested_amount(), 500);
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
    assert!(!t.client.is_fully_released());
    advance_ledgers(&t.env, 200); // past end
    assert!(!t.client.is_fully_released());

    let amount = t.client.release();
    assert_eq!(amount, 1_000);
    assert_eq!(t.token.balance(&t.beneficiary), 1_000);
    assert_eq!(t.client.releasable(), 0);
    assert!(t.client.is_fully_released());
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

    // Filtered to this contract's own events: `release` also triggers the
    // stake token's `transfer` event, which is not part of this audit trail.
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
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

    // `events().all()` only reflects the most recent top-level call, so only
    // the second `release` invocation's event is present here. It must pair
    // this call's amount (250) with the running cumulative total (750), so an
    // indexer can track progress from the events alone. Filtered to this
    // contract's own events since `release` also triggers the stake token's
    // `transfer` event.
    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        vec![
            &t.env,
            (
                t.contract_id.clone(),
                vec![
                    &t.env,
                    symbol_short!("vest").into_val(&t.env),
                    symbol_short!("released").into_val(&t.env),
                ],
                (t.beneficiary.clone(), 250i128, 750i128).into_val(&t.env),
            )
        ]
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
    assert!(t.env.events().all().events().is_empty());
}

#[test]
fn test_release_immediately_after_cliff() {
    // Cliff at +100 ledgers, period 200 (end = start + 300), total = 1000.
    // Advance exactly to the cliff boundary and verify release works.
    let t = setup(100, 200, 1_000);

    // Before cliff: nothing releasable.
    advance_ledgers(&t.env, 99);
    assert_eq!(t.client.releasable(), 0);
    assert_eq!(t.client.release(), 0);

    // Advance to exactly cliff (start + 100).
    advance_ledgers(&t.env, 1);
    // Vested = 1000 * 100 / 300 = 333
    let vested_at_cliff = t.client.vested_amount();
    assert!(vested_at_cliff > 0, "tokens must be vested at cliff");
    assert_eq!(vested_at_cliff, 333);

    let released = t.client.release();
    assert_eq!(released, 333);
    assert_eq!(t.token.balance(&t.beneficiary), 333);
    assert_eq!(t.client.released_amount(), 333);
    assert_eq!(t.client.releasable(), 0);
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

// ── get_vesting_overview tests (#409) ─────────────────────────────────────────
//
// A single-call schedule *and* progress read, so a dashboard does not have to
// reconcile four separately-timed contract calls.

#[test]
fn test_get_vesting_overview_returns_schedule_and_progress() {
    let t = setup_schedule(50, 200, 1_000, true);
    advance_ledgers(&t.env, 100); // halfway between start and end

    let overview = t.client.get_vesting_overview();

    // Schedule fields match get_vesting_schedule exactly.
    assert_eq!(overview.beneficiary, t.beneficiary);
    assert_eq!(overview.token, t.token_address);
    assert_eq!(overview.total_amount, 1_000);
    assert_eq!(overview.start_ledger, t.start);
    assert_eq!(overview.cliff_ledger, t.start + 50);
    assert_eq!(overview.end_ledger, t.start + 250);
    assert!(overview.revocable);
    assert!(!overview.revoked);

    // Progress fields answer the three follow-up calls in one shot.
    // Vesting runs from start (not cliff) to end = start + 50 + 200 = start+250,
    // so at start+100 exactly 1000 * 100/250 is vested.
    assert_eq!(overview.vested_amount, 400);
    assert_eq!(overview.released_amount, 0);
    assert_eq!(overview.releasable_amount, 400);
}

#[test]
fn test_get_vesting_overview_reflects_amounts_already_released() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    t.client.release(); // 500 out

    let overview = t.client.get_vesting_overview();

    assert_eq!(overview.vested_amount, 500);
    assert_eq!(overview.released_amount, 500);
    assert_eq!(overview.releasable_amount, 0);
}

#[test]
fn test_get_vesting_overview_matches_the_individual_queries() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 40);

    let overview = t.client.get_vesting_overview();

    // The point of the combined call: it must agree with the separate reads.
    assert_eq!(overview.vested_amount, t.client.vested_amount());
    assert_eq!(overview.released_amount, t.client.released_amount());
    assert_eq!(overview.releasable_amount, t.client.releasable());
}

#[test]
fn test_get_vesting_overview_is_zero_before_the_cliff() {
    let t = setup(50, 200, 1_000);

    let overview = t.client.get_vesting_overview();

    assert_eq!(overview.vested_amount, 0);
    assert_eq!(overview.released_amount, 0);
    assert_eq!(overview.releasable_amount, 0);
}

#[test]
fn test_get_vesting_overview_reports_full_vesting_after_end() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 200);

    let overview = t.client.get_vesting_overview();

    assert_eq!(overview.vested_amount, 1_000);
    assert_eq!(overview.releasable_amount, 1_000);
}

#[test]
fn test_get_vesting_overview_reports_revocation() {
    let t = setup_revocable(0, 200, 1_000);
    advance_ledgers(&t.env, 100);
    t.client.revoke();

    let overview = t.client.get_vesting_overview();

    assert!(overview.revoked);
    // Vested is frozen at the revocation point, never decreasing below what has
    // already been released.
    assert_eq!(overview.vested_amount, 500);
    assert_eq!(overview.released_amount, 0);
    assert_eq!(overview.releasable_amount, 500);
}

#[test]
fn test_get_vesting_overview_releasable_never_goes_negative_after_emergency_withdraw() {
    // emergency_withdraw zeroes RevokedVested while released may already be
    // non-zero, so releasable must saturate rather than wrap negative.
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    t.client.release(); // 500 released
    t.client.emergency_withdraw(); // zeroes the frozen vested amount

    let overview = t.client.get_vesting_overview();

    assert!(overview.revoked);
    assert_eq!(overview.released_amount, 500);
    assert_eq!(overview.releasable_amount, 0);
    assert!(overview.releasable_amount >= 0);
}

#[test]
fn test_get_vesting_overview_uninitialized_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert!(matches!(
        client.try_get_vesting_overview(),
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

// ── release_all tests (#407) ──────────────────────────────────────────────────

#[test]
fn test_release_all_claims_everything_vested_in_one_call() {
    let t = setup(0, 100, 1_000);
    // Mid-schedule: 60% vested, and none of it claimed yet.
    advance_ledgers(&t.env, 60);

    let released = t.client.release_all();

    // One call claims the whole vested balance — no repeated release needed.
    assert_eq!(released, 600);
    assert_eq!(t.token.balance(&t.beneficiary), 600);
    assert_eq!(t.client.released_amount(), 600);
    assert_eq!(t.client.releasable(), 0);
}

#[test]
fn test_release_all_returns_zero_before_anything_vests() {
    let t = setup(50, 100, 1_000);
    // Still before the cliff.
    let released = t.client.release_all();

    assert_eq!(released, 0);
    assert_eq!(t.token.balance(&t.beneficiary), 0);
}

#[test]
fn test_release_all_after_end_claims_the_entire_schedule() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 200);

    assert_eq!(t.client.release_all(), 1_000);
    assert_eq!(t.token.balance(&t.beneficiary), 1_000);
}

#[test]
fn test_release_all_is_idempotent_after_a_full_claim() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 200);

    assert_eq!(t.client.release_all(), 1_000);
    // A second call has nothing left to hand over.
    assert_eq!(t.client.release_all(), 0);
    assert_eq!(t.token.balance(&t.beneficiary), 1_000);
}

#[test]
fn test_release_all_emits_the_same_event_as_release() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    let released = t.client.release_all();

    assert_eq!(
        t.env.events().all().filter_by_contract(&t.contract_id),
        vec![
            &t.env,
            (
                t.contract_id.clone(),
                vec![
                    &t.env,
                    symbol_short!("vest").into_val(&t.env),
                    symbol_short!("released").into_val(&t.env),
                ],
                (t.beneficiary.clone(), released, released).into_val(&t.env),
            )
        ],
        "release_all must stay on the same audited event as release"
    );
}

#[test]
fn test_release_all_requires_beneficiary_auth() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    t.env.mock_auths(&[MockAuth {
        address: &t.beneficiary,
        invoke: &MockAuthInvoke {
            contract: &t.contract_id,
            fn_name: "release_all",
            args: vec![&t.env],
            sub_invokes: &[],
        },
    }]);

    assert!(t.client.release_all() > 0);
}

#[test]
fn test_release_all_rejects_an_unauthorized_caller() {
    // The convenience entry point must not become a way for a third party to
    // force a release at a time the beneficiary did not choose.
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    t.env.mock_auths(&[]);

    assert!(t.client.try_release_all().is_err());
    assert_eq!(t.token.balance(&t.beneficiary), 0);
}

#[test]
fn test_release_all_on_uninitialized_wallet_errors() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert!(matches!(
        client.try_release_all(),
        Err(Ok(VestingError::NotInitialized))
    ));
}

#[test]
fn test_release_all_counts_as_a_release_operation() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    assert_eq!(t.client.release_count(), 0);

    t.client.release_all();

    assert_eq!(t.client.release_count(), 1);
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
        &zero_address(&env), // #405: unusable beneficiary
        &asset.address(),
        &1_000i128,
        &start,
        &start,
        &(start + 200),
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

    let rejected = t.client.try_transfer_beneficiary(&zero_address(&t.env));
    assert!(
        matches!(rejected, Err(Ok(VestingError::InvalidInput))),
        "transfer_beneficiary must reject the zero address, which could strand the funds"
    );

    // A real address still works, and the rejected call changed nothing.
    t.client.transfer_beneficiary(&good);
    assert_eq!(t.client.beneficiary(), good);
}

// ── cliff-end notification (#615) ─────────────────────────────────────────────

fn cliff_end_events(t: &TestEnv) -> std::vec::Vec<ScVal> {
    let topic_vals: soroban_sdk::Vec<soroban_sdk::Val> = vec![
        &t.env,
        symbol_short!("vest").into_val(&t.env),
        symbol_short!("cliff_end").into_val(&t.env),
    ];
    let mut topics: std::vec::Vec<ScVal> = std::vec::Vec::new();
    for value in topic_vals.iter() {
        topics.push(ScVal::try_from_val(&t.env, &value).unwrap());
    }

    t.env
        .events()
        .all()
        .events()
        .iter()
        .filter_map(|event| match &event.body {
            ContractEventBody::V0(body) if body.topics.as_slice() == topics.as_slice() => {
                Some(body.data.clone())
            }
            _ => None,
        })
        .collect()
}

fn assert_cliff_end_payload(t: &TestEnv, data: &ScVal) {
    let expected_val: soroban_sdk::Val =
        (t.beneficiary.clone(), t.env.ledger().timestamp()).into_val(&t.env);
    let expected = ScVal::try_from_val(&t.env, &expected_val).unwrap();
    assert_eq!(*data, expected);
}

#[test]
fn test_cliff_end_event_fires_once_when_cliff_is_reached() {
    let t = setup(10, 100, 1_000);

    advance_ledgers(&t.env, 9);
    assert_eq!(t.client.release(), 0);
    assert!(cliff_end_events(&t).is_empty());
    assert!(!t.client.cliff_end_notified());

    advance_ledgers(&t.env, 1);
    assert!(t.client.release() > 0);
    let events = cliff_end_events(&t);
    assert_eq!(events.len(), 1);
    assert_cliff_end_payload(&t, &events[0]);
    assert!(t.client.cliff_end_notified());

    t.client.release();
    assert!(cliff_end_events(&t).is_empty());
}

#[test]
fn test_no_cliff_schedule_does_not_emit_cliff_end_event() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);

    assert!(t.client.release() > 0);
    assert!(cliff_end_events(&t).is_empty());
    assert!(!t.client.cliff_end_notified());
}

#[test]
fn test_revoke_before_cliff_does_not_emit_cliff_end_event() {
    let t = setup_revocable(10, 100, 1_000);

    t.client.revoke();
    assert!(cliff_end_events(&t).is_empty());
    assert!(!t.client.cliff_end_notified());
}

#[test]
fn test_revoke_after_cliff_emits_cliff_end_event_once() {
    let t = setup_revocable(10, 100, 1_000);

    advance_ledgers(&t.env, 10);
    t.client.revoke();
    let events = cliff_end_events(&t);
    assert_eq!(events.len(), 1);
    assert_cliff_end_payload(&t, &events[0]);
    assert!(t.client.cliff_end_notified());
}

// ── #693 — backup_schedule ────────────────────────────────────────────────────

#[test]
fn test_backup_schedule_captures_the_configured_schedule() {
    let t = setup_revocable(10, 100, 1_000);

    let backup = t.client.backup_schedule(&t.beneficiary);

    assert_eq!(backup.version, 1);
    assert_eq!(backup.backed_up_at_ledger, t.env.ledger().sequence());
    assert_eq!(backup.schedule, t.client.get_vesting_schedule());
    assert_eq!(backup.schedule.total_amount, 1_000);
    assert!(backup.schedule.revocable);
    assert_eq!(backup.admin, t.admin);
    assert_eq!(backup.funder, t.admin);
    assert_eq!(backup.treasury, None);
    assert_eq!(backup.delegate, None);
    assert_eq!(backup.released_amount, 0);
    assert_eq!(backup.release_count, 0);
    assert!(!backup.revoked);
    assert_eq!(backup.revoked_vested, 0);
}

#[test]
fn test_backup_schedule_records_release_progress() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 50);
    let released = t.client.release_all();
    assert!(released > 0);

    let backup = t.client.backup_schedule(&t.beneficiary);

    assert_eq!(backup.released_amount, released);
    assert_eq!(backup.release_count, 1);
    assert_eq!(backup.backed_up_at_ledger, t.env.ledger().sequence());
}

#[test]
fn test_backup_schedule_records_revocation() {
    let t = setup_revocable(0, 100, 1_000);
    advance_ledgers(&t.env, 40);
    t.client.revoke();

    let backup = t.client.backup_schedule(&t.beneficiary);

    assert!(backup.revoked);
    assert_eq!(backup.revoked_vested, t.client.vested_amount());
    assert!(backup.revoked_vested > 0);
}

#[test]
fn test_backup_schedule_records_treasury_delegate_and_admin_transfer() {
    let t = setup(0, 100, 1_000);
    let treasury = Address::generate(&t.env);
    let delegate = Address::generate(&t.env);
    let new_admin = Address::generate(&t.env);
    t.client.set_treasury(&treasury);
    t.client.delegate(&delegate);
    t.client.transfer_admin(&new_admin);

    let backup = t.client.backup_schedule(&t.beneficiary);

    assert_eq!(backup.treasury, Some(treasury));
    assert_eq!(backup.delegate, Some(delegate));
    assert_eq!(backup.admin, new_admin);
    // The funder is fixed at initialization and survives an admin transfer.
    assert_eq!(backup.funder, t.admin);
}

#[test]
fn test_backup_schedule_rejects_a_different_beneficiary() {
    let t = setup(0, 100, 1_000);
    let stranger = Address::generate(&t.env);

    assert_eq!(
        t.client.try_backup_schedule(&stranger),
        Err(Ok(VestingError::InvalidInput))
    );
}

#[test]
fn test_backup_schedule_rejects_an_uninitialized_wallet() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);
    let anyone = Address::generate(&env);

    assert_eq!(
        client.try_backup_schedule(&anyone),
        Err(Ok(VestingError::NotInitialized))
    );
}

#[test]
fn test_backup_schedule_is_read_only_and_repeatable() {
    let t = setup(0, 100, 1_000);
    advance_ledgers(&t.env, 10);
    let overview_before = t.client.get_vesting_overview();

    let first = t.client.backup_schedule(&t.beneficiary);
    let second = t.client.backup_schedule(&t.beneficiary);

    assert_eq!(first, second);
    assert_eq!(t.client.get_vesting_overview(), overview_before);
    assert_eq!(t.client.release_count(), 0);
}

// ── compare_schedules ─────────────────────────────────────────────────────────

#[test]
fn test_compare_schedules_reports_diffs_and_winners() {
    let w = funded_uninitialised_wallet(3_000);
    let start = w.env.ledger().sequence();

    let a = VestingWalletClient::new(&w.env, &w.contract_id);
    a.initialize(&w.beneficiary, &w.token_address, &1_000i128, &start, &(start + 10), &(start + 100), &false, &w.admin);

    let b_id = w.env.register(VestingWallet, ());
    let b = VestingWalletClient::new(&w.env, &b_id);
    b.initialize(&w.beneficiary, &w.token_address, &2_000i128, &start, &(start + 50), &(start + 200), &false, &w.admin);

    advance_ledgers(&w.env, 50);

    // Called on wallet `a`, comparing itself (local read) with `b` (cross-contract).
    let cmp = a.compare_schedules(&w.contract_id, &b_id);
    assert_eq!(cmp.total_amount_diff, -1_000);
    assert_eq!(cmp.duration_a, 100);
    assert_eq!(cmp.duration_b, 200);
    assert_eq!(cmp.cliff_a, 10);
    assert_eq!(cmp.cliff_b, 50);
    assert_eq!(cmp.vested_bps_a, 5_000);
    assert_eq!(cmp.vested_bps_b, 2_500);
    assert!(cmp.same_token);
    assert_eq!(cmp.faster_vesting, Some(w.contract_id.clone()));
    assert_eq!(cmp.earlier_unlock, Some(w.contract_id.clone()));
}

#[test]
fn test_compare_schedules_rejects_same_wallet_and_non_wallets() {
    let w = funded_uninitialised_wallet(1_000);
    let start = w.env.ledger().sequence();
    let a = VestingWalletClient::new(&w.env, &w.contract_id);
    a.initialize(&w.beneficiary, &w.token_address, &1_000i128, &start, &start, &(start + 100), &false, &w.admin);

    assert_eq!(
        a.try_compare_schedules(&w.contract_id, &w.contract_id),
        Err(Ok(VestingError::InvalidInput))
    );
    let uninitialised = w.env.register(VestingWallet, ());
    assert_eq!(
        a.try_compare_schedules(&w.contract_id, &uninitialised),
        Err(Ok(VestingError::InvalidInput))
    );
}

// ── Issue #657: lock_schedule prevents further modifications ──────────────────

#[test]
fn test_lock_schedule_sets_locked_flag() {
    let w = setup_schedule(0, 1_000, 1_000, false);
    assert_eq!(w.client.is_schedule_locked(), false);
    w.client.lock_schedule();
    assert_eq!(w.client.is_schedule_locked(), true);
}

#[test]
fn test_lock_schedule_idempotent_error_on_second_call() {
    let w = setup_schedule(0, 1_000, 1_000, false);
    w.client.lock_schedule();
    assert_eq!(
        w.client.try_lock_schedule(),
        Err(Ok(VestingError::ScheduleAlreadyLocked)),
    );
}

#[test]
fn test_lock_schedule_emits_locked_event() {
    use soroban_sdk::{symbol_short, testutils::Events, IntoVal};

    let w = setup_schedule(0, 1_000, 1_000, false);
    w.client.lock_schedule();

    let events = w.env.events().all();
    let matched = events.iter().any(|(_, topics, _)| {
        topics
            == soroban_sdk::vec![
                &w.env,
                symbol_short!("vest").into_val(&w.env),
                symbol_short!("locked").into_val(&w.env),
            ]
    });
    assert!(matched, "vest/locked event was not published");
}

// ── Issue #642: vesting schedule parameters are validated on initialize ────────

#[test]
fn test_initialize_rejects_start_in_past() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    soroban_sdk::token::StellarAssetClient::new(&env, &asset.address()).mint(&admin, &1_000i128);

    let start = env.ledger().sequence();
    // Advance so start is in the past.
    advance_ledgers(&env, 10);

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    // Should panic with "start must be in the future"
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.initialize(
            &beneficiary,
            &asset.address(),
            &1_000i128,
            &start,       // in the past
            &start,
            &(start + 100),
            &false,
            &admin,
        );
    }));
    assert!(result.is_err(), "expected panic for past start_ledger");
}

#[test]
fn test_initialize_rejects_end_not_after_cliff() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    soroban_sdk::token::StellarAssetClient::new(&env, &asset.address()).mint(&admin, &1_000i128);

    let start = env.ledger().sequence();
    let cliff = start + 100;
    let end = cliff; // end == cliff, not strictly after

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.initialize(
            &beneficiary,
            &asset.address(),
            &1_000i128,
            &start,
            &cliff,
            &end,
            &false,
            &admin,
        );
    }));
    assert!(result.is_err(), "expected panic for end_ledger == cliff_ledger");
}

#[test]
fn test_initialize_rejects_schedule_exceeding_max_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin.clone());
    soroban_sdk::token::StellarAssetClient::new(&env, &asset.address()).mint(&admin, &1_000i128);

    let start = env.ledger().sequence();
    // MAX_VESTING_DURATION_LEDGERS = 365 * 24 * 3600 / 5 = 6_307_200
    let too_long_end = start + 6_307_201u32;

    let contract_id = env.register(VestingWallet, ());
    let client = VestingWalletClient::new(&env, &contract_id);

    assert_eq!(
        client.try_initialize(
            &beneficiary,
            &asset.address(),
            &1_000i128,
            &start,
            &start,
            &too_long_end,
            &false,
            &admin,
        ),
        Err(Ok(VestingError::InvalidSchedule)),
    );
}
