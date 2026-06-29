# Implementation Notes — Issue #23

**Issue:** Add upgrade entrypoint to farming-pool for WASM hot-swap
**Upstream:** https://github.com/SmartDropLabs/smartdrop-contracts/issues/23

## Acceptance Criteria

## Overview

Soroban contracts can upgrade their own WASM at runtime via `env.deployer().update_current_contract_wasm(new_hash)`. The farming-pool contract has no such entrypoint, making bug fixes require deploying an entirely new pool — all staker positions, credits, and lock states are lost.

## Design

Add an admin-only `upgrade` function that replaces the contract WASM in place:

```rust
/// Admin: replace this contract's WASM with the implementation at `new_wasm_hash`.
///
/// Storage (positions, stakes, credits) is preserved across upgrades.
/// The new WASM must be installed on-chain before calling this function.
/// Emits a `("pool", "upgraded")` event with the new hash.
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), PoolError> {
    get_admin(&env).require_auth();
    bump_instance(&env);
    env.deployer().update_current_contract_wasm(new_wasm_hash.clone());
    env.events().publish(
        (symbol_short!("pool"), symbol_short!("upgraded")),
        new_wasm_hash,
    );
    Ok(())
}
```

Also add a corresponding `upgrade_pool(pool_id, new_wasm_hash)` to the factory that:
1. Verifies the caller is the factory admin
2. Calls the pool's `upgrade` function via cross-contract invocation
3. Does NOT update the factory's stored `WasmHash` (pool-by-pool upgrade, not factory-wide)

## Migration Safety

The upgrade process requires that the new WASM's storage schema is backwards-compatible with existing `DataKey` entries. Add a `SCHEMA_VERSION` constant a

---
_Delete this file before merging._