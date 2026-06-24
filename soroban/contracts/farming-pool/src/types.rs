use soroban_sdk::{contracttype, Address};

/// Per-user boost configuration returned by `get_boost_config`.
/// `multiplier` is the current global multiplier set by the admin.
/// `allocation_pct` is the percentage of the user's stake allocated to boosted earning (1-100).
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostConfig {
    pub multiplier: u32,
    pub allocation_pct: u32,
}

/// Recorded state for a user's stake position (boost system).
/// Credits are checkpointed into `credits_banked` whenever the boost config or stake changes.
#[contracttype]
#[derive(Clone, Debug)]
pub struct UserStake {
    /// Token amount currently staked.
    pub amount: i128,
    /// Ledger sequence at which the last checkpoint occurred.
    pub start_ledger: u32,
    /// Credits already earned before the last checkpoint.
    pub credits_banked: i128,
}

/// Recorded state for a user's locking position (lock/unlock system).
/// `lock_ledger` is when the position was created and is used for minimum lock period enforcement.
/// `checkpoint_ledger` is the last time credits were banked; used for accrual calculation.
/// `total_credits` accumulates banked credits across partial unlocks and updates.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Position {
    /// Token amount currently locked.
    pub amount: i128,
    /// Ledger sequence when the position was first created (enforces minimum lock period).
    pub lock_ledger: u32,
    /// Ledger sequence of the last credit checkpoint.
    pub checkpoint_ledger: u32,
    /// Credits banked at the last checkpoint.
    pub total_credits: i128,
}

/// Storage keys for all persistent and instance data.
#[contracttype]
pub enum DataKey {
    Admin,
    GlobalMultiplier,
    /// Credits accrued per unit of effective stake per ledger.
    CreditRate,
    StakeToken,
    /// Minimum number of ledgers a position must be locked before unlock is allowed.
    MinLockPeriod,
    /// Whether the pool is paused (admin-controlled emergency switch).
    Paused,
    /// Per-user boost allocation percentage (u32, 1-100). Absent if boost not set.
    UserBoost(Address),
    /// Per-user stake record (boost system).
    UserStake(Address),
    /// Per-user locking position (lock/unlock system).
    UserPosition(Address),
}
