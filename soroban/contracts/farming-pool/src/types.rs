use soroban_sdk::{contracterror, contracttype, Address};

/// Error codes returned by the farming pool contract.
#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PoolError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidCreditRate = 3,
    NotPaused = 13,
    Paused = 20,
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
    Paused,
    UserBoost(Address),
    UserStake(Address),
    UserPosition(Address),
    BankedCredits(Address),
}
