use soroban_sdk::{contracterror, contracttype, Address, Vec};

/// Error codes returned by the farming pool contract.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PoolError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidCreditRate = 3,
    /// `global_multiplier` was 0 or exceeded `MAX_GLOBAL_MULTIPLIER`. See #89.
    InvalidGlobalMultiplier = 4,
    NotWhitelisted = 5,
    BelowMinimumStake = 6,
    InvalidMinStakeAmount = 7,
    /// Returned by `emergency_withdraw` when the pool is not currently paused.
    NotPaused = 8,
    /// Returned by `emergency_withdraw` when the user has no stake or locked position.
    NoActiveStake = 9,
    Paused = 10,
    /// Returned by `accept_admin` when no admin handoff is pending.
    NoPendingAdmin = 11,
    /// Returned by `batch_add_to_whitelist` or `batch_remove_from_whitelist` when batch exceeds 50 users.
    BatchTooLarge = 12,
    /// Returned when a supplied amount is zero or negative.
    InvalidAmount = 13,
    /// Returned when the user has no active locked position.
    NoActivePosition = 14,
    /// Returned when withdrawing more than the locked/staked balance.
    InsufficientBalance = 15,
    /// Returned when the minimum lock period has not elapsed yet.
    LockPeriodNotElapsed = 16,
    /// Returned when `set_boost` receives an `allocation_pct` outside 1-100.
    InvalidAllocation = 17,
    ExceedsMaxStake = 18,
    /// Returned when `set_min_lock_period` exceeds `MAX_LOCK_PERIOD`.
    InvalidLockPeriod = 19,
    /// Returned by `stake` when the configured stake token does not answer a
    /// `balance` query — the address is not a deployed contract, or it has no
    /// token interface. See #459.
    InvalidStakeToken = 20,
    /// Returned by `stake` when the stake token did not deliver the full
    /// deposit into the pool (fee-on-transfer / under-delivering token), so
    /// crediting `amount` would leave the pool short of its own liabilities.
    /// See #458.
    TransferAmountMismatch = 21,
}

/// Per-user boost configuration returned by `get_boost_config`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostConfig {
    pub multiplier: u32,
    pub allocation_pct: u32,
}

/// Recorded state for a user's stake position in the boost / continuous staking system.
///
/// Unlike `Position` (which enforces a strict minimum lock period), `UserStake` supports
/// flexible, non-locked staking with optional user-allocated boost multipliers (`BoostConfig`).
/// Accrual is calculated using `compute_total_stake(amount, allocation_pct, multiplier) * credit_rate * elapsed`.
/// Users interact via `stake`, `unstake`, `set_boost`, `get_boost_config`, and `get_credits`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct UserStake {
    pub amount: i128,
    pub start_ledger: u32,
    pub credits_banked: i128,
    /// Credit rate snapshot used for accrual since `start_ledger`.
    pub credit_rate: i128,
    /// Global multiplier snapshot captured at checkpoint time (#60).
    pub multiplier: u32,
}

/// Recorded state for a user's locking position in the time-locked staking system.
///
/// `Position` enforces a mandatory minimum lock duration (`min_lock_period`), preventing
/// withdrawals via `unlock_assets` until `unlock_ledger` is reached. Adding to an existing
/// position extends `unlock_ledger` to the later of its current value and a fresh lock period
/// from the top-up ledger. Accrual is calculated using `amount * credit_rate * elapsed`.
/// Emits structured `locked` and `unlocked` events.
/// Users interact via `lock_assets`, `unlock_assets`, `calculate_credits`, and `get_user_position`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Position {
    pub amount: i128,
    pub lock_ledger: u32,
    /// Earliest ledger at which the whole position may be unlocked. Top-ups
    /// extend this to at least a fresh minimum lock period from the top-up.
    pub unlock_ledger: u32,
    pub checkpoint_ledger: u32,
    pub total_credits: i128,
    /// Credit rate snapshot used for accrual since `checkpoint_ledger`.
    pub credit_rate: i128,
}

