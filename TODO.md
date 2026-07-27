# Farming Pool: Typed Validation Errors (#66)

## Progress

- [x] Step 1: Add PoolError variants to types.rs
- [x] Step 2: Replace assert! with typed errors in lib.rs
  - [x] lock_assets: amount > 0 → InvalidAmount
  - [x] unlock_assets: amount > 0 → InvalidAmount
  - [x] unlock_assets: amount <= position.amount → InsufficientBalance
  - [x] unlock_assets: current >= position.unlock_ledger → LockPeriodNotElapsed
  - [x] unlock_assets: .expect("no active position") → NoActiveStake (fixes #65 adjacency)
  - [x] stake: amount > 0 → InvalidAmount
  - [x] set_boost: allocation_pct 1-100 → InvalidAllocation
- [x] Step 3: Update tests to assert specific PoolError variants
- [x] Step 4: Run tests to verify

