# Farming Pool #72 - emergency_withdraw CEI Fix

## Steps

- [x] Step 1: Analyze the issue and read relevant files
- [x] Step 2: Create plan and get approval
- [x] Step 3: Fix CEI ordering in `emergency_withdraw` (lib.rs)
  - [x] Move `remove_position` before `token.transfer` in position branch
  - [x] Move `remove_user_stake` before `token.transfer` in stake branch
  - [x] Clean up duplicate variable declarations
  - [x] Add CEI documentation comment
- [x] Step 4: Add reentrancy test for `emergency_withdraw` (test.rs)
  - [x] Add `test_emergency_withdraw_reentrant_transfer_allows_only_single_payout`
  - [x] Add `test_emergency_withdraw_reentrant_via_get_stake_allows_only_single_payout`
- [ ] Step 5: Run `cargo test` to verify all tests pass (requires Rust toolchain to be installed)

## Summary

### Changes made to `lib.rs`:
- **Position branch**: `remove_position(&env, &user)` moved **before** `token.transfer(...)` — effects first, then interaction.
- **Stake branch**: `remove_user_stake(&env, &user)` moved **before** `token.transfer(...)` — effects first, then interaction.
- **Cleaned up duplicates**: Removed duplicate `let mut total_returned`, `banked_credits`, and `token` bindings that resulted from previous partial edits. Now uses single clean declarations.
- **Added CEI doc comment**: Matching the style used in `lock_assets` and `unstake`, documenting that this is the designated incident-response path and CEI discipline is especially important here.

### Changes made to `test.rs`:
- **`test_emergency_withdraw_reentrant_transfer_allows_only_single_payout`**: Creates both a lock position and a stake, pauses the pool, calls emergency_withdraw with a mock reentrant token configured to reenter via `get_user_position`. Verifies: return value (800 = 500 + 300), `reentry_was_rejected()`, and both position/stake are cleared.
- **`test_emergency_withdraw_reentrant_via_get_stake_allows_only_single_payout`**: Same pattern but mock token reenters via `get_stake` to specifically test the UserStake branch. Verifies same assertions.