/// Credits banked for a user by `emergency_withdraw`, kept as two distinct
/// totals rather than a single merged sum so that the individual accrual
/// history of the lock/unlock (`position`) and boost (`stake`) systems is not
/// lost when a user has both. See #145.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BankedCreditTotals {
    pub position_credits: i128,
    pub stake_credits: i128,
}

/// Storage keys for all persistent and instance data.
#[contracttype]
pub enum DataKey {
    Admin,
    PendingAdmin,
    GlobalMultiplier,
    CreditRate,
    StakeToken,
    MinLockPeriod,
    SchemaVersion,
    Paused,
    PausedStaking,
    PausedWithdrawals,
    GlobalMultiplierChangeLedger,
    UserBoost(Address),
    UserStake(Address),
    UserPosition(Address),
    BankedCredits(Address),
    // Whitelist keys
    WhitelistEnabled,
    Whitelisted(Address),
    /// Ordered list of all currently whitelisted addresses (instance storage).
    WhitelistedUsers,
    MinStakeAmount,
    TotalStaked,
    /// Cumulative credits committed to users since pool initialization.
    TotalDistributedCredits,
    StakedUserCount,
    /// Running count of total lock operations performed.
    LockCount,
    /// Running count of total unstake operations performed.
    UnstakeCount,
    TotalCredits,
    EmergencyWithdrawalCount,
    /// Running count of active stakes.
    ActiveStakeCount,
    /// Running count of credit rate updates performed.
    CreditRateChangeCount,
    /// Running count of `set_boost` calls performed (#230).
    BoostCount,
    /// Total tokens currently locked in time-locked `Position`s (#232).
    TotalLocked,
    /// Running total of all tokens deposited into the pool via `stake` and `lock_assets`.
    TotalDeposits,
    /// Running total of all tokens withdrawn from the pool via `unstake`,
    /// `unlock_assets`, and `emergency_withdraw`.
    TotalWithdrawals,
    /// Sum of every user's current boost `allocation_pct`, tracked incrementally
    /// as `set_boost` calls change each user's allocation.
    TotalBoostAlloc,
    /// Count of users with a non-zero boost allocation currently set.
    BoostUserCount,
    /// Aggregate of credits currently banked across all users.
    TotalBankedCredits,
    /// Cumulative credits earned by a user across their entire lifetime.
    TotalCreditsEarned(Address),
    /// Ordered list of all users with active stakes (persistent storage).
    StakedUsers,
    /// Ordered list of all users with active locked positions (persistent storage).
    LockedUsers,
    // ── History storage keys ────────────────────────────────────────────────
    /// Global counter for whitelist events.
    WhitelistEventCount,
    /// Whitelist event by index (u32 -> WhitelistEvent).
    WhitelistHistory(u32),
    /// Global counter for boost events.
    BoostEventCount,
    /// Boost event by index (u32 -> BoostEvent).
    BoostHistory(u32),
    /// Global counter for stake/unstake/lock/unlock events.
    StakeEventCount,
    /// Stake event by index (u32 -> StakeEvent).
    StakeHistory(u32),
    /// Global counter for admin action events.
    AdminActionCount,
    /// Admin action event by index (u32 -> AdminActionEvent).
    AdminActionHistory(u32),
    /// Global counter for credit rate change events (#310).
    CreditRateEventCount,
    /// Credit rate change event by index (#310).
    CreditRateHistory(u32),
    /// Global counter for global multiplier change events (#311).
    GlobalMultiplierEventCount,
    /// Global multiplier change event by index (#311).
    GlobalMultiplierHistory(u32),
    // ── Feature 1: User Activity Tracking ────────────────────────────────────
    /// Ledger sequence of user's last interaction with the pool.
    UserLastActivity(Address),
    /// Total activity count per user (stake, unstake, lock, unlock operations).
    UserActivityCount(Address),
    // ── Feature 2: Performance Tiers ─────────────────────────────────────────
    /// User's current tier level (0=bronze, 1=silver, 2=gold, 3=platinum).
    UserTier(Address),
    /// Total earned tier points for the user.
    UserTierPoints(Address),
    /// Counter for users currently at each tier level.
    TierUserCount(u32),
    // ── Feature 3: Bulk Reward Distribution ──────────────────────────────────
    /// Distribution event ID counter.
    DistributionEventCount,
    /// Store distribution event by index.
    DistributionHistory(u32),
    /// Total rewards distributed through bulk operations.
    TotalDistributedRewards,
    // ── Feature 4: Extended Pool Analytics ───────────────────────────────────
    /// Average lock duration across all users (cached, updated on checkpoints).
    AverageLockDuration,
    /// Peak total staked amount recorded.
    PeakTotalStaked,
    /// Current number of unique users with any stake or position.
    UniqueUserCount,
}

