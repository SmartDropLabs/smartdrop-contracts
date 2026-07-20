#![cfg(test)]

//! A minimal token-interface contract for exercising checks-effects-
//! interactions (CEI) reentrancy scenarios in tests (#69). Configured with a
//! target contract + user, its `transfer` attempts to call back into the
//! target *during* the transfer — exactly what a non-standard `stake_token`
//! could do, since `token::TokenClient::transfer` is a synchronous
//! cross-contract call and `stake_token` is an admin-supplied address, not
//! necessarily a trusted Stellar Asset Contract.
//!
//! The reentrant call is made via `try_invoke_contract` rather than
//! `invoke_contract` so a rejected reentry doesn't itself trap this
//! contract's `transfer` (and, transitively, the caller's whole
//! invocation) — this lets a test assert on the outcome instead of just
//! catching a panic. Whether the reentry attempt was accepted or rejected
//! is recorded in this contract's own storage for the test to read back
//! afterward via `reentry_was_rejected()`.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

#[contracttype]
enum DataKey {
    Target,
    ReentrantUser,
    ReentryWasRejected,
}

#[contract]
pub struct MockReentrantToken;

#[contractimpl]
impl MockReentrantToken {
    /// `target` is the contract to reenter (e.g. the FarmingPool under
    /// test); `reentrant_user` is the user address to pass to the reentrant
    /// call.
    pub fn configure(env: Env, target: Address, reentrant_user: Address) {
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage()
            .instance()
            .set(&DataKey::ReentrantUser, &reentrant_user);
    }

    /// Matches the token interface's `transfer(from, to, amount)` exactly —
    /// this is the only function `token::TokenClient::transfer` invokes.
    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let target: Address = env.storage().instance().get(&DataKey::Target).unwrap();
        let reentrant_user: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReentrantUser)
            .unwrap();

        let args: Vec<Val> = soroban_sdk::vec![&env, reentrant_user.into_val(&env)];
        let result = env.try_invoke_contract::<Val, soroban_sdk::Error>(
            &target,
            &Symbol::new(&env, "get_user_position"),
            args,
        );

        // Ok(_) means the reentrant call actually went through (host allowed
        // it); Err(_) covers both a typed contract error and the host
        // rejecting the call outright (e.g. "Contract re-entry is not
        // allowed") — either way, the call did not return usable data, so
        // treat both as "rejected" for this test's purposes.
        let reentry_was_rejected = result.is_err();
        env.storage()
            .instance()
            .set(&DataKey::ReentryWasRejected, &reentry_was_rejected);
    }

    pub fn reentry_was_rejected(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::ReentryWasRejected)
            .unwrap_or(false)
    }
}

/// Same idea as [`MockReentrantToken`], but reenters via the plain,
/// panic-on-failure `invoke_contract` instead of `try_invoke_contract` — the
/// more naive (and arguably more realistic) way a hostile token author would
/// write this without any special handling. Used to confirm that even
/// without any graceful error handling in the token, a rejected reentry
/// safely aborts the *entire* invocation (including `lock_assets`'s state
/// writes) rather than leaving anything partially applied.
#[contract]
pub struct MockNaiveReentrantToken;

#[contractimpl]
impl MockNaiveReentrantToken {
    pub fn configure(env: Env, target: Address, reentrant_user: Address) {
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage()
            .instance()
            .set(&DataKey::ReentrantUser, &reentrant_user);
    }

    pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
        let target: Address = env.storage().instance().get(&DataKey::Target).unwrap();
        let reentrant_user: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReentrantUser)
            .unwrap();

        let args: Vec<Val> = soroban_sdk::vec![&env, reentrant_user.into_val(&env)];
        // No try_invoke_contract here — a rejected reentry traps this call
        // (and the whole transaction) immediately.
        let _: Val = env.invoke_contract(&target, &Symbol::new(&env, "get_user_position"), args);
    }
}
