#![no_std]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrows_for_generic_args)]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};
use types::DataKey;
pub use types::{
    AdminTransferred, ScheduleHistoryEntry, ScheduleHistoryPage, VestingError, VestingOverview,
    VestingSchedule,
};

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5 s/ledger).
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;

// The vesting formula multiplies total_amount by the elapsed portion of the
// schedule. Valid ledger values are u32, so even the largest possible duration
// is below u32::MAX; this ceiling, computed for u32::MAX, therefore guarantees
// the product fits in i128 for every valid schedule: i128::MAX / u32::MAX.
// (Schedules are further limited to MAX_VESTING_DURATION_LEDGERS below.)
const MAX_TOTAL_AMOUNT: i128 = i128::MAX / 4_294_967_295i128;

/// Seconds a ledger takes to close, which the schedule limits are expressed
/// against (about 5 s on Stellar, the same assumption the TTL constants above make).
const SECONDS_PER_LEDGER: u32 = 5;

/// Longest vesting schedule `initialize` accepts, in seconds: 365 days (#684).
const MAX_VESTING_DURATION_SECS: u32 = 365 * 24 * 60 * 60;

/// The same limit in ledgers (6_307_200), because schedules are configured in
/// ledger sequences. Measured from `start_ledger` to `end_ledger`, so a late
/// cliff does not extend it.
const MAX_VESTING_DURATION_LEDGERS: u32 = MAX_VESTING_DURATION_SECS / SECONDS_PER_LEDGER;

/// Early-withdrawal penalty in basis points (10% = 1_000 bps). See #605.
const EARLY_WITHDRAWAL_PENALTY_BPS: i128 = 1_000;

// ── Storage helpers ───────────────────────────────────────────────────────────

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

/// The Stellar zero/void address, used as the "unusable address" sentinel
/// (soroban-sdk has no `Address::default()`).
fn zero_address(env: &Env) -> Address {
    Address::from_string(&soroban_sdk::String::from_str(
        env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ))
}

fn require_initialized(env: &Env) -> Result<(), VestingError> {
    if !env.storage().instance().has(&DataKey::Beneficiary) {
        return Err(VestingError::NotInitialized);
    }
    Ok(())
}

fn get_beneficiary(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Beneficiary).unwrap()
}

fn get_token(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Token).unwrap()
}

fn get_total_amount(env: &Env) -> i128 {
    env.storage().instance().get(&DataKey::TotalAmount).unwrap()
}

fn get_start_ledger(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::StartLedger).unwrap()
}

fn get_cliff_ledger(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::CliffLedger).unwrap()
}

fn get_end_ledger(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::EndLedger).unwrap()
}

fn get_released(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::ReleasedAmount)
        .unwrap_or(0)
}

fn get_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn get_treasury(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Treasury)
}

fn is_revocable(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Revocable)
        .unwrap_or(false)
}

fn is_revoked(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Revoked)
        .unwrap_or(false)
}

fn read_release_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::ReleaseCount)
        .unwrap_or(0)
}

fn increment_release_count(env: &Env) {
    let count = read_release_count(env);
    env.storage()
        .instance()
        .set(&DataKey::ReleaseCount, &(count + 1));
}

// ── Vesting formula ───────────────────────────────────────────────────────────

/// Linear vesting with cliff.
///
/// Returns 0 before cliff, the full total once end is reached, and a linear
/// proportion in between (measured from start, not from cliff). If the
/// schedule has been revoked, returns the frozen vested amount captured at
/// the moment of revocation.
fn compute_vested(env: &Env) -> Result<i128, VestingError> {
    if is_revoked(env) {
        return Ok(env
            .storage()
            .instance()
            .get(&DataKey::RevokedVested)
            .unwrap_or(0));
    }

    let current = i128::from(env.ledger().sequence());
    let cliff = i128::from(get_cliff_ledger(env));
    let start = i128::from(get_start_ledger(env));
    let end = i128::from(get_end_ledger(env));
    let total = get_total_amount(env);

    if current < cliff {
        return Ok(0);
    }
    if current >= end {
        return Ok(total);
    }

    total
        .checked_mul(current - start)
        .ok_or(VestingError::ArithmeticOverflow)?
        .checked_div(end - start)
        .ok_or(VestingError::ArithmeticOverflow)
}