/// Paginated response for `get_whitelisted_users`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ListWhitelistedResponse {
    /// Whitelisted addresses in the requested page.
    pub users: Vec<Address>,
    /// Total number of whitelisted addresses.
    pub total: u32,
}

/// Aggregate pool parameters returned by `get_pool_info` (Issue #395).
///
/// A pool overview previously needed several separate contract calls
/// (`total_staked`, `credit_rate`, the global multiplier, the min lock
/// period, the min stake amount and the paused flag), each paying its own
/// invocation and TTL bump. This bundles the read-only pool configuration
/// into one call for dashboards and analytics.
///
/// The issue text also names "total credits" and "number of stakers"; those
/// are the maintained `total_distributed_credits` / `total_banked_credits`
/// counters, included here so a full overview is a single round trip. There
/// is no per-staker count kept in storage — staker counts are derived by
/// paging `get_positions`, so no field for it is invented here.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolInfo {
    /// Total value staked across all positions, in stroops.
    pub total_staked: i128,
    /// Credits earned banked by users but not yet withdrawn, in stroops.
    pub total_banked_credits: i128,
    /// Credits distributed to all users since initialization, in stroops.
    pub total_distributed_credits: i128,
    /// Current global credit rate.
    pub credit_rate: i128,
    /// Current global reward multiplier.
    pub global_multiplier: u32,
    /// Minimum lock period in ledgers.
    pub min_lock_period: u32,
    /// Minimum amount accepted by `stake`, in stroops.
    pub min_stake_amount: i128,
    /// Whether the pool is currently paused.
    pub is_paused: bool,
}

/// Consolidated user account overview returned by `get_user_summary` (Feature 8 improvement, Issue #561).
///
/// Bundles a user's flexible stake amount, locked position amount, cumulative accrued credits,
/// active boost allocation percentage, whitelist authorization status, and earliest position
/// unlock ledger into a single read-only query.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSummary {
    /// Amount of tokens currently staked in flexible staking.
    pub staked_amount: i128,
    /// Amount of tokens currently locked in time-locked position.
    pub locked_amount: i128,
    /// Total accrued credits across flexible, locked, and banked credits.
    pub total_credits: i128,
    /// Current boost allocation percentage (0-100).
    pub boost_allocation: u32,
    /// Whether user is whitelisted (or if whitelist is disabled).
    pub is_whitelisted: bool,
    /// Ledger at which the locked position unlocks (or 0 if none).
    pub unlock_ledger: u32,
}

// ─── History / audit trail types ─────────────────────────────────────────────

/// A single whitelist change event (add or remove).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WhitelistEvent {
    pub user: Address,
    pub added: bool,
    pub ledger: u32,
}

/// A single boost allocation change event.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostEvent {
    pub user: Address,
    pub old_allocation: u32,
    pub new_allocation: u32,
    pub ledger: u32,
}

/// A single stake/unstake/lock/unlock transaction event.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakeEvent {
    pub user: Address,
    /// "stake", "unstake", "lock", "unlock", or "emergency"
    pub action: soroban_sdk::Symbol,
    pub amount: i128,
    pub ledger: u32,
}

