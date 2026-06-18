use soroban_sdk::{contracttype, Address};

/// Storage keys used by the factory contract.
#[contracttype]
pub enum DataKey {
    /// Address of the factory admin — set once during initialize.
    Admin,
    /// Running count of pools created; doubles as the next pool ID.
    PoolCount,
    /// SHA-256 hash of the uploaded farming-pool WASM used for all pool deployments.
    WasmHash,
    /// Per-pool record keyed by monotonically assigned pool ID.
    Pool(u32),
}

/// On-chain record for a registered farming pool.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolRecord {
    /// Address of the deployed farming-pool contract instance.
    pub address: Address,
    /// The staking asset for this pool.
    pub asset: Address,
    /// Per-ledger credit rate set for this pool at creation time.
    pub daily_rate: u128,
    /// Minimum number of ledgers a stake must be held before withdrawal.
    pub min_lock_period: u64,
}
