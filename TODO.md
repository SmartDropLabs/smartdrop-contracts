# TODO - factory/farming-pool test integration

- [ ] Add a new factory unit test that deploys the _real_ farming-pool WASM (instead of MOCK_POOL_WASM) and uses `FarmingPoolClient` to assert the deployed pool is initialized / not initialized as appropriate.
- [ ] Keep existing MOCK_POOL_WASM tests unchanged.
- [ ] Run `cargo test -p factory --tests` to confirm no regressions.
- [ ] Update `factory/Cargo.toml` only if additional test-only dependencies are required.