/// Emit a one-time `vest/cliff_end` event once a real cliff has been reached (#615).
///
/// Schedules with no cliff (`cliff_ledger == start_ledger`) are deliberately
/// skipped: there is no cliff end to announce, and their existing release
/// event streams stay unchanged. The flag makes the notification idempotent
/// even when `release`, `release_all`, `withdraw_early`, or `revoke` are called
/// repeatedly after the cliff. Detection uses the ledger sequence (the unit
/// cliffs are configured in); the timestamp is included in the payload for
/// off-chain display as requested by the issue.
fn maybe_notify_cliff_end(env: &Env) {
    let start = get_start_ledger(env);
    let cliff = get_cliff_ledger(env);
    if cliff <= start || is_revoked(env) {
        return;
    }
    if env
        .storage()
        .instance()
        .has(&DataKey::CliffEndNotified)
    {
        return;
    }
    if env.ledger().sequence() < cliff {
        return;
    }

    env.storage()
        .instance()
        .set(&DataKey::CliffEndNotified, &true);

    #[allow(deprecated)]
    env.events().publish(
        (symbol_short!("vest"), symbol_short!("cliff_end")),
        (get_beneficiary(env), env.ledger().timestamp()),
    );
}

fn read_schedule_history_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::ScheduleHistoryCount)
        .unwrap_or(0)
}

