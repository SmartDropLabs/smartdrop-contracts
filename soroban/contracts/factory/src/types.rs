use soroban_sdk::{contracterror, contracttype, Address, BytesN, String, Vec};

/// Health status returned by `check_pool_health` (issue #653).
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthStatus {
    /// Pool is operating normally.
    Healthy,
    /// Pool's TVL could not be read; it may be running an older WASM.
    Degraded,
    /// Pool is deprecated and no longer accepting new stakes.
    Deprecated,
}

/// Advisory metadata attached to a pool at creation time.
/// Fields here are stored on-chain for display purposes; they are not enforced
/// by the pool's own `initialize` and do not affect reward mechanics.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolMetadata {
    /// Human-readable name for the pool. Must be non-empty.
    pub name: String,
}

/// Parameters for creating one farming pool through `create_pools_batch`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolParams {
    pub asset: Address,
    pub daily_rate: u128,
    pub global_multiplier: u32,
    pub min_lock_period: u64,
    pub min_stake_amount: i128,
    /// Advisory metadata for the pool. The name must be non-empty.
    pub metadata: PoolMetadata,
}

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
    /// Flag indicating if pool creation is currently paused.
    PoolCreationPaused,
    /// Pool IDs matching a specific asset address.
    AssetPools(Address),
    /// Number of pools registered for a specific asset address.
    AssetPoolCount(Address),
    /// Running count of admin transfers performed.
    AdminTransferCount,
    /// Running total of successful `upgrade_pool` calls, for version tracking (#258).
    UpgradeCount,
    /// List of pool IDs created by a specific admin.
    PoolsByAdmin(Address),
    /// List of pool IDs currently running a specific WASM hash.
    PoolsByWasmHash(BytesN<32>),
    /// Hashes explicitly approved by the factory admin for pool upgrades.
    ApprovedWasmHash(BytesN<32>),
    /// Aggregate value locked across every pool, maintained incrementally by
    /// `sync_pool_tvl` so `total_tvl` is an O(1) read (#249).
    TotalTvl,
    /// Last-synced TVL for a single pool, keyed by pool ID. This is the term
    /// currently folded into `TotalTvl` for that pool (#249).
    PoolTvl(u32),
    /// Insurance fund balance for a specific pool.
    InsuranceFund(u32),
    /// One recorded ballot per authenticated address and pool.
    PoolVote(u32, Address),
    /// Aggregate approval votes for a pool.
    PoolYesVotes(u32),
    /// Aggregate rejection votes for a pool.
    PoolNoVotes(u32),
    /// Global counter for pool configuration change events (#680).
    PoolConfigHistoryCount(u32),
    /// Pool configuration change event by index (#680).
    PoolConfigHistory(u32, u32),
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
    /// The originally requested daily rate, preserved here before ledger conversion.
    pub daily_rate: u128,
    /// The WASM hash used to deploy or upgrade the pool.
    pub wasm_hash: soroban_sdk::BytesN<32>,
    /// Whether this pool has been deprecated by the admin. Deprecated pools
    /// are still readable but callers should treat them as decommissioned.
    /// See #602.
    pub deprecated: bool,
    /// Monotonically-increasing version counter, incremented on each
    /// `upgrade_pool` call so callers can detect upgrades (#638).
    pub version: u32,
}

/// Side-by-side view of a pool returned by `compare_pools` (#617).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolComparison {
    pub pool_id: u32,
    pub address: Address,
    pub asset: Address,
    pub daily_rate: u128,
    pub credit_rate: i128,
    pub global_multiplier: u32,
    pub min_lock_period: u32,
    pub tvl: i128,
    pub risk_score: u32,
    pub deprecated: bool,
}

/// Sort keys supported by `list_pools_sorted`.
#[contracttype]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PoolSort {
    /// Sort by pool ID in ascending order.
    PoolId,
    /// Sort by per-ledger credit rate in ascending order.
    CreditRate,
    /// Sort by global multiplier in ascending order.
    GlobalMultiplier,
    /// Sort by minimum lock period in ascending order.
    MinLockPeriod,
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
    /// Whether there are more records available beyond this page.
    pub has_more: bool,
}

/// Full pool dependency graph, returned by `get_dependency_graph` (#668).
///
/// `edges` holds one entry per pool that has explicit dependencies set via
/// `set_pool_dependencies`, each pairing that pool's ID with the IDs it
/// depends on. Pools with no dependencies configured are omitted rather than
/// listed with an empty dependency list, so the graph stays proportional to
/// how much of the registry actually declares dependencies.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DependencyGraph {
    pub edges: Vec<(u32, Vec<u32>)>,
}

