# VestingWallet — public API and event schemas

Reference for the `vesting-wallet` Soroban contract
(`soroban/contracts/vesting-wallet`): the entry points a client can call, the
values they return, and the events an indexer can rely on.

This complements [`docs/events.md`](events.md), the repo-wide event registry.
Where the two disagree, the contract source is authoritative — the
`released` payload correction below is the current state.

Vesting is measured in **ledger sequence numbers**, not wall-clock time.
`start_ledger`, `cliff_ledger` and `end_ledger` are all sequences, and the
schedule is linear **from `start_ledger`** (not from the cliff) to
`end_ledger`, with nothing releasable before the cliff:

```
vested(now) = 0                                        now < cliff_ledger
            = total * (now - start) / (end - start)    cliff <= now < end
            = total                                    now >= end
```

## Read entry points

| Entry point | Returns | Notes |
| :--- | :--- | :--- |
| `get_vesting_overview()` | `VestingOverview` | **One call for the whole schedule plus live progress.** Preferred for dashboards. |
| `get_vesting_schedule()` | `VestingSchedule` | Configured parameters only. Superseded by `get_vesting_overview` for anything that also shows progress. |
| `vested_amount()` | `i128` | Total vested as of the current ledger. |
| `released_amount()` | `i128` | Cumulative amount already transferred to the beneficiary. |
| `releasable()` | `i128` | Vested minus released — what a release would transfer right now. |
| `vesting_dates()` | `(u32, u32, u32)` | `(start_ledger, cliff_ledger, end_ledger)` in one read. |
| `revocable()` / `revoked()` | `bool` | Whether `admin` may revoke, and whether it already has. |
| `cliff_end_notified()` | `bool` | Whether the one-time `vest/cliff_end` event has fired (#615). |
| `beneficiary()` / `token()` / `admin()` | `Address` | Current addresses. `beneficiary` reflects `transfer_beneficiary`. |
| `release_count()` / `get_release_count()` | `u32` | Number of successful release operations. |

All read entry points return `NotInitialized` on a wallet that was never
initialized, and each extends the instance TTL.

### `VestingOverview`

Returned by `get_vesting_overview()`. Declared in `src/types.rs`.

| Field | Type | Description |
| :--- | :--- | :--- |
| `beneficiary` | `Address` | Current beneficiary. |
| `token` | `Address` | Token being vested. |
| `total_amount` | `i128` | Total placed in the schedule. |
| `start_ledger` | `u32` | Ledger at which linear vesting begins. |
| `cliff_ledger` | `u32` | Ledger before which nothing is releasable. |
| `end_ledger` | `u32` | Ledger at which the full amount is vested. |
| `revocable` | `bool` | Whether `admin` may still revoke. |
| `revoked` | `bool` | Whether the schedule has been revoked. |
| `vested_amount` | `i128` | Total vested now; frozen at the revocation ledger once revoked. |
| `released_amount` | `i128` | Cumulative amount already transferred. |
| `releasable_amount` | `i128` | Vested but unclaimed — what `release_all` transfers now. Never negative. |

`releasable_amount` is a saturating subtraction: after `emergency_withdraw`
freezes `vested_amount` at zero while `released_amount` is non-zero, it reports
`0` rather than a negative amount.

### `VestingSchedule`

Returned by `get_vesting_schedule()`. Its fields are a subset of
`VestingOverview` (`beneficiary`, `token`, `total_amount`, `start_ledger`,
`cliff_ledger`, `end_ledger`, `revocable`) and its layout is unchanged, so
clients already decoding it keep working. New consumers should prefer
`get_vesting_overview`.

## Write entry points

| Entry point | Authorization | Notes |
| :--- | :--- | :--- |
| `initialize(...)` | `admin` | Once per wallet. Pulls `total_amount` from `admin`. Rejects schedules over the [limits](#schedule-limits). |
| `release()` | `beneficiary` | Transfers the **entire** vested-but-unclaimed balance in one transfer. |
| `release_all()` | `beneficiary` | Convenience alias for `release()`. |
| `revoke()` | `admin` | Only when `revocable` and not already revoked. |
| `emergency_withdraw()` | `admin` | Break-glass: drains the raw balance and marks the schedule revoked. |
| `transfer_beneficiary(...)` | `admin` | Rejects the zero address. |
| `transfer_admin(...)` | `admin` | Current admin only. |

### Schedule limits

`initialize` (and `import_vesting`, which calls it) bounds the schedule so a mistake cannot
lock funds for an unreasonable time. Funds are pulled in at initialisation and the schedule
cannot be changed afterwards, so the check happens before anything is stored or transferred.

| Limit | Value | Error |
| :--- | :--- | :--- |
| Duration (`end_ledger - start_ledger`) | at most **6,307,200 ledgers** (365 days at about 5 s per ledger) | `InvalidSchedule` |
| `total_amount` | at most `i128::MAX / u32::MAX`, the ceiling the vesting formula can handle without overflow | `TotalAmountTooLarge` |

- The duration is measured from `start_ledger` to `end_ledger`, so a late cliff does not extend
  it: a cliff at the limit with the end one ledger later is still rejected.
- A schedule of exactly 6,307,200 ledgers is accepted; one ledger more is not.
- If both limits are broken, `TotalAmountTooLarge` is reported.
- A rejected `initialize` changes nothing: the wallet stays uninitialised and no tokens move,
  so it can be initialised again with a valid schedule.
- Before this limit a schedule could run for up to `u32::MAX` ledgers (over 600 years). Any
  schedule longer than 365 days must now be split or shortened.
- The other ordering rules are unchanged: `start_ledger` in the future, `cliff_ledger >=
  start_ledger`, `end_ledger > cliff_ledger`, and a positive `total_amount`.

### `release_all`

Claims everything currently vested in a single call, so a beneficiary never has
to call `release` repeatedly to drain a balance.

It takes **no arguments**: the beneficiary is read from storage and is the
account whose authorization is required. That is deliberate — a
`release_all(beneficiary)` signature would either ignore the argument or let a
third party force a release at a moment the beneficiary did not choose, which is
exactly what `release`'s authorization requirement exists to prevent.

`release_all` delegates to `release`, so the arithmetic, the authorization and
the emitted event are identical to a normal release; it is a self-documenting
entry point, not a second code path. It returns the amount transferred (`0` when
nothing has vested) and counts towards `release_count`.

## Events

Topics are written in emission order. Payload tables list fields in tuple
order.

### `init`

Emitted by `initialize`.

* **Topics:** `(Symbol, Symbol)` → `(symbol_short!("vest"), symbol_short!("init"))`

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `beneficiary` | `Address` | Beneficiary the schedule was created for. |
| `token` | `Address` | Token being vested. |
| `total_amount` | `i128` | Total pulled from `admin`. |
| `start_ledger` | `u32` | Ledger at which vesting begins. |
| `end_ledger` | `u32` | Ledger at which the full amount is vested. |

### `released`

Emitted by `release` — and therefore by `release_all` — whenever a nonzero
amount is transferred to the beneficiary. A call that has nothing to release
emits nothing, so an indexer never records a release that did not happen.

* **Topics:** `(Symbol, Symbol)` → `(symbol_short!("vest"), symbol_short!("released"))`

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `beneficiary` | `Address` | Address that received the tokens. |
| `releasable` | `i128` | Amount transferred by **this** call. |
| `released_total` | `i128` | Cumulative released total **after** this call. |

> The third field (`released_total`) is part of the published payload.
> `docs/events.md` previously listed only two fields; the tuple has always had
> three, so an indexer built from that table would mis-parse it. Track progress
> from `released_total` and the previous event's value rather than summing the
> per-call amounts yourself.

### `cliff_end`

Emitted once when a schedule with a real cliff reaches its cliff ledger. The
check runs in `release`, `withdraw_early`, and `revoke`; the stored flag makes
it idempotent. Schedules with `cliff_ledger == start_ledger` emit nothing.

* **Topics:** `(Symbol, Symbol)` → `(symbol_short!("vest"), symbol_short!("cliff_end"))`

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `beneficiary` | `Address` | The schedule's beneficiary. |
| `timestamp` | `u64` | Ledger timestamp when the cliff was detected. |

### `revoked`

Emitted by `revoke` (admin-only, requires `revocable = true` and not already
revoked).

* **Topics:** `(Symbol, Symbol)` → `(symbol_short!("vest"), symbol_short!("revoked"))`

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `admin` | `Address` | Admin that executed the revocation. |
| `vested` | `i128` | Vested and still claimable at the revocation ledger. |
| `unvested` | `i128` | Unvested remainder returned to the original funder. |

The unvested remainder goes to the **funder** (the address that funded the
schedule at `initialize`), not the current admin, so an admin transfer does not
redirect unvested tokens.

### `adm_xfr`

Emitted by `transfer_admin`.

* **Topics:** `(Symbol, Symbol)` → `(symbol_short!("vest"), symbol_short!("adm_xfr"))`

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_admin` | `Address` | The outgoing administrator. |
| `new_admin` | `Address` | The incoming administrator. |

## Errors

`VestingError`: `AlreadyInitialized` (1), `NotInitialized` (2), `NotRevocable` (3),
`AlreadyRevoked` (4), `Unauthorized` (5), `TotalAmountTooLarge` (6), `ArithmeticOverflow` (7),
`InvalidInput` (8), `InsufficientBalance` (9), `TreasuryNotConfigured` (10), `InvalidAmount` (11),
`InvalidSchedule` (12, a schedule longer than 365 days; see [Schedule limits](#schedule-limits)).

## Deployment caveat

This contract is standalone and `initialize` is **not** bound to the deploying
account, so an uninitialized wallet can be front-run and permanently occupied by
another caller. Deploy and initialize in one atomic transaction and never expose
an uninitialized wallet between transactions.
