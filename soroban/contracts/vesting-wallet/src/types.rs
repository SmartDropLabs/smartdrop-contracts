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
    /// The vesting schedule is outside the supported limits: it runs for longer
    /// than the maximum vesting duration (365 days of ledgers). See #684.
    InvalidSchedule = 12,
    /// An operation that would modify the schedule was rejected because the
    /// beneficiary has permanently locked it via `lock_schedule` (#657).
    ScheduleIsLocked = 13,
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
    /// Global counter for schedule change events (#681).
    ScheduleHistoryCount,
    /// Schedule history event by index (#681).
    ScheduleHistory(u32),
    /// Publicly shared schedule template set by beneficiary (#648).
    SharedSchedule,
    /// Set to true once the beneficiary permanently locks the schedule (#657).
    ScheduleLocked,
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

/// A shareable vesting schedule template (#648).
///
/// When the beneficiary calls `share_schedule`, the wallet stores a copy of
/// the current schedule parameters here so other wallets can read it as a
/// reusable template without being granted full `SharingPermissions` access.
/// The template is stripped of the beneficiary address since a consumer would
/// substitute their own.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedScheduleTemplate {
    /// Token this template was created with (a hint; consumers may substitute).
    pub token: Address,
    pub total_amount: i128,
    pub start_ledger: u32,
    pub cliff_ledger: u32,
    pub end_ledger: u32,
    pub revocable: bool,
    /// Ledger at which the template was published.
    pub shared_at_ledger: u32,
}

/// A single schedule change event (#681).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleHistoryEntry {
    pub action: soroban_sdk::Symbol,
    pub old_schedule: Option<VestingSchedule>,
    pub new_schedule: Option<VestingSchedule>,
    pub ledger: u32,
}

/// Paginated response for schedule history queries (#681).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleHistoryPage {
    pub events: soroban_sdk::Vec<ScheduleHistoryEntry>,
    pub total: u32,
}

/// Vesting data for importing schedules (#702).
/// This type bundles all the information needed to initialize a vesting wallet
/// from external data, making it easier to import schedules from other sources.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingData {
    pub schedule: VestingSchedule,
    pub token: Address,
    pub admin: Address,
}

/// Layout version written into every `ScheduleBackup`. Bump it when the
/// backup's fields change so a restore tool can refuse a layout it does not
/// understand (#693).
pub const SCHEDULE_BACKUP_VERSION: u32 = 1;

/// Point-in-time copy of everything that defines a vesting wallet's state,
/// returned by `backup_schedule` (#693).
///
/// `VestingSchedule` holds the configured parameters only. Recovering a wallet
/// also needs who administers it, who funded it, how much has already been
/// released and whether it was revoked, so those are captured here too. It is a
/// separate type, not extra fields on `VestingSchedule`, so that struct's XDR
/// layout stays unchanged for clients that already decode it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleBackup {
    /// `SCHEDULE_BACKUP_VERSION` at the time the backup was taken.
    pub version: u32,
    /// Ledger sequence the backup was read at.
    pub backed_up_at_ledger: u32,
    pub schedule: VestingSchedule,
    pub admin: Address,
    pub funder: Address,
    pub treasury: Option<Address>,
    pub delegate: Option<Address>,
    /// Cumulative tokens already transferred to the beneficiary.
    pub released_amount: i128,
    /// Number of release operations performed.
    pub release_count: u32,
    pub revoked: bool,
    /// Vested amount frozen at revocation; 0 when not revoked.
    pub revoked_vested: i128,
}

/// Head-to-head comparison of two vesting wallets' schedules, returned by
/// `compare_schedules`.
///
/// `a` and `b` are the two wallets' live `VestingOverview`s, read at the same
/// ledger. Every `*_diff` field is `a - b`. Durations are in ledgers:
/// `duration_*` runs from start to end, `cliff_*` from start to cliff.
/// `vested_bps_*` is the vested share of the total in basis points
/// (10_000 = fully vested).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleComparison {
    pub wallet_a: Address,
    pub wallet_b: Address,
    pub a: VestingOverview,
    pub b: VestingOverview,
    pub total_amount_diff: i128,
    pub releasable_diff: i128,
    pub duration_a: u32,
    pub duration_b: u32,
    pub cliff_a: u32,
    pub cliff_b: u32,
    pub vested_bps_a: u32,
    pub vested_bps_b: u32,
    /// Whether both wallets vest the same token (a like-for-like comparison).
    pub same_token: bool,
    /// Wallet with the shorter start-to-end duration; `None` on a tie.
    pub faster_vesting: Option<Address>,
    /// Wallet whose cliff ledger is reached first; `None` on a tie.
    pub earlier_unlock: Option<Address>,
}

/// Optimization suggestion for a vesting schedule (#675).
///
/// Provides recommendations for improving schedule efficiency based on
/// current market conditions and best practices.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizationSuggestion {
    /// Type of optimization suggested
    pub suggestion_type: soroban_sdk::Symbol,
    /// Description of the optimization
    pub reason: soroban_sdk::String,
    /// Current value
    pub current_value: i128,
    /// Suggested value
    pub suggested_value: i128,
}

/// Response containing optimization suggestions for a vesting schedule (#675).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Suggestions {
    /// Address of the vesting wallet
    pub wallet: Address,
    /// List of optimization suggestions
    pub suggestions: Vec<OptimizationSuggestion>,
    /// Whether the schedule is already optimal
    pub is_optimal: bool,
}
