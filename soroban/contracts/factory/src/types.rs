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
///
/// Every field here is a direct mirror of the value passed to the deployed
/// pool's `initialize` call — not advisory metadata. Callers can trust that
/// `credit_rate`/`global_multiplier`/`min_lock_period` match what
/// `FarmingPoolClient::credit_rate()`/`min_lock_period()` etc. return on the
/// pool itself at creation time (see `test_create_pool_configures_deployed_pool_matching_factory_record`).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolRecord {
    /// Address of the deployed farming-pool contract instance.
    pub address: Address,
    /// The staking asset for this pool.
    pub asset: Address,
    /// Per-ledger credit accrual rate, as passed to the pool's `initialize`.
    pub credit_rate: i128,
    /// Boost multiplier applied to allocated stake, as passed to `initialize`.
    pub global_multiplier: u32,
    /// Minimum number of ledgers a stake must be held before withdrawal.
    pub min_lock_period: u32,
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
    /// `create_pool`'s `global_multiplier` was < 1 (mirrors `FarmingPool::initialize`'s own check).
    InvalidGlobalMultiplier = 4,
    /// `create_pool`'s `daily_rate` converts to a `credit_rate` of zero (or doesn't fit `i128`).
    ///
    /// `FarmingPool::initialize` requires `credit_rate > 0`; a `daily_rate` below
    /// `LEDGERS_PER_DAY` truncates to zero under the daily-to-per-ledger conversion
    /// and is rejected here rather than silently deploying a pool that can never
    /// initialize.
    InvalidCreditRate = 5,
    /// `create_pool`'s `min_lock_period` does not fit in the pool's native `u32`.
    MinLockPeriodOutOfRange = 6,
    /// A function requiring initialization was called on an uninitialized factory.
    NotInitialized = 7,
    /// `create_pool`'s PoolCount increment would overflow `u32::MAX`.
    ///
    /// Returned by `create_pool` when the running pool counter is already at
    /// `u32::MAX` and a further increment would wrap around to 0, which would
    /// silently clobber pool 0's registry record.  Provides an explicit, typed
    /// failure instead of relying solely on the workspace Cargo profile's
    /// `overflow-checks` flag.
    PoolCountOverflow = 8,
}
