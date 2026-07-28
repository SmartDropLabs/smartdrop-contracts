# Farming Pool: keep_alive / TTL Recovery Implementation

## Steps

- [x] 1. Analyze codebase and create plan
- [x] 2. Fix `types.rs` - duplicate enum variants and conflicting error codes
- [x] 3. Fix `lib.rs` - compilation issues:
  - [x] 3a. Merge duplicate imports
  - [x] 3b. Add `SCHEMA_VERSION` constant
  - [x] 3c. Add `is_user_whitelisted` helper function
  - [x] 3d. Fix `initialize` function (dead code, missing MinStakeAmount, SchemaVersion)
  - [x] 3e. Fix `calculate_credits` (pos -> position, rate -> position.credit_rate)
  - [x] 3f. Fix `get_credits` (stake.credits_banked ownership)
  - [x] 3g. Fix `set_boost` (duplicate guards)
  - [x] 3h. Fix `admin` and `emergency_withdraw` (unwrap -> ?)
  - [x] 3i. Fix `set_global_multiplier` (wrong error variant)
  - [x] 3j. Fix `transfer_admin` return type
- [x] 4. Add `keep_alive` function to `lib.rs`
- [x] 5. Add tests for `keep_alive` in `test.rs`
- [ ] 6. Run `cargo build` / `cargo test` to verify everything compiles and passes

