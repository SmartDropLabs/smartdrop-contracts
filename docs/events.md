# SmartDrop Contracts - Event Schema Registry

This document defines the event schemas actually emitted by the SmartDrop contracts (`Factory`, `FarmingPool`, and `VestingWallet`), as of the current `soroban/contracts` source. Off-chain indexers rely on these definitions; any modification to topics or payload shapes constitutes a client-breaking change.

Topic tuples below are written in emission order, exactly as passed to `env.events().publish((topic0, topic1), data)`. Payload tables list fields in tuple order (unnamed events publish a bare value, not a struct).

---

## 1. Factory Contract (`soroban/contracts/factory`)

### `pool_crtd`
Emitted by `create_pool` immediately after the new pool is deployed and initialized.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("factory"), symbol_short!("pool_crtd"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `pool_id` | `u32` | Monotonically assigned ID of the new pool. |
| `pool_address` | `Address` | The deployed contract address of the new pool. |
| `asset` | `Address` | The token asset address being staked in the pool. |
| `credit_rate` | `i128` | Per-ledger credit accrual rate, as passed to the pool's `initialize` (converted from `create_pool`'s caller-facing `daily_rate`). |
| `global_multiplier` | `u32` | Boost multiplier applied to allocated stake, as passed to `initialize`. |
| `min_lock_period` | `u32` | The minimum number of ledgers tokens must remain locked. |

### `adm_xfr`
Emitted by `transfer_admin` when the factory admin is rotated.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("factory"), symbol_short!("adm_xfr"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_admin` | `Address` | The outgoing administrator. |
| `new_admin` | `Address` | The incoming administrator. |

### `pool_upg`
Emitted by `upgrade_pool` after hot-swapping one registered pool's WASM.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("factory"), symbol_short!("pool_upg"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `pool_id` | `u32` | ID of the upgraded pool. |
| `pool_address` | `Address` | Contract address of the upgraded pool. |
| `new_wasm_hash` | `BytesN<32>` | The WASM hash the pool was upgraded to. |

### `wasm_set`
Emitted by `set_pool_wasm_hash` when the WASM hash used for *future* `create_pool` deployments changes.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("factory"), symbol_short!("wasm_set"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_hash` | `BytesN<32>` | The previous pool WASM hash. |
| `new_hash` | `BytesN<32>` | The newly configured pool WASM hash. |

---

## 2. FarmingPool Contract (`soroban/contracts/farming-pool`)

Note: the legacy `stake`/`unstake` entry points do **not** emit events — only the `Position`-based `lock_assets`/`unlock_assets` path does.

### `locked`
Emitted by `lock_assets` when a user deposits assets into the pool.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("locked"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The wallet address that locked assets. |
| `amount` | `i128` | The quantity of assets deposited in this call. |

### `unlocked`
Emitted by `unlock_assets` when a user withdraws assets from the pool.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("unlocked"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The wallet address that unlocked assets. |
| `amount` | `i128` | The quantity of assets withdrawn in this call. |
| `total_credits` | `i128` | The user's checkpointed total credit balance at the time of withdrawal. |

### `paused`
Emitted by `pause`.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("paused"))`
* **Payload:** `()` — no data.

### `unpaused`
Emitted by `unpause`.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("unpaused"))`
* **Payload:** `()` — no data.

### `emrg_exit`
Emitted by `emergency_withdraw` (user-authorized, requires the pool to be paused).

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("emrg_exit"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `admin` | `Address` | The pool admin address at the time of the emergency exit. |
| `user` | `Address` | The user whose funds were returned. |
| `total_returned` | `i128` | Combined amount returned across both the `Position` and `UserStake` records. |

### `adm_xfr`
Emitted by `transfer_admin` when the pool admin is rotated.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("adm_xfr"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_admin` | `Address` | The outgoing administrator. |
| `new_admin` | `Address` | The incoming administrator. |

### `upgraded`
Emitted by `upgrade` just before the contract's own WASM is updated.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("upgraded"))`
* **Payload:** `new_wasm_hash: BytesN<32>` — bare value, not a tuple.

### `rate_set`
Emitted by `set_credit_rate`.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("rate_set"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_rate` | `i128` | Credit rate before the update. |
| `new_rate` | `i128` | Credit rate after the update. |

### `lock_set`
Emitted by `set_min_lock_period`.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("lock_set"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_period` | `u32` | Minimum lock period (in ledgers) before the update. |
| `new_period` | `u32` | Minimum lock period (in ledgers) after the update. |

### `applied` (boost)
Emitted by `set_boost` when a user sets their allocation percentage.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("boost"), symbol_short!("applied"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The user who set their boost allocation. |
| `allocation_pct` | `u32` | The allocation percentage (1-100) applied. |
| `multiplier` | `u32` | The global multiplier in effect at the time of the call. |

### `mult_set` (boost)
Emitted by `set_global_multiplier`.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("boost"), symbol_short!("mult_set"))`
* **Payload:** `multiplier: u32` — bare value, not a tuple. The newly configured global multiplier.

---

## 3. VestingWallet Contract (`soroban/contracts/vesting-wallet`)

### `released`
Emitted by `release` whenever a nonzero amount is transferred to the beneficiary.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("vest"), symbol_short!("released"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `beneficiary` | `Address` | The address that received the released tokens. |
| `releasable` | `i128` | The amount transferred in this call. |

### `revoked`
Emitted by `revoke` (admin-only, requires `revocable = true` and not already revoked).

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("vest"), symbol_short!("revoked"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `admin` | `Address` | The admin address that executed the revocation. |
| `vested` | `i128` | The total amount vested (and still claimable by the beneficiary) as of the revocation ledger. |
| `unvested` | `i128` | The unvested remainder returned to admin immediately. |

### `adm_xfr`
Emitted by `transfer_admin` when the vesting wallet admin is rotated.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("vest"), symbol_short!("adm_xfr"))`
* **Payload Structure (tuple order):**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_admin` | `Address` | The outgoing administrator. |
| `new_admin` | `Address` | The incoming administrator. |
