#![no_std]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrows_for_generic_args)]

mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env};
use types::DataKey;
pub use types::VestingError;

// Persistent-storage TTL: extend to ~60 days if below ~30 days (at ~5 s/ledger).
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 1_036_800;

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

// ── Vesting formula ───────────────────────────────────────────────────────────

/// Linear vesting with cliff.
///
/// Returns 0 before cliff, the full total once end is reached, and a linear
/// proportion in between (measured from start, not from cliff). If the
/// schedule has been revoked, returns the frozen vested amount captured at
/// the moment of revocation.
fn compute_vested(env: &Env) -> i128 {
    if is_revoked(env) {
        return env
            .storage()
            .instance()
            .get(&DataKey::RevokedVested)
            .unwrap_or(0);
    }

    let current = env.ledger().sequence() as i128;
    let cliff = get_cliff_ledger(env) as i128;
    let start = get_start_ledger(env) as i128;
    let end = get_end_ledger(env) as i128;
    let total = get_total_amount(env);

    if current < cliff {
        return 0;
    }
    if current >= end {
        return total;
    }

    total * (current - start) / (end - start)
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
        assert!(cliff_ledger >= start_ledger, "cliff must be >= start");
        assert!(end_ledger > cliff_ledger, "end must be > cliff");

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
        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &0i128);

        // Pull tokens from admin into the contract.
        token::TokenClient::new(&env, &token).transfer(
            &admin,
            &env.current_contract_address(),
            &total_amount,
        );

        bump_instance(&env);
        Ok(())
    }

    /// Transfer all vested-but-unclaimed tokens to the beneficiary.
    ///
    /// Permissionless: tokens always flow to the stored beneficiary address.
    /// Returns the amount transferred (0 if nothing is releasable).
    pub fn release(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);

        let vested = compute_vested(&env);
        let released = get_released(&env);
        let releasable = vested - released;

        if releasable == 0 {
            return Ok(0);
        }

        env.storage()
            .instance()
            .set(&DataKey::ReleasedAmount, &(released + releasable));

        let beneficiary = get_beneficiary(&env);
        token::TokenClient::new(&env, &get_token(&env)).transfer(
            &env.current_contract_address(),
            &beneficiary,
            &releasable,
        );

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("released")),
            (beneficiary, releasable),
        );

        Ok(releasable)
    }

    /// Admin: cancel the unvested portion and return it to admin.
    ///
    /// Only callable when `revocable = true`. Tokens vested at the time of the
    /// call remain claimable by the beneficiary via `release()`. The unvested
    /// remainder is transferred back to admin immediately.
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

        let vested = compute_vested(&env);
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
                &admin,
                &unvested,
            );
        }

        #[allow(deprecated)]
        env.events().publish(
            (symbol_short!("vest"), symbol_short!("revoked")),
            (admin, vested, unvested),
        );

        Ok(())
    }

    /// Return the total amount vested as of the current ledger.
    pub fn vested_amount(env: Env) -> Result<i128, VestingError> {
        require_initialized(&env)?;
        bump_instance(&env);
        Ok(compute_vested(&env))
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
        Ok(compute_vested(&env) - get_released(&env))
    }
}

mod test;
