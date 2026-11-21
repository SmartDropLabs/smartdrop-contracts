use soroban_sdk::{contracterror, contracttype, Address, Vec};

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
    /// The wallet does not hold enough tokens to satisfy a release.
    InsufficientBalance = 9,
    /// `withdraw_early` was called but no treasury address was configured. See #605.
    TreasuryNotConfigured = 10,
    /// `withdraw_early` was called with a zero or negative amount. See #605.
    InvalidAmount = 11,
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
    /// Address that receives early-withdrawal penalties. See #605.
    Treasury,
    /// Delegated address that can claim vesting rights on behalf of beneficiary.
    Delegate,
    /// Set once the cliff has been reached and its notification emitted (#615).
    CliffEndNotified,
    /// Monotonically increasing schedule version, bumped whenever the
    /// schedule's effective state changes (#666).
    ScheduleVersion,
    /// Sharing permissions controlling who may read via
    /// `get_vesting_overview_for` (#669).
    SharingPermissions,
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
    /// Schedule version, bumped on beneficiary transfer, revocation,
    /// emergency withdrawal, and migration (#666).
    pub version: u32,
}

/// Controls who may read this wallet's vesting progress via
/// `get_vesting_overview_for` (#669).
///
/// Defaults to fully public (`public: true`, no explicit viewers) when never
/// set, matching the pre-existing unguarded behavior of
/// `get_vesting_schedule` / `get_vesting_overview`, which remain available to
/// any caller regardless of this setting. The beneficiary can always read
/// their own schedule.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharingPermissions {
    /// When true, any caller may read via `get_vesting_overview_for`.
    pub public: bool,
    /// Addresses explicitly allowed to read when `public` is false.
    pub allowed_viewers: Vec<Address>,
}
