//! Workspace-level integration suite for issue #87.
//!
//! Per-crate unit tests (`factory/src/test.rs`, `farming-pool/src/test.rs`) each
//! register their own contract directly via `env.register(..)` and exercise it
//! in isolation. Neither suite proves that a farming-pool instance *born from
//! `Factory::create_pool`* — i.e. deployed from real, compiled farming-pool
//! WASM via `env.deployer().deploy_v2`, at the address the factory computed
//! and recorded — actually behaves like a farming pool once driven through a
//! `FarmingPoolClient`. That cross-contract deployment boundary (factory
//! deploys real bytecode, caller then talks to the deployed instance through
//! the *other* crate's generated client) is exactly what unit tests using
//! `env.register(FarmingPool, ())` cannot cover, because they never go through
//! `deploy_v2` at all. This file closes that gap. See issue #87.
//!
//! ## Why this lives in `factory/tests/`
//!
//! `factory/Cargo.toml` already carries `farming-pool` as a path
//! dev-dependency, and `cargo test --workspace` (the exact command CI's
//! "Run tests" step runs) automatically discovers and runs every file under
//! `factory/tests/` as its own integration-test binary. That makes this the
//! lowest-friction home: no new workspace member, no `[workspace.members]`
//! edit, and it is exercised by CI with zero additional wiring.
//!
//! ## Real WASM, not a native call
//!
//! Because this file is an external integration test (not `mod test;` inside
//! the crate), it only sees `factory`'s and `farming-pool`'s *public* API —
//! same as any real caller would. To deploy a genuine farming-pool instance
//! the way `Factory::create_pool` does in production, the test needs actual
//! compiled farming-pool WASM bytes to hand to
//! `env.deployer().upload_contract_wasm(..)`, exactly as
//! `factory/src/test.rs` does today with its small synthetic `MOCK_POOL_WASM`
//! blob — except here we want the *real* contract, not a stand-in, so the
//! lifecycle tests below actually exercise farming-pool's real logic.
//!
//! `tests/fixtures/farming_pool.wasm` is a checked-in build of the
//! farming-pool crate for the `wasm32v1-none` target (the target this
//! workspace's CI and `#[81]`'s remediation both settled on, since
//! soroban-sdk 25.3.1 rejects `wasm32-unknown-unknown` on modern Rust). It is
//! loaded via `include_bytes!` below, so `cargo test --workspace` needs no
//! prerequisite build step, no build.rs, and no manual instructions on a
//! fresh clone — the bytes are just part of the checked-in source tree.
//!
//! Trade-off: this fixture can go stale if farming-pool's source changes
//! without regenerating it. That risk is bounded — an interface drift (e.g. a
//! renamed or re-signatured method) will make the lifecycle tests below fail
//! loudly (link/invoke errors), not pass silently — and it avoids a much
//! riskier alternative: a `build.rs` that shells out to a nested `cargo build`
//! for every build of the `factory` package (including its production release
//! WASM build), which would slow down and complicate a build step that has
//! nothing to do with testing. Regenerate the fixture after any farming-pool
//! change with:
//!
//! ```text
//! cd soroban
//! cargo build -p farming-pool --target wasm32v1-none --release
//! cp target/wasm32v1-none/release/farming_pool.wasm \
//!    contracts/factory/tests/fixtures/farming_pool.wasm
//! ```

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

use factory::{Factory, FactoryClient};
use farming_pool::{FarmingPoolClient, PoolError};

/// Real, compiled farming-pool WASM — see the module doc comment above for
/// how this fixture is produced and kept fresh.
const FARMING_POOL_WASM: &[u8] = include_bytes!("fixtures/farming_pool.wasm");

fn advance_ledgers(env: &Env, by: u32) {
    let current = env.ledger().sequence();
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = current + by;
    });
}

/// Deploys a real Factory, initializes it with the real farming-pool WASM
/// hash, and creates one pool through `Factory::create_pool`. Returns the
/// live `FarmingPoolClient` bound to the address the factory itself computed
/// and recorded — never a directly-registered contract.
fn deploy_pool_via_factory(
    env: &Env,
    admin: &Address,
    asset: &Address,
    daily_rate: u128,
    min_lock_period: u64,
) -> (FarmingPoolClient<'static>, Address) {
    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);

    let factory_addr = env.register(Factory, ());
    let factory_client = FactoryClient::new(env, &factory_addr);
    factory_client.initialize(admin, &wasm_hash);

    let pool_id = factory_client.create_pool(asset, &daily_rate, &min_lock_period);
    let record = factory_client.get_pool(&pool_id);
    let pool_address = record.address.clone();

    let pool_client = FarmingPoolClient::new(env, &pool_address);
    let pool_client = unsafe {
        core::mem::transmute::<FarmingPoolClient<'_>, FarmingPoolClient<'static>>(pool_client)
    };
    (pool_client, pool_address)
}

