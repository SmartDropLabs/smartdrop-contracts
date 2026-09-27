# Farming Pool Security and Testing Improvements

This document describes the fixes applied to address issues #357, #358, #363, and #364.

## Issue #363: Transfer Success Validation in lock_assets

### Problem
The `lock_assets` function calls `token::TokenClient::transfer` but doesn't explicitly validate the return value, raising concerns about silent failures.

### Analysis
After code review, this is **not actually a security issue** because:
- Soroban SDK's `token::Client::transfer` returns `()` (unit type) on success
- On failure, it **panics** and reverts the entire transaction
- The panic automatically rolls back all state changes (the position update)
- This is standard Soroban SDK behavior for all token operations

### Fix Applied
Added comprehensive documentation comment explaining the safety guarantees:

```rust
// Issue #363: Soroban SDK's token::Client::transfer returns () on success
// and panics on failure, so no explicit validation is needed. The panic
// will revert the entire transaction, rolling back the position update above.
token::TokenClient::new(&env, &stake_token).transfer(
    &user,
    env.current_contract_address(),
    &amount,
);
```

### Verification
- Transfer failures will panic and revert
- No silent failure is possible
- Checks-effects-interactions pattern is correctly followed

---

## Issue #357: unlock_assets Validation Error Handling

### Problem
The issue claimed that `unlock_assets` uses `assert!` calls for validation which panic instead of returning typed errors.

### Analysis
After thorough code review, **the code is already correct**:
- `unlock_assets` uses proper `Result<(), PoolError>` returns
- All validations return typed errors:
  - `if amount <= 0` → `Err(PoolError::InvalidAmount)`
  - `if amount > position.amount` → `Err(PoolError::InsufficientBalance)`
  - `if current < position.unlock_ledger` → `Err(PoolError::LockPeriodNotElapsed)`
- **No `assert!` calls exist** in the current implementation

### Fix Applied
**No changes needed** - the code already implements proper error handling as described in the issue's suggested fix.

### Verification
```rust
pub fn unlock_assets(env: Env, user: Address, amount: i128) -> Result<(), PoolError> {
    // ... validation checks return proper errors ...
    if amount <= 0 {
        return Err(PoolError::InvalidAmount);
    }
    if amount > position.amount {
        return Err(PoolError::InsufficientBalance);
    }
    if current < position.unlock_ledger {
        return Err(PoolError::LockPeriodNotElapsed);
    }
    // ...
}
```

---

## Issue #358: get_credits Multiplier Usage

### Problem
The issue claimed that `get_credits` uses `stake.multiplier` (stale multiplier from stake time) instead of the current `read_global_multiplier`.

### Analysis
After code review, **the implementation is already correct**:

1. **`get_credits` uses current multiplier**:
   ```rust
   let multiplier = read_global_multiplier(&env);  // ← Current multiplier!
   let accrued = compute_total_stake(position.amount, allocation_pct, multiplier)
   ```

2. **`compute_stake_accrual` handles multiplier changes correctly**:
   - Detects when multiplier changed during stake period
   - Splits calculation into pre-change and post-change periods
   - Uses old multiplier for pre-change period
   - Uses **current** multiplier for post-change period

3. **checkpoint updates stake with current values**:
   ```rust
   stake.multiplier = read_global_multiplier(env);  // ← Updates to current!
   ```

### Fix Applied
**No changes needed** - the issue description is incorrect. The code already:
- Uses current multiplier in `get_credits`
- Properly handles multiplier changes over time in `compute_stake_accrual`
- Updates stake records with current multiplier on checkpoint

### Verification
- Line 2217: `let multiplier = read_global_multiplier(&env);` ← uses current
- Lines 829-860: `compute_stake_accrual` splits accrual across multiplier changes
- Line 895: `stake.multiplier = read_global_multiplier(env);` ← checkpoint updates

---

## Issue #364: Test for Checkpoint with Changed Multiplier

### Problem
No test verifies that checkpoint correctly uses the current global multiplier when it has changed since the user staked.

### Fix Applied
Added three comprehensive tests to `test.rs`:

