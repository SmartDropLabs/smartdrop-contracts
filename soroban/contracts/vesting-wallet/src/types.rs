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
    /// A supplied address is unusable (e.g. the zero address as beneficiary),
    /// which would strand released funds permanently (#405).
    InvalidInput = 8,
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

/// A vesting schedule plus its live progress, returned by
/// `get_vesting_overview`.
///
/// `VestingSchedule` above carries the configured schedule only. A dashboard
/// also needs how far along that schedule is, which previously meant three
/// further contract calls (`vested_amount`, `released_amount`, `releasable`).
/// This is a separate type rather than extra fields on `VestingSchedule` so the
/// existing struct's layout — and therefore its XDR, which clients already
/// decode — is unchanged (#409).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingOverview {
    pub beneficiary: Address,
    pub token: Address,
    pub total_amount: i128,
    pub start_ledger: u32,
    pub cliff_ledger: u32,
    pub end_ledger: u32,
    pub revocable: bool,
    pub revoked: bool,
    /// Total vested as of the current ledger (frozen at revocation).
    pub vested_amount: i128,
    /// Cumulative amount already transferred to the beneficiary.
    pub released_amount: i128,
    /// Vested but not yet claimed; the amount `release_all` would transfer.
    pub releasable_amount: i128,
}
