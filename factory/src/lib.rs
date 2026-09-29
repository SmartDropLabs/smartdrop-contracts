<!-- Other code remains unchanged -->

impl Factory {
    // Other methods remain unchanged

    pub fn set_pool_wasm_hash(&mut self, new_hash: Hash) {
        // Validate that the new hash is different from the current one
        assert!(new_hash != self.pool_wasm_hash, "New pool WASM hash must be different from the current one");

        // Proceed with setting the new hash
        self.pool_wasm_hash = new_hash;
        self.emit_event(Event::PoolWasmHashChanged(new_hash));
    }

    // Other methods remain unchanged
}

// Unit test for set_pool_wasm_hash
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_pool_wasm_hash_with_same_hash() {
        let mut factory = Factory::default();
        let initial_hash = Hash::default();

        // Set the initial pool WASM hash
        factory.set_pool_wasm_hash(initial_hash);

        // Attempt to set the same hash again
        assert_eq!((), factory.set_pool_wasm_hash(initial_hash));

        // Validate that the pool WASM hash has not changed
        assert_eq!(initial_hash, factory.pool_wasm_hash);
    }

    #[test]
    fn test_set_pool_wasm_hash_with_different_hash() {
        let mut factory = Factory::default();
        let initial_hash = Hash::default();
        let new_hash = Hash::default();

        // Set the initial pool WASM hash
        factory.set_pool_wasm_hash(initial_hash);

        // Set a different hash
        assert_eq!((), factory.set_pool_wasm_hash(new_hash));

        // Validate that the pool WASM hash has changed
        assert_ne!(initial_hash, factory.pool_wasm_hash);
        assert_eq!(new_hash, factory.pool_wasm_hash);
    }
}

<!-- Other code remains unchanged -->