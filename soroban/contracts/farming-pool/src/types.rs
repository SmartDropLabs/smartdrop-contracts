use soroban_sdk::{contracterror, contracttype, Address};

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
    NoActiveStake = 8,
    Paused = 9,
    /// `amount` was <= 0, or exceeded the caller's withdrawable balance.
    /// Returned by `unstake` (see #77).
    InvalidAmount = 10,
    NoActiveStake = 9,
    Paused = 10,
    /// Returned by `accept_admin` when no admin handoff is pending.
    NoPendingAdmin = 11,
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
    MinStakeAmount,
    TotalStaked,
}