/// A single admin action event (parameter change, upgrade, pause, etc.).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminActionEvent {
    /// What was changed: "credit_rate", "multiplier", "pause", "unpause",
    /// "min_lock", "min_stake", "upgrade", "admin_transfer", "migrate",
    /// "whitelist_enable", "whitelist_disable", "recover_credits"
    pub action: soroban_sdk::Symbol,
    pub admin: Address,
    pub ledger: u32,
}

/// Paginated response for whitelist history queries.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct WhitelistHistoryPage {
    pub events: Vec<WhitelistEvent>,
    pub total: u32,
}

/// Paginated response for boost history queries.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostHistoryPage {
    pub events: Vec<BoostEvent>,
    pub total: u32,
}

/// Paginated response for stake history queries.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct StakeHistoryPage {
    pub events: Vec<StakeEvent>,
    pub total: u32,
}

/// Paginated response for admin action history queries.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct AdminActionHistoryPage {
    pub events: Vec<AdminActionEvent>,
    pub total: u32,
}

/// A single credit rate change event (#310).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CreditRateEvent {
    pub old_rate: i128,
    pub new_rate: i128,
    pub ledger: u32,
}

/// Paginated response for credit rate history queries (#310).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CreditRateHistoryPage {
    pub events: Vec<CreditRateEvent>,
    pub total: u32,
}

/// A single global multiplier change event (#311).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalMultiplierEvent {
    pub old_multiplier: u32,
    pub new_multiplier: u32,
    pub ledger: u32,
}

/// Paginated response for global multiplier history queries (#311).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalMultiplierHistoryPage {
    pub events: Vec<GlobalMultiplierEvent>,
    pub total: u32,
}

// ── Feature 1: User Activity Tracking (Issue #554) ─────────────────────────

/// User activity summary returned by `get_user_activity`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserActivity {
    /// Ledger sequence of user's last interaction.
    pub last_activity_ledger: u32,
    /// Total number of staking/locking operations by user.
    pub activity_count: u32,
    /// Ledger range since first activity.
    pub activity_span: u32,
}

// ── Feature 2: Performance Tiers (Issue #555) ────────────────────────────────

/// Performance tier level and rewards configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierInfo {
    /// Tier level: 0=bronze, 1=silver, 2=gold, 3=platinum.
    pub tier_level: u32,
    /// Total points earned by user toward current tier.
    pub tier_points: i128,
    /// Reward multiplier for this tier (e.g., 1000 = 1.0x, 1500 = 1.5x).
    pub reward_multiplier: u32,
}

// ── Feature 3: Bulk Reward Distribution (Issue #556) ───────────────────────

/// Distribution event recorded for bulk operations.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DistributionEvent {
    /// Number of users included in distribution.
    pub user_count: u32,
    /// Total amount distributed.
    pub total_amount: i128,
    /// Admin who initiated the distribution.
    pub admin: Address,
    /// Ledger at which distribution occurred.
    pub ledger: u32,
}

/// Paginated response for distribution history queries (Issue #566).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct DistributionHistoryPage {
    pub events: Vec<DistributionEvent>,
    pub total: u32,
}

// ── Feature 4: Extended Pool Analytics (Issue #557) ──────────────────────────

/// Extended analytics snapshot for pool health monitoring.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolAnalytics {
    /// Average lock duration in ledgers across all locked positions.
    pub average_lock_duration: u32,
    /// Peak total staked amount ever recorded.
    pub peak_total_staked: i128,
    /// Current total unique users (with stake or position).
    pub unique_user_count: u32,
    /// Pool TVL turnover ratio (total_deposits / total_staked).
    pub tvl_ratio: u64,
}

/// Leaderboard entry for top stakers returned by `get_leaderboard`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderboardEntry {
    /// The staker's address.
    pub staker: Address,
    /// Total stake balance held by this staker.
    pub stake_balance: i128,
    /// Rank position in the leaderboard.
    pub rank: u32,
}
