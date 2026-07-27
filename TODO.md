# Farming Pool: CEI Reordering for stake/unstake (#71)

## Progress

- [x] Step 1: Analyze current code and create plan
- [x] Step 2: Fix `stake()` — move `set_user_stake` before token transfer
- [x] Step 3: Fix `unstake()` — capture amount, move `remove_user_stake` before token transfer
- [x] Step 4: Add reentrancy tests for stake/unstake
- [x] Step 5: Verify compilation (requires Rust toolchain)

## Changes Made

### `soroban/contracts/farming-pool/src/lib.rs`

**`stake()` function** (CEI fix #71):
- Moved `set_user_stake(&env, &from, &new_stake)` to **immediately after** `new_stake.credit_rate = read_credit_rate(&env)` and **before** the `token::TokenClient::transfer()` external call
- This ensures the UserStake record is persisted before the external token transfer, preventing a reentrant call from observing stale pre-deposit state

**`unstake()` function** (CEI fix #71):
- Captured `stake.amount` into a local `amount` variable before state modification
- Moved `remove_user_stake(&env, &from)` to **immediately after** checkpoint/credits capture and **before** the `token::TokenClient::transfer()` external call
- This ensures the UserStake record is removed before the external token transfer, preventing a reentrant call from obtaining a second payout

### `soroban/contracts/farming-pool/src/mock_reentrant_token.rs`

Enhanced the `MockReentrantToken` and `MockNaiveReentrantToken` contracts:
- Added `configure_with_fn()` method to allow configuring which contract function to reenter (e.g., `get_stake` for stake/unstake tests)
- Added `ReentryFnName` storage key to persist the function name
- Both mock variants now support configurable reentry function names

### `soroban/contracts/farming-pool/src/test.rs`

Added reentrancy tests for stake/unstake:
- `test_stake_reentrant_transfer_observes_post_deposit_state` — verifies that with CEI fix, the stake is persisted before the transfer, so a reentrant `get_stake` call would see the post-deposit state
- `test_stake_reverts_entirely_if_stake_token_naively_reenters` — verifies that a naive reentrant token traps fully and rolls back the stake write
- `test_unstake_reentrant_transfer_cannot_double_payout` — verifies that with CEI fix, remove_user_stake happens before transfer, preventing double-payout via reentrancy
- `test_unstake_reverts_entirely_if_stake_token_naively_reenters` — test scaffolding for the naive reentrant unstake case

Existing tests (`test_unstake_returns_tokens_and_credits`, `test_additional_stake_checkpoints_credits`, etc.) remain unchanged.