/// GATE 1 smoke test: Factory deploys real farming-pool WASM and records a
/// live, queryable pool address.
#[test]
fn smoke_create_pool_returns_live_pool_address() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset = Address::generate(&env);

    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let factory_client = FactoryClient::new(&env, &factory_addr);
    factory_client.initialize(&admin, &wasm_hash);

    let pool_id = factory_client.create_pool(&asset, &100u128, &10u64);
    let record = factory_client.get_pool(&pool_id);

    // The deployed pool exists at a real address the factory tracked, and it
    // is reachable via FarmingPoolClient (a bogus/undeployed address would
    // panic on the first call below). create_pool (#79) already initialized
    // it atomically, so it must already report the factory's admin as its own.
    let pool_client = FarmingPoolClient::new(&env, &record.address);
    assert_eq!(pool_client.admin(), admin);
}

/// #79 acceptance criterion 1: `create_pool` deploys and initializes the
/// pool atomically, so the pool's admin is set to the factory's admin by the
/// time `create_pool` returns — there is no separate step the factory (or
/// anyone else) needs to perform afterwards.
#[test]
fn test_create_pool_pool_admin_is_set_atomically() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset = Address::generate(&env);

    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let factory_client = FactoryClient::new(&env, &factory_addr);
    factory_client.initialize(&admin, &wasm_hash);

    let pool_id = factory_client.create_pool(&asset, &100u128, &10u64);
    let record = factory_client.get_pool(&pool_id);

    let pool_client = FarmingPoolClient::new(&env, &record.address);
    assert_eq!(pool_client.admin(), admin);
}

/// #79 acceptance criterion 2: because `create_pool` initializes the pool in
/// the same invocation that deploys it, there is no externally-observable
/// window in which the pool is deployed but uninitialized. An unrelated
/// third party who tries to call the deployed pool's own `initialize` after
/// the fact — the exact front-running move #79 closes — must be rejected
/// with `PoolError::AlreadyInitialized`, not silently succeed and seize
/// admin.
#[test]
fn test_third_party_cannot_reinitialize_deployed_pool() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    let attacker = Address::generate(&env);

    let wasm_hash = env.deployer().upload_contract_wasm(FARMING_POOL_WASM);
    let factory_addr = env.register(Factory, ());
    let factory_client = FactoryClient::new(&env, &factory_addr);
    factory_client.initialize(&admin, &wasm_hash);

    let pool_id = factory_client.create_pool(&asset, &100u128, &10u64);
    let record = factory_client.get_pool(&pool_id);

    let pool_client = FarmingPoolClient::new(&env, &record.address);
    let result = pool_client.try_initialize(&attacker, &asset, &1u32, &1i128, &10u32);
    assert_eq!(result, Err(Ok(PoolError::AlreadyInitialized)));

    // The rejected re-initialize attempt must not have changed anything.
    assert_eq!(pool_client.admin(), admin);
}

/// Boost/stake lifecycle against a factory-deployed pool: stake → advance
/// ledgers → set_boost (checkpoints the pre-boost period) → advance ledgers
/// → unstake. Asserts exact token balances and exact credited amount,
/// reconciling both accrual periods (pre- and post-boost) by hand against
/// farming-pool's own accrual formula, plus confirms internal stake state is
/// cleared after unstake.
#[test]
fn end_to_end_create_pool_then_stake_and_unstake() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    let token = TokenClient::new(&env, &asset.address());

    const INITIAL_MINT: i128 = 1_000_000_000;
    token_sac.mint(&user, &INITIAL_MINT);

    let global_multiplier = 2u32;
    let credit_rate = 1i128;
    let daily_rate = 500u128;
    let min_lock_period: u32 = 0;

    let (pool_client, pool_address) = deploy_pool_via_factory(
        &env,
        &admin,
        &asset.address(),
        daily_rate,
        min_lock_period as u64,
    );

    // create_pool (#79) already initialized the pool atomically with the
    // factory's hardcoded defaults (global_multiplier=1, credit_rate=1);
    // reconfigure to this test's values via the pool's own admin setters
    // (factory's admin became the pool's admin, so this is authorized).
    pool_client.set_global_multiplier(&global_multiplier);

    let stake_amount: i128 = 1_000;
    pool_client.stake(&user, &stake_amount);

    // Stake debited from the user, credited to the pool contract.
    assert_eq!(token.balance(&user), INITIAL_MINT - stake_amount);
    assert_eq!(token.balance(&pool_address), stake_amount);

    // Period 1: 10 ledgers with no boost set (allocation_pct defaults to 0).
    let period1_ledgers: i128 = 10;
    advance_ledgers(&env, 10);

    // set_boost checkpoints the stake using the *old* (zero) allocation_pct
    // before recording the new one, then future checkpoints use 50%.
    let allocation_pct = 50u32;
    pool_client.set_boost(&user, &allocation_pct);

    // Period 2: 20 ledgers accruing under the new 50% boost.
    let period2_ledgers: i128 = 20;
    advance_ledgers(&env, 20);

    let total_credits = pool_client.unstake(&user);

    // Reconcile against farming-pool's own accrual formula:
    //   total_stake = principal + (boosted_amount * multiplier)
    //   credits = total_stake * credit_rate * elapsed_ledgers
    // Period 1 (0% allocation): total_stake == stake_amount unchanged.
    let period1_credits = stake_amount * credit_rate * period1_ledgers;
    // Period 2 (50% allocation, 2x multiplier): half principal, half virtual-doubled.
    let boosted = stake_amount * allocation_pct as i128 / 100;
    let principal = stake_amount - boosted;
    let period2_total_stake = principal + boosted * global_multiplier as i128;
    let period2_credits = period2_total_stake * credit_rate * period2_ledgers;
    let expected_credits = period1_credits + period2_credits;

    assert_eq!(total_credits, expected_credits);
    assert_eq!(total_credits, 40_000);

    // Full principal returned; pool and user balances reconcile exactly.
    assert_eq!(token.balance(&user), INITIAL_MINT);
    assert_eq!(token.balance(&pool_address), 0);

    // Internal stake state is cleared: a second unstake has nothing to act on.
    assert!(pool_client.try_unstake(&user).is_err());
}

