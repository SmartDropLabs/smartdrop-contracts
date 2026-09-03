use soroban_sdk::{contracterror, contracttype, Address};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum VestingError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotRevocable = 3,
    AlreadyRevoked = 4,
    Unauthorized = 5,
    TotalAmountTooLarge = 6,
    ArithmeticOverflow = 7,
}

/// Storage keys for all instance data in the vesting wallet.
#[contracttype]
pub enum DataKey {
    Beneficiary,
    Token,
    /// Total tokens placed in the vesting schedule.
    TotalAmount,
    /// Ledger sequence at which linear vesting begins counting.
    StartLedger,
    /// Ledger sequence before which nothing is releasable; vesting uses start as origin.
    CliffLedger,
    /// Ledger sequence at which the full amount is vested.
    EndLedger,
    /// Cumulative tokens already transferred to the beneficiary.
    ReleasedAmount,
    /// Address authorised to revoke (admin).
    Admin,
    /// Original address that funded the vesting schedule (set at init, never changed).
    Funder,
    /// Whether the schedule can be revoked by admin.
    Revocable,
    /// Set to true once admin calls revoke().
    Revoked,
    /// Vested amount frozen at the moment of revocation.
    RevokedVested,
    /// Running count of release operations performed.
    ReleaseCount,
}

/// Emitted when admin rights are transferred.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferred {
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Full vesting schedule parameters, returned by `get_vesting_schedule`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingSchedule {
    pub beneficiary: Address,
    pub token: Address,
    pub total_amount: i128,
    pub start_ledger: u32,
    pub cliff_ledger: u32,
    pub end_ledger: u32,
    pub revocable: bool,
}