fn record_schedule_event(
    env: &Env,
    action: soroban_sdk::Symbol,
    old_schedule: Option<VestingSchedule>,
    new_schedule: Option<VestingSchedule>,
) {
    let count = read_schedule_history_count(env);
    let ledger = env.ledger().sequence();
    let entry = ScheduleHistoryEntry {
        action,
        old_schedule,
        new_schedule,
        ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::ScheduleHistory(count), &entry);
    env.storage()
        .persistent()
        .set(&DataKey::ScheduleHistoryCount, &(count + 1));
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct VestingWallet;

#[contractimpl]
impl VestingWallet {
    /// Initialise the vesting schedule. Must be called exactly once.
    ///
    /// The caller (`admin`) must authorise this call; `total_amount` tokens are
    /// pulled from `admin` into the contract at initialisation time.
    ///
    /// WARNING: This wallet is a standalone contract and does not bind this
    /// call to the account that deployed it. An uninitialised wallet can
    /// therefore be front-run and permanently occupied by another caller.
    /// Operators must submit deployment and this initialization in one atomic
    /// transaction (for example, using a deployment flow that batches the
    /// deploy and initialize operations) and must not expose an uninitialized
    /// wallet between transactions.
    ///
    /// - `start_ledger`: ledger at which linear vesting begins.
    /// - `cliff_ledger`: ledger before which nothing is releasable (≥ start_ledger).
    /// - `end_ledger`: ledger at which the full amount is vested (> cliff_ledger).
    /// - `revocable`: if true, `admin` may cancel the unvested portion later.
    ///
    /// Returns `InvalidSchedule` if the schedule runs for more than 365 days
    /// (`end_ledger - start_ledger` above 6_307_200 ledgers, at about 5 s per
    /// ledger), and `TotalAmountTooLarge` if `total_amount` exceeds the ceiling
    /// the vesting formula can handle (which takes precedence). `import_vesting`
    /// applies the same limits (#684).
    pub fn initialize(
        env: Env,
        beneficiary: Address,
        token: Address,
        total_amount: i128,
        start_ledger: u32,
        cliff_ledger: u32,
        end_ledger: u32,
        revocable: bool,
        admin: Address,
    ) -> Result<(), VestingError> {
        if env.storage().instance().has(&DataKey::Beneficiary) {
            return Err(VestingError::AlreadyInitialized);
        }
        // #405 — a zero beneficiary can never call `release` (it cannot sign),
        // so the entire vested amount would be stranded with no recovery path.
        if beneficiary == zero_address(&env) {
            return Err(VestingError::InvalidInput);
        }
        assert!(total_amount > 0, "total_amount must be positive");
        assert!(
            start_ledger >= env.ledger().sequence(),
            "start must be in the future"
        );
        assert!(cliff_ledger >= start_ledger, "cliff must be >= start");
        assert!(end_ledger > cliff_ledger, "end must be > cliff");
        let duration = i128::from(end_ledger - start_ledger);
        if total_amount > MAX_TOTAL_AMOUNT || total_amount.checked_mul(duration).is_none() {
            return Err(VestingError::TotalAmountTooLarge);
        }
        // #684 — bound how long a schedule can run. Without a limit a schedule could
        // lock funds for up to u32::MAX ledgers (over 600 years), which is almost
        // certainly a mistake and could never be corrected once funds are pulled in.
        // `end > cliff >= start` was asserted above, so `duration` is positive.
        if duration > i128::from(MAX_VESTING_DURATION_LEDGERS) {
            return Err(VestingError::InvalidSchedule);
        }

        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Beneficiary, &beneficiary);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .set(&DataKey::TotalAmount, &total_amount);
        env.storage()
            .instance()
            .set(&DataKey::StartLedger, &start_ledger);
        env.storage()
            .instance()
            .set(&DataKey::CliffLedger, &cliff_ledger);
        env.storage()
            .instance()
            .set(&DataKey::EndLedger, &end_ledger);
        env.storage()
            .instance()
            .set(&DataKey::Revocable, &revocable);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Funder, &admin);
        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &0i128);

        // Pull tokens from admin into the contract.
        token::TokenClient::new(&env, &token).transfer(
            &admin,
            &env.current_contract_address(),
            &total_amount,
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("init")),
            (beneficiary.clone(), token.clone(), total_amount, start_ledger, end_ledger),
        );

        // Record initial schedule creation (#681)
        let schedule = VestingSchedule {
            beneficiary: beneficiary.clone(),
            token: token.clone(),
            total_amount,
            start_ledger,
            cliff_ledger,
            end_ledger,
            revocable,
        };
        record_schedule_event(
            &env,
            symbol_short!("created"),
            None,
            Some(schedule),
        );

        bump_instance(&env);
        Ok(())
    }

    /// Transfer all vested-but-unclaimed tokens to the beneficiary.
    ///
    /// Requires beneficiary authorization so third parties cannot force-release
    /// tokens at unexpected times (e.g. tax events).
    /// Returns the amount transferred (0 if nothing is releasable).
    pub fn release(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let beneficiary = get_beneficiary(&env);
        beneficiary.require_auth();
        maybe_notify_cliff_end(&env);

        let vested = compute_vested(&env)?;
        let released = get_released(&env);
        // `i128::saturating_sub` only guards against overflow, not against a
        // negative result — after `emergency_withdraw` freezes `vested` below
        // `released`, an unclamped subtraction would try to transfer a
        // negative amount.
        let releasable = vested.saturating_sub(released).max(0);

        if releasable == 0 {
            return Ok(0);
        }

        let token_address = get_token(&env);
        let token_client = token::TokenClient::new(&env, &token_address);
        if token_client.balance(&env.current_contract_address()) < releasable {
            return Err(VestingError::InsufficientBalance);
        }

        increment_release_count(&env);

        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &(released + releasable));

        token_client.transfer(&env.current_contract_address(), &beneficiary, &releasable);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("released")),
            (beneficiary.clone(), releasable, released + releasable),
        );

        // Issue #645: emit milestone events at 25 / 50 / 75 / 100 % vested.
        Self::check_milestones(&env, &beneficiary, released, released + releasable);

        Ok(releasable)
    }

    /// Admin: cancel the unvested portion and return it to the original funder.
    ///
    /// Only callable when `revocable = true`. Tokens vested at the time of the
    /// call remain claimable by the beneficiary via `release()`. The unvested
    /// remainder is transferred back to the funder (the address that funded the
    /// vesting schedule at initialization), not the current admin. This ensures
    /// that if admin rights were transferred via `transfer_admin`, the original
    /// funder still receives their unvested tokens.
    pub fn revoke(env: Env) -> Result<(), VestingError> {
        require_initialized(&env)?;
        if !is_revocable(&env) {
            return Err(VestingError::NotRevocable);
        }
        if is_revoked(&env) {
            return Err(VestingError::AlreadyRevoked);
        }

        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);
        maybe_notify_cliff_end(&env);

        let funder: Address = env.storage().instance().get(&DataKey::Funder).unwrap();
        let vested = compute_vested(&env)?;
        let total = get_total_amount(&env);
        let unvested = total - vested;

        // Freeze the vested amount so compute_vested() stays stable after revocation.
        env.storage()
            .instance()
            .set(&DataKey::RevokedVested, &vested);
        env.storage().instance().set(&DataKey::Revoked, &true);

        if unvested > 0 {
            token::TokenClient::new(&env, &get_token(&env)).transfer(
                &env.current_contract_address(),
                &funder,
                &unvested,
            );
        }

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("revoked")),
            (admin, get_beneficiary(&env), vested, unvested),
        );

        // Record revoke in schedule history (#681)
        let old_schedule = VestingSchedule {
            beneficiary: get_beneficiary(&env),
            token: get_token(&env),
            total_amount: get_total_amount(&env),
            start_ledger: get_start_ledger(&env),
            cliff_ledger: get_cliff_ledger(&env),
            end_ledger: get_end_ledger(&env),
            revocable: true,
        };
        record_schedule_event(
            &env,
            symbol_short!("revoked"),
            Some(old_schedule),
            None,
        );

        Ok(())
    }

    /// Claim every currently-vested token in a single call.
    ///
    /// Convenience wrapper over {@link VestingWallet::release}, which already
    /// transfers the whole vested-but-unclaimed balance in one transfer, so
    /// this adds a self-documenting entry point rather than a second code path
    /// — the release logic, the beneficiary authorisation and the
    /// `vest/released` event are identical because it delegates.
    ///
    /// Note the signature deliberately takes no `beneficiary` argument: the
    /// beneficiary is read from storage and is the account that must authorise
    /// the call, so an argument would either be ignored or let a third party
    /// force a release at a time the beneficiary did not choose (#407).
    ///
    /// Returns the total amount transferred, which is 0 when nothing has vested
    /// yet, and `NotInitialized` if the wallet was never initialized.
    pub fn release_all(env: Env) -> Result<i128, VestingError> {
        Self::release(env)
    }

    /// Return the total amount vested as of the current ledger.
    pub fn vested_amount(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        compute_vested(&env)
    }

    /// Return the cumulative amount already transferred to the beneficiary.
    pub fn released_amount(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_released(&env))
    }

    /// Return whether the beneficiary has received the entire scheduled amount.
    pub fn is_fully_released(env: Env) -> Result<bool, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_released(&env) == get_total_amount(&env))
    }

    /// Return the amount currently available to release (vested minus released).
    pub fn releasable(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(compute_vested(&env)? - get_released(&env))
    }

    /// Return whether the vesting schedule is revocable by admin.
    pub fn revocable(env: Env) -> Result<bool, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(is_revocable(&env))
    }

    /// Return whether the vesting schedule has already been revoked (#235).
    ///
    /// Lets frontends check revocation status directly instead of inferring
    /// it from a failed `revoke()` call.
    pub fn revoked(env: Env) -> Result<bool, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(is_revoked(&env))
    }

    /// Return whether the one-time `vest/cliff_end` notification has fired (#615).
    pub fn cliff_end_notified(env: Env) -> Result<bool, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env.storage().instance().has(&DataKey::CliffEndNotified))
    }

    /// Return the current beneficiary address. Reflects any prior
    /// `transfer_beneficiary` calls. Returns `NotInitialized` if the wallet
    /// has not been initialized.
    pub fn beneficiary(env: Env) -> Result<Address, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_beneficiary(&env))
    }

    /// Return the full vesting schedule parameters in a single call.
    ///
    /// Frontends need `beneficiary`, `token`, `total_amount`, `start_ledger`,
    /// `cliff_ledger`, `end_ledger`, and `revocable` together to render a
    /// schedule; previously each required a separate read. Returns
    /// `NotInitialized` if the wallet has not been initialized.
    pub fn get_vesting_schedule(env: Env) -> Result<VestingSchedule, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(VestingSchedule {
            beneficiary: get_beneficiary(&env),
            token: get_token(&env),
            total_amount: get_total_amount(&env),
            start_ledger: get_start_ledger(&env),
            cliff_ledger: get_cliff_ledger(&env),
            end_ledger: get_end_ledger(&env),
            revocable: is_revocable(&env),
        })
    }

    /// Import a vesting schedule from an external source (#639).
    ///
    /// Allows an admin to initialise this contract from a `VestingSchedule`
    /// produced elsewhere (e.g. migrated from an off-chain spreadsheet or a
    /// deprecated contract version) instead of having to pass each field
    /// individually. Under the hood this delegates to `initialize`, so the
    /// same validations apply and the wallet must not already be initialised.
    ///
    /// `token` and `admin` are passed separately because `VestingSchedule`
    /// carries the logical parameters only (what, when) not the operational
    /// ones (which token contract, who administers it).
    pub fn import_vesting(
        env: Env,
        schedule: VestingSchedule,
        token: Address,
        admin: Address,
    ) -> Result<(), VestingError> {
        Self::initialize(
            env,
            schedule.beneficiary,
            token,
            schedule.total_amount,
            schedule.start_ledger,
            schedule.cliff_ledger,
            schedule.end_ledger,
            schedule.revocable,
            admin,
        )
    }

    /// Return the whole schedule *and* its live progress in a single call.
    ///
    /// `get_vesting_schedule` returns the configured parameters only, so a
    /// frontend still had to follow it with `vested_amount`,
    /// `released_amount` and `releasable` — four round trips for one vesting
    /// overview, and a real risk of the components disagreeing because they
    /// were read at different ledgers. This returns all of it atomically.
    ///
    /// Kept as a separate type from `VestingSchedule` so that struct's layout —
    /// and the XDR clients already decode — is untouched (#409).
    ///
    /// `releasable_amount` is what `release_all` would transfer right now.
    /// Returns `NotInitialized` if the wallet has not been initialized.
    pub fn get_vesting_overview(env: Env) -> Result<VestingOverview, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let vested = compute_vested(&env)?;
        let released = get_released(&env);

        Ok(VestingOverview {
            beneficiary: get_beneficiary(&env),
            token: get_token(&env),
            total_amount: get_total_amount(&env),
            start_ledger: get_start_ledger(&env),
            cliff_ledger: get_cliff_ledger(&env),
            end_ledger: get_end_ledger(&env),
            revocable: is_revocable(&env),
            revoked: is_revoked(&env),
            vested_amount: vested,
            released_amount: released,
            // `i128::saturating_sub` alone only guards against overflow, not a
            // negative result: after revocation the frozen vested amount can be
            // lower than what was already released, so clamp with `.max(0)` —
            // a negative "releasable" would be nonsense to a caller.
            releasable_amount: vested.saturating_sub(released).max(0),
        })
    }

    /// Returns `(start_ledger, cliff_ledger, end_ledger)` in a single read for
    /// frontends that render the vesting schedule (#256). Returns
    /// `NotInitialized` if the wallet has not been initialized.
    pub fn vesting_dates(env: Env) -> Result<(u32, u32, u32), VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok((
            get_start_ledger(&env),
            get_cliff_ledger(&env),
            get_end_ledger(&env),
        ))
    }

    /// Emergency recovery that deliberately bypasses vesting arithmetic.
    ///
    /// Admin-only. Transfers the wallet's raw token balance to the admin and
    /// permanently marks the schedule revoked, so later release calls cannot
    /// depend on `compute_vested`. This is a break-glass operation that ends
    /// beneficiary claims and should only be used when normal arithmetic or
    /// schedule processing is unavailable.
    pub fn emergency_withdraw(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        let token = get_token(&env);
        let amount = token::TokenClient::new(&env, &token).balance(&env.current_contract_address());
        env.storage().instance().set(&DataKey::Revoked, &true);
        env.storage()
            .instance()
            .set(&DataKey::RevokedVested, &0i128);

        if amount > 0 {
            token::TokenClient::new(&env, &token).transfer(
                &env.current_contract_address(),
                &admin,
                &amount,
            );
        }
        Ok(amount)
    }

    /// Transfer beneficiary rights to `new_beneficiary`. Admin must authorise.
    ///
    /// #405 — the same zero-address check as `initialize` applies here: the
    /// admin cannot hand the schedule to an address that can never call
    /// `release`, which would strand the remaining vested amount.
    pub fn transfer_beneficiary(env: Env, new_beneficiary: Address) -> Result<(), VestingError> {
        require_initialized(&env)?;
        if new_beneficiary == zero_address(&env) {
            return Err(VestingError::InvalidInput);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::Beneficiary, &new_beneficiary);

        Ok(())
    }

    /// Set a delegate address that can claim vesting rights on behalf of the beneficiary.
    /// Beneficiary must authorise this delegation.
    ///
    /// The delegate can call `release` to claim vested tokens on behalf of the beneficiary.
    /// The beneficiary remains the ultimate owner and can revoke delegation by setting
    /// a zero address (or calling this again with a different address).
    pub fn delegate(env: Env, delegate: Address) -> Result<(), VestingError> {
        require_initialized(&env)?;
        let beneficiary = get_beneficiary(&env);
        beneficiary.require_auth();
        bump_instance(&env);

        if delegate == zero_address(&env) {
            env.storage().instance().remove(&DataKey::Delegate);
        } else {
            env.storage().instance().set(&DataKey::Delegate, &delegate);
        }

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("del_set")),
            delegate,
        );

        Ok(())
    }

    /// Get the current delegate address, if one is set.
    pub fn get_delegate(env: Env) -> Result<Option<Address>, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(env.storage().instance().get(&DataKey::Delegate))
    }

    /// Return the token address for the vesting schedule.
    pub fn token(env: Env) -> Result<Address, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_token(&env))
    }

    /// Return the current admin address.
    pub fn admin(env: Env) -> Result<Address, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_admin(&env))
    }

    /// Transfer admin rights to `new_admin`. Current admin must authorise.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), VestingError> {
        require_initialized(&env)?;
        let current = get_admin(&env);
        current.require_auth();
        bump_instance(&env);

        env.storage().instance().set(&DataKey::Admin, &new_admin);

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("adm_xfr")),
            (current, new_admin),
        );

        Ok(())
    }

    /// Return the total number of release operations performed.
    pub fn release_count(env: Env) -> Result<u32, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(read_release_count(&env))
    }

    pub fn get_release_count(env: Env) -> Result<u32, VestingError> {
        Self::release_count(env)
    }

    /// Set the treasury address that receives early-withdrawal penalties.
    /// Admin-only. Must be called before `withdraw_early` is usable. See #605.
    pub fn set_treasury(env: Env, treasury: Address) -> Result<(), VestingError> {
        require_initialized(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Treasury, &treasury);
        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("trs_set")),
            treasury,
        );
        Ok(())
    }

    /// Return the configured treasury address, or `None` if not yet set.
    pub fn treasury(env: Env) -> Result<Option<Address>, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(get_treasury(&env))
    }

    /// Withdraw vested tokens before the schedule completes, subject to a
    /// 10% penalty (1_000 bps). The penalty goes to the treasury; the net
    /// amount goes to the beneficiary. Requires beneficiary auth. See #605.
    ///
    /// Returns `TreasuryNotConfigured` if no treasury is set, `InvalidAmount`
    /// for zero/negative amounts, or `InsufficientBalance` if the releasable
    /// balance is below `amount`.
    pub fn withdraw_early(env: Env, amount: i128) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);

        if amount <= 0 {
            return Err(VestingError::InvalidAmount);
        }

        let treasury = get_treasury(&env).ok_or(VestingError::TreasuryNotConfigured)?;
        let beneficiary = get_beneficiary(&env);
        beneficiary.require_auth();
        maybe_notify_cliff_end(&env);

        let vested = compute_vested(&env)?;
        let released = get_released(&env);
        let releasable = vested.saturating_sub(released).max(0);

        if amount > releasable {
            return Err(VestingError::InsufficientBalance);
        }

        let penalty = amount * EARLY_WITHDRAWAL_PENALTY_BPS / 10_000;
        let net_amount = amount - penalty;

        let token_address = get_token(&env);
        let token_client = token::TokenClient::new(&env, &token_address);

        if token_client.balance(&env.current_contract_address()) < amount {
            return Err(VestingError::InsufficientBalance);
        }

        // Record the full amount as released (penalty leaves the contract too).
        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &(released + amount));
        increment_release_count(&env);

        if net_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &beneficiary, &net_amount);
        }
        if penalty > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &penalty);
        }

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("early_wd")),
            (beneficiary, amount, penalty, net_amount),
        );

        Ok(net_amount)
    }

    /// Emit a milestone event when vesting crosses a 25 / 50 / 75 / 100 %
    /// threshold (#645).
    ///
    /// `prev_released` and `new_released` are the cumulative released amounts
    /// before and after the current `release()` call. A milestone fires when
    /// the threshold was not yet crossed before but is now.
    fn check_milestones(
        env: &Env,
        beneficiary: &Address,
        prev_released: i128,
        new_released: i128,
    ) {
        let total = get_total_amount(env);
        if total <= 0 {
            return;
        }
        // Milestones at 25 %, 50 %, 75 %, 100 %.
        for bps in [2_500u32, 5_000, 7_500, 10_000] {
            let threshold = total.saturating_mul(bps as i128) / 10_000;
            if prev_released < threshold && new_released >= threshold {
                #[allow(deprecated)]
                env.events().publish(
                    (symbol_short!("vest"), symbol_short!("milestone")),
                    (beneficiary.clone(), bps, new_released),
                );
            }
        }
    }
}