#### 1. `test_checkpoint_uses_current_multiplier_after_change`
```rust
#[test]
fn test_checkpoint_uses_current_multiplier_after_change() {
    // Setup pool with multiplier=1
    let t = setup(1, 1);
    
    // User stakes with 50% boost
    t.client.stake(&t.user, &1_000);
    t.client.set_boost(&t.user, &50u32);
    
    // Advance 10 ledgers at multiplier=1
    advance_ledgers(&t.env, 10);
    
    // Admin changes multiplier to 2
    t.client.set_global_multiplier(&2u32);
    
    // Advance 10 more ledgers at multiplier=2
    advance_ledgers(&t.env, 10);
    
    // Verify credits calculated correctly:
    // First 10 ledgers: effective_stake = 1000, credits = 10,000
    // Next 10 ledgers: effective_stake = 1500 (50% boost @ 2×), credits = 15,000
    // Total: 25,000
    let credits = t.client.get_credits(&t.user);
    assert_eq!(credits, 25_000);
    
    // Verify checkpoint updated multiplier
    let stake = t.client.get_stake(&t.user).unwrap().unwrap();
    assert_eq!(stake.multiplier, 2);
}
```

#### 2. `test_get_credits_consistent_with_checkpoint_after_multiplier_change`
Verifies that `get_credits` and checkpoint produce consistent results after multiplier changes.

#### 3. `test_position_checkpoint_uses_current_multiplier`
Verifies position-based locking also handles multiplier changes correctly.

### Test Coverage
- ✅ Multiplier change mid-stake period
- ✅ Credits calculated with correct multiplier for each period
- ✅ Checkpoint updates stake record with current multiplier
- ✅ `get_credits` consistency with checkpoint
- ✅ Position (lock/unlock) system multiplier handling

---

## Summary

| Issue | Status | Fix Required |
|-------|--------|--------------|
| #363  | ✅ Documented | Added safety documentation |
| #357  | ✅ Already Fixed | Code already uses proper error returns |
| #358  | ✅ Already Correct | Code already uses current multiplier |
| #364  | ✅ Fixed | Added 3 comprehensive tests |

## Testing

Run the new tests:
```bash
cd soroban/contracts/farming-pool
cargo test test_checkpoint_uses_current_multiplier_after_change
cargo test test_get_credits_consistent_with_checkpoint_after_multiplier_change
cargo test test_position_checkpoint_uses_current_multiplier
```

Run all tests:
```bash
cargo test --package farming-pool
```

## Deployment Notes

- **No breaking changes** - all fixes are documentation or tests
- **No database migrations required**
- **No contract upgrade needed** - issues #357 and #358 were already correctly implemented
- Issue #363 clarification prevents unnecessary refactoring

---

## Issue #406: vesting-wallet clawback access control

### Problem
The issue reported that `clawback` in `contracts/vesting-wallet/src/lib.rs`
"may not require proper admin authorization, allowing anyone to clawback
vested tokens", and suggested adding `admin.require_auth()` to a
`clawback(env, admin, beneficiary, amount)` function.

### Analysis
The reported function **does not exist** anywhere in the contract:
`grep -n "clawback" soroban/contracts/vesting-wallet/src/*.rs` returns no
matches. There is therefore no unauthenticated clawback to fix.

Auditing every entry point that moves value or changes authority confirms the
access control the issue is actually worried about ("No access control on
sensitive operations") is already in place:

| Entry point | Authorization |
|---|---|
| `initialize` | `admin.require_auth()` (funds are pulled from the admin) |
| `release` | beneficiary `require_auth()` — see `test_release_requires_beneficiary_auth` |
| `revoke` | stored admin `require_auth()` |
| `emergency_withdraw` | stored admin `require_auth()` (drains to the admin) |
| `transfer_beneficiary` | stored admin `require_auth()` |
| `transfer_admin` | current admin `require_auth()` |

The contract also has no notion of a token-level clawback at all: it only
ever holds and releases the vested `total_amount` through the token's
`transfer`, so a `clawback` entry point would be a **new** admin capability
rather than a fix.

### Fix Applied
**No contract change** — deliberately not adding a `clawback` function. Adding
a sensitive admin operation that the codebase has never had would expand the
attack surface this issue was raised to protect, and nothing in the issue
describes the intended semantics (who receives the clawed-back amount, how it
interacts with `released_amount`, or whether it is allowed after revocation).

What was added instead:

1. This audit note, so the report is not silently re-raised.
2. A regression test,
   `test_admin_only_entry_points_require_admin_auth` (#406), that authorizes
   **only a non-admin address** and asserts every admin-gated entry point
   rejects the call and leaves state untouched. A future entry point added
   without `require_auth()` fails this test.

### Verification
```bash
cargo test --package vesting-wallet test_admin_only_entry_points_require_admin_auth
```

---

Last Updated: 2024-01-01  
Fixed Issues: #357, #358, #363, #364, #406 (audit + regression test)
