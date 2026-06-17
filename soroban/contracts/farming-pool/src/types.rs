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

/// Recorded state for a user's stake position.
/// Credits are checkpointed into `credits_banked` whenever the boost config or stake changes,
/// so accrued credits are never lost across updates.
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

/// Storage keys for all persistent and instance data.
#[contracttype]
pub enum DataKey {
    Admin,
    GlobalMultiplier,
    /// Credits accrued per unit of effective stake per ledger.
    CreditRate,
    StakeToken,
    /// Per-user boost allocation percentage (u32, 1-100). Absent if boost not set.
    UserBoost(Address),
    /// Per-user stake record.
    UserStake(Address),
}
