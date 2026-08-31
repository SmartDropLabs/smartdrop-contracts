use soroban_sdk::{contracterror, contracttype, Address, BytesN, Vec};

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
    /// Running count of admin transfers performed.
    AdminTransferCount,
    /// Running total of successful `upgrade_pool` calls, for version tracking (#258).
    UpgradeCount,
    /// List of pool IDs for a specific asset.
    AssetPools(Address),
    /// List of pool IDs created by a specific admin.
    PoolsByAdmin(Address),
    /// List of pool IDs currently running a specific WASM hash.
    PoolsByWasmHash(BytesN<32>),
    /// Aggregate value locked across every pool, maintained incrementally by
    /// `sync_pool_tvl` so `total_tvl` is an O(1) read (#249).
    TotalTvl,
    /// Last-synced TVL for a single pool, keyed by pool ID. This is the term
    /// currently folded into `TotalTvl` for that pool (#249).
    PoolTvl(u32),
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
    /// `initialize` was called with an invalid admin address.
    InvalidAdmin = 16,
    /// A pool's TVL could not be read during `total_tvl` maintenance because the
    /// deployed pool did not answer the `total_staked` getter (e.g. a pool
    /// deployed from an older WASM that predates it).
    PoolQueryFailed = 17,
    /// `create_pool`'s `min_lock_period` exceeded `MAX_MIN_LOCK_PERIOD`. See #132.
    MinLockPeriodAboveCeiling = 18,
}
