use soroban_sdk::{contracterror, contracttype};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum VestingError {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    NotRevocable       = 3,
    AlreadyRevoked     = 4,
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
    /// Whether the schedule can be revoked by admin.
    Revocable,
    /// Set to true once admin calls revoke().
    Revoked,
    /// Vested amount frozen at the moment of revocation.
    RevokedVested,
}
