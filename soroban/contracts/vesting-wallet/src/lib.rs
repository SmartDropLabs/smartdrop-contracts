#![no_std]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrows_for_generic_args)]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};
use types::DataKey;
pub use types::{AdminTransferred, VestingError, VestingSchedule};

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5 s/ledger).
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;

// The vesting formula multiplies total_amount by the elapsed portion of the
// schedule. Since valid ledger values are u32 and the largest valid duration
// is u32::MAX, this ceiling guarantees that product fits in i128 for every
// valid schedule: i128::MAX / u32::MAX.
const MAX_TOTAL_AMOUNT: i128 = i128::MAX / 4_294_967_295i128;

// ── Storage helpers ───────────────────────────────────────────────────────────

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
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
            (beneficiary, token, total_amount, start_ledger, end_ledger),
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

        let vested = compute_vested(&env)?;
        let released = get_released(&env);
        let releasable = vested.saturating_sub(released);

        if releasable == 0 {
            return Ok(0);
        }

        increment_release_count(&env);

        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &(released + releasable));

        token::TokenClient::new(&env, &get_token(&env)).transfer(
            &env.current_contract_address(),
            &beneficiary,
            &releasable,
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("released")),
            (beneficiary, releasable, released + releasable),
        );

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

        Ok(())
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
    pub fn transfer_beneficiary(env: Env, new_beneficiary: Address) -> Result<(), VestingError> {
        require_initialized(&env)?;
        let admin = get_admin(&env);
        admin.require_auth();
        bump_instance(&env);

        env.storage()
            .instance()
            .set(&DataKey::Beneficiary, &new_beneficiary);

        Ok(())
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
}

mod test;
