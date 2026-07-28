use soroban_sdk::{contracterror, contracttype, Address};

/// Error codes returned by the farming pool contract.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PoolError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    /// `credit_rate` was ≤ 0 or exceeded `MAX_CREDIT_RATE`. See #89.
    InvalidCreditRate = 3,
    /// Lock/stake amount is below the configured minimum.
    BelowMinimumStake = 4,
    /// Returned by `emergency_withdraw` when the pool is not currently paused.
    NotPaused = 13,
    /// Returned when the user has no stake or locked position.
    NoActiveStake = 14,
    /// Credit computation overflowed i128. Returned instead of trapping the
    /// contract via overflow-checks = true. The affected operation may still
    /// complete with degraded results (e.g., returning principal without the
    /// overflowing credit component).
    CreditOverflow = 15,
    /// Amount must be positive for lock/stake operations.
    InvalidAmount = 16,
    /// Unlock amount exceeds the locked position balance.
    InsufficientBalance = 17,
    /// Minimum lock period has not yet elapsed.
    LockPeriodNotElapsed = 18,
    /// Allocation percentage must be between 1 and 100.
    InvalidAllocation = 19,
    /// Pool is paused and the operation is not allowed.
    Paused = 20,
    /// Global multiplier was 0 (legacy variant, use `InvalidGlobalMultiplier`).
    InvalidMultiplier = 21,
    /// User is not whitelisted (whitelist mode is enabled).
    NotWhitelisted = 22,
    /// `global_multiplier` was 0 or exceeded `MAX_GLOBAL_MULTIPLIER`. See #89.
    InvalidGlobalMultiplier = 23,
}

/// Per-user boost configuration returned by `get_boost_config`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostConfig {
    pub multiplier: u32,
    pub allocation_pct: u32,
}

/// Recorded state for a user's stake position in the boost system.
#[contracttype]
#[derive(Clone, Debug)]
pub struct UserStake {
    pub amount: i128,
    pub start_ledger: u32,
    pub credits_banked: i128,
    /// Credit rate snapshot used for accrual since `start_ledger`.
    pub credit_rate: i128,
}

/// Recorded state for a user's locking position in the lock/unlock system.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Position {
    pub amount: i128,
    pub lock_ledger: u32,
    /// Earliest ledger at which the position may be unlocked.
    pub unlock_ledger: u32,
    pub checkpoint_ledger: u32,
    pub total_credits: i128,
    /// Credit rate snapshot used for accrual since `checkpoint_ledger`.
    pub credit_rate: i128,
}

/// Storage keys for all persistent and instance data.
#[contracttype]
pub enum DataKey {
    Admin,
    GlobalMultiplier,
    CreditRate,
    StakeToken,
    MinLockPeriod,
    SchemaVersion,
    Paused,
    UserBoost(Address),
    UserStake(Address),
    UserPosition(Address),
    BankedCredits(Address),
    // Whitelist keys
    WhitelistEnabled,
    Whitelisted(Address),
    MinStakeAmount,
}
