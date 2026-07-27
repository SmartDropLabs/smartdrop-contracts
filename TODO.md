# Issue #62: farming-pool overflow fix

## Steps

- [x] Step 1: Read all source files and gather information
- [x] Step 2: Create and confirm plan with user
- [ ] Step 3: Edit `types.rs` - Add `CreditOverflow = 15` to `PoolError`
- [ ] Step 4: Edit `lib.rs` - Convert `compute_total_stake` to return `Result<i128, PoolError>`
- [ ] Step 5: Edit `lib.rs` - Convert `compute_credits` to return `Result<i128, PoolError>`
- [ ] Step 6: Edit `lib.rs` - Convert `checkpoint` to return `Result<(), PoolError>`
- [ ] Step 7: Edit `lib.rs` - Convert `checkpoint_position` to return `Result<(), PoolError>`
- [ ] Step 8: Edit `lib.rs` - Propagate `?` through `stake`, `set_boost`, `lock_assets`
- [ ] Step 9: Edit `lib.rs` - Graceful degradation in `unstake` and `unlock_assets`
- [ ] Step 10: Edit `lib.rs` - Fix `calculate_credits` and `get_credits` arithmetic
- [ ] Step 11: Edit `test.rs` - Update existing tests for new Result returns
- [ ] Step 12: Edit `test.rs` - Add overflow tests
- [ ] Step 13: Run `cargo test -p farming-pool` to verify

