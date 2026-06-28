use soroban_sdk::{contracterror, contracttype, Address};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PoolError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotPaused = 13,
    NoActiveStake = 14,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BoostConfig {
    pub multiplier: u32,
    pub allocation_pct: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct UserStake {
    pub amount: i128,
    pub start_ledger: u32,
    pub credits_banked: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Position {
    pub amount: i128,
    pub lock_ledger: u32,
    pub checkpoint_ledger: u32,
    pub total_credits: i128,
}

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