/// Result of a `refresh_pool_ttls` sweep (#393).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RefreshPoolTtlsResponse {
    /// Pool IDs in the scanned range that had a record and were refreshed.
    pub refreshed: Vec<u32>,
    /// The scanned range's end (exclusive); pass as the next `start_id`.
    pub end_id: u32,
    /// Count of pool IDs in the scanned range with no record (gaps).
    pub missing: u32,
}

/// Pool health status returned by `pool_status` (Issue #375).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolStatus {
    /// Address of the pool contract.
    pub address: Address,
    /// Whether staking is currently paused in the pool.
    pub is_paused: bool,
    /// Current total amount staked in the pool.
    pub total_staked: i128,
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
    /// `create_pool`'s `daily_rate` converts to a `credit_rate` of zero (or doesn't fit `i128`).
    ///
    /// `FarmingPool::initialize` requires `credit_rate > 0`; a `daily_rate` below
    /// `LEDGERS_PER_DAY` truncates to zero under the daily-to-per-ledger conversion
    /// and is rejected here rather than silently deploying a pool that can never
    /// initialize.
    InvalidCreditRate = 5,
    /// `create_pool`'s `min_lock_period` does not fit in the pool's native `u32`.
    MinLockPeriodOutOfRange = 6,
    /// `create_pool`'s `global_multiplier` was < 1 (mirrors `FarmingPool::initialize`'s own check).
    InvalidGlobalMultiplier = 7,
    /// `create_pool` cannot allocate another monotonically increasing pool ID.
    PoolCountOverflow = 8,
    /// `upgrade_pool` was called on a pool whose own stored admin no longer
    /// matches the factory's current admin (the two diverge once either side
    /// calls its own `transfer_admin` independently — see `upgrade_pool`'s docs).
    PoolAdminMismatch = 9,
    /// `upgrade_pool` failed because the target pool does not support upgrades
    /// (e.g. older deployment without upgrade/admin entry points) or the upgrade call failed.
    PoolUpgradeFailed = 10,
    /// `create_pool`'s asset does not respond as a valid token contract.
    InvalidAsset = 11,
    /// `create_pool`'s minimum stake is below the protocol dust threshold.
    InvalidMinStakeAmount = 12,
    /// `create_pool` was called while pool creation is paused.
    PoolCreationPaused = 13,
    /// `set_pool_wasm_hash` or `initialize` was called with an all-zero WASM hash.
    InvalidWasmHash = 14,
    /// `create_pool`'s minimum lock period is below the minimum allowed threshold.
    MinLockPeriodTooShort = 15,
    /// `initialize` was called with a zero-address admin, or `transfer_admin`
    /// was called with a zero-address `new_admin`, which would permanently
    /// lock the factory.
    InvalidAdmin = 16,
    /// A pool's TVL could not be read during `total_tvl` maintenance because the
    /// deployed pool did not answer the `total_staked` getter (e.g. a pool
    /// deployed from an older WASM that predates it).
    PoolQueryFailed = 17,
    SameWasmHash = 18,
    /// `create_pool`'s minimum lock period exceeds the maximum allowed
    /// threshold, which would trap staked funds for an effectively permanent
    /// window.
    MinLockPeriodAboveCeiling = 19,
    /// `create_pools_batch` was called with an empty `pools` vector, which
    /// would consume admin authorization and bump instance TTL while creating
    /// nothing.
    EmptyBatch = 20,
    /// `create_pools_batch` was called with more entries than `MAX_BATCH_SIZE`.
    BatchTooLarge = 21,
    /// `deprecate_pool` was called on a pool that is already deprecated.
    /// See #602.
    AlreadyDeprecated = 22,
    /// Insufficient funds in the insurance fund for the requested claim.
    InsufficientFunds = 23,
    /// `compare_pools` was called with an empty ID list.
    EmptyPoolList = 24,
    /// `compare_pools` was called with more than `MAX_COMPARE_POOLS` IDs.
    TooManyPools = 25,
    /// `compare_pools` was called with the same pool ID more than once.
    DuplicatePoolId = 26,
    /// A voter has already voted on a pool.
    AlreadyVoted = 27,
    /// `create_pool`'s metadata is invalid (e.g. empty name).
    InvalidMetadata = 28,
}

/// A single pool configuration change event (#680).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolConfigChangeEntry {
    pub pool_id: u32,
    pub action: soroban_sdk::Symbol,
    pub old_config: Option<PoolRecord>,
    pub new_config: Option<PoolRecord>,
    pub ledger: u32,
}

/// Paginated response for pool configuration history queries (#680).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolConfigHistoryPage {
    pub events: Vec<PoolConfigChangeEntry>,
    pub total: u32,
}