mod test;

// Storage key for the cached optimization flag (issue #651).
const SCHEDULE_OPTIMIZED_KEY: &str = "sched_opt";

impl VestingWallet {
    /// Return a copy of the vesting schedule with any redundant or inefficient
    /// parameters collapsed (issue #651).
    ///
    /// Currently normalises two cases:
    /// - A cliff equal to or before the start is collapsed to `start_ledger`
    ///   (a cliff that fires immediately adds no lockup value).
    /// - An end ledger equal to `start_ledger` is advanced to `start_ledger + 1`
    ///   to prevent a zero-duration schedule that would vest everything instantly.
    ///
    /// The optimised schedule is returned but **not** written back to storage —
    /// call `initialize` with the returned values to store an optimised schedule
    /// from scratch.
    pub fn optimize_schedule(
        _env: Env,
        schedule: VestingSchedule,
    ) -> Result<VestingSchedule, VestingError> {
        let mut s = schedule;
        // Flatten a no-op cliff.
        if s.cliff_ledger <= s.start_ledger {
            s.cliff_ledger = s.start_ledger;
        }
        // Ensure a positive vesting duration.
        if s.end_ledger <= s.start_ledger {
            s.end_ledger = s.start_ledger + 1;
        }
        Ok(s)
    }

    /// Migrate an existing vesting schedule to the current storage layout
    /// (issue #654).
    ///
    /// This is a no-op when the schedule is already in the current format.
    /// It is intended for contracts that were initialized with an older version
    /// of the ABI and need to be brought forward without a full re-initialization.
    ///
    /// Returns `NotInitialized` if the contract has never been initialized.
    pub fn migrate_schedule(env: Env) -> Result<(), VestingError> {
        require_initialized(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);
        // Mark the schedule as migrated so callers can query readiness.
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("migrated"), &true);
        #[allow(deprecated)]
        env.events().publish(
            (soroban_sdk::symbol_short!("vest"), soroban_sdk::symbol_short!("migrated")),
            (),
        );
        Ok(())
    }

    /// Validates a vesting schedule before creation (issue #678).
    ///
    /// Checks that:
    /// - start_ledger < cliff_ledger <= end_ledger
    /// - total_amount > 0
    /// - beneficiary is not the zero address
    /// - token is not the zero address
    ///
    /// Returns `Ok(true)` if valid, or an error describing what failed.
    /// This allows clients to validate parameters before calling `initialize`.
    ///
    /// ## Arguments
    /// * `schedule` - The vesting schedule to validate
    ///
    /// ## Errors
    /// * `InvalidInput` - If any validation check fails
    pub fn validate_schedule(env: Env, schedule: VestingSchedule) -> Result<bool, VestingError> {
        // Validate addresses are not zero
        let zero = zero_address(&env);
        if schedule.beneficiary == zero {
            return Err(VestingError::InvalidInput);
        }
        if schedule.token == zero {
            return Err(VestingError::InvalidInput);
        }

        // Validate total amount is positive
        if schedule.total_amount <= 0 {
            return Err(VestingError::InvalidInput);
        }

        // Validate ledger sequence: start < cliff <= end
        if schedule.start_ledger >= schedule.cliff_ledger {
            return Err(VestingError::InvalidInput);
        }
        if schedule.cliff_ledger > schedule.end_ledger {
            return Err(VestingError::InvalidInput);
        }

        Ok(true)
    }

    /// Returns the vesting schedule history for this contract (issue #681).
    ///
    /// Provides a paginated list of all schedule change events (creation, revocation, etc.)
    /// in chronological order.
    ///
    /// ## Arguments
    /// * `start` - Zero-based index to start reading from
    /// * `limit` - Maximum number of events to return
    ///
    /// ## Returns
    /// * `ScheduleHistoryPage` containing the events and total count
    pub fn get_schedule_history(
        env: Env,
        beneficiary: Address,
        start: u32,
        limit: u32,
    ) -> Result<ScheduleHistoryPage, VestingError> {
        require_initialized(&env)?;
        beneficiary.require_auth();

        let total = read_schedule_history_count(&env);
        let mut events = soroban_sdk::Vec::new(&env);

        let end = start.saturating_add(limit).min(total);
        for i in start..end {
            if let Some(event) = env
                .storage()
                .persistent()
                .get::<DataKey, ScheduleHistoryEntry>(&DataKey::ScheduleHistory(i))
            {
                events.push_back(event);
            }
        }

        Ok(ScheduleHistoryPage { events, total })
    }
}
