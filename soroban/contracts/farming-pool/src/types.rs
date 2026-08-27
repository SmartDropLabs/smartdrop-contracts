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
    /// Global multiplier snapshot captured at checkpoint time (#60).
    pub multiplier: u32,
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
