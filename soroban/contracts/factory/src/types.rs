use soroban_sdk::{contracterror, contracttype, Address, Vec};

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

/// Paginated pool registry response.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ListPoolsResponse {
    /// Pool IDs and records in ascending pool ID order.
    pub records: Vec<(u32, PoolRecord)>,
    /// Start ID to use for the next page, or `total` when exhausted.
    pub next_start_id: u32,
    /// Total number of pools registered in the factory.
    pub total: u32,
}

/// Typed errors returned by the factory contract.
///
/// Using `#[contracterror]` exposes these as a stable on-chain error code so
/// clients and indexers can match on the specific failure rather than parsing
/// a panic message string.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// `initialize` was called on an already-initialised factory.
    AlreadyInitialized = 1,
    /// `get_pool` was called with a pool ID that has not been created yet.
    PoolNotFound = 2,
    /// `transfer_admin` was called by an address that is not the current admin.
    Unauthorized = 3,
    /// A public function was called before `initialize` seeded the factory state.
    ///
    /// Returned instead of panicking on an absent `Admin`/`WasmHash` entry, and
    /// also by the read-only getters that would otherwise report a misleading
    /// empty registry for a factory that does not exist yet.
    NotInitialized = 4,
}