/// Lock/unlock lifecycle against a factory-deployed pool: lock_assets →
/// unlock attempted before `min_lock_period` fails → advance ledgers past the
/// lock period → partial unlock → full unlock. Asserts exact token balances
/// and exact locked-position amounts at each step.
#[test]
fn end_to_end_create_pool_then_lock_and_unlock() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env.register_stellar_asset_contract_v2(token_admin);
    let token_sac = StellarAssetClient::new(&env, &asset.address());
    let token = TokenClient::new(&env, &asset.address());

    const INITIAL_MINT: i128 = 1_000_000_000;
    token_sac.mint(&user, &INITIAL_MINT);

    // global_multiplier is left at create_pool's hardcoded default (1) —
    // this test only exercises lock/unlock, which isn't affected by it.
    let credit_rate = 2i128;
    let daily_rate = 300u128;
    let min_lock_period_ledgers = 50u32;

    let (pool_client, pool_address) = deploy_pool_via_factory(
        &env,
        &admin,
        &asset.address(),
        daily_rate,
        min_lock_period_ledgers as u64,
    );

    // create_pool (#79) already initialized the pool atomically with the
    // factory's hardcoded defaults (global_multiplier=1, credit_rate=1);
    // reconfigure to this test's values via the pool's own admin setters
    // (factory's admin became the pool's admin, so this is authorized).
    pool_client.set_credit_rate(&credit_rate);

    let lock_amount: i128 = 2_000;
    pool_client.lock_assets(&user, &lock_amount);

    assert_eq!(token.balance(&user), INITIAL_MINT - lock_amount);
    assert_eq!(token.balance(&pool_address), lock_amount);

    // Only 30 of the required 50 ledgers have elapsed — must fail. Today
    // farming-pool enforces this via a plain `assert!` (no dedicated
    // `PoolError` variant exists for "lock period not elapsed"), so the only
    // thing a caller can observe is that the call errors — same assertion
    // style as farming-pool's own unit test
    // `test_unlock_blocked_before_min_lock_period`.
    advance_ledgers(&env, 30);
    assert!(pool_client.try_unlock_assets(&user, &lock_amount).is_err());

    // Confirm the failed attempt did not mutate locked state.
    let position_after_failed_unlock = pool_client.get_user_position(&user).unwrap();
    assert_eq!(position_after_failed_unlock.amount, lock_amount);

    // Advance past the remaining lock period (30 + 30 = 60 >= 50).
    advance_ledgers(&env, 30);

    let partial_unlock: i128 = 800;
    pool_client.unlock_assets(&user, &partial_unlock);

    let remaining = lock_amount - partial_unlock;
    assert_eq!(token.balance(&user), INITIAL_MINT - remaining);
    assert_eq!(token.balance(&pool_address), remaining);

    let position_after_partial_unlock = pool_client.get_user_position(&user).unwrap();
    assert_eq!(position_after_partial_unlock.amount, remaining);

    // Unlock the remainder; the position must be cleared entirely.
    pool_client.unlock_assets(&user, &remaining);

    assert_eq!(token.balance(&user), INITIAL_MINT);
    assert_eq!(token.balance(&pool_address), 0);
    assert!(pool_client.get_user_position(&user).is_none());
}
