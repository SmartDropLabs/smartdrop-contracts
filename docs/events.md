# SmartDrop Contracts - Event Schema Registry

This document defines the official event schemas emitted by the SmartDrop contracts (`Factory` and `FarmingPool`). Off-chain indexers rely on these strict definitions; any modification to topics or payload shapes constitutes a client-breaking change.

---

## 1. Factory Contracts

### `pool_created`
Emitted immediately whenever a new liquidity or farming pool is successfully deployed by the factory.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("Factory"), symbol_short!("pool_created"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `pool_id` | `Symbol` | The unique symbol identifier of the pool (Max 9 characters). |
| `pool_address` | `Address` | The deployed contract address of the new pool. |
| `asset` | `Address` | The token asset address being staked in the pool. |
| `daily_rate` | `u128` | The base daily reward generation rate for stakers. |
| `min_lock_period`| `u64` | The minimum duration (in seconds) tokens must remain locked. |

### `adm_chg`
Emitted when administrative privileges are transferred to a new account.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("Factory"), symbol_short!("adm_chg"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `old_admin` | `Address` | The address of the outgoing administrator. |
| `new_admin` | `Address` | The address of the incoming administrator. |

---

## 2. FarmingPool Contracts

### `staked`
Emitted when a user deposits assets into the farming pool to begin earning rewards.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("staked"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The wallet address of the staker performing the action. |
| `amount` | `u128` | The total quantity of assets deposited. |

### `unstaked`
Emitted when a user withdraws their assets from the farming pool.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("unstaked"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The wallet address of the unstaker performing the withdrawal. |
| `amount` | `u128` | The total quantity of assets withdrawn. |
| `total_credits` | `u128` | The updated total point/credit balance remaining for the user. |

### `paused`
Emitted by the contract administrator to temporarily halt pool actions.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("paused"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `admin` | `Address` | The administrative actor address executing the pause command. |

### `unpaused`
Emitted by the contract administrator to resume all pool interactions.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("pool"), symbol_short!("unpaused"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `admin` | `Address` | The administrative actor address executing the resume command. |

### `claimed`
Emitted when a user claims their accumulated farming rewards.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("Reward"), symbol_short!("claimed"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `user` | `Address` | The wallet address receiving the reward distribution. |
| `allocation_pct` | `u32` | The finalized percentage allocation of the pool claimed. |
| `multiplier` | `u128` | The applicable yield multiplier applied to the user's claim base. |

### `mult_set`
Emitted when the global yield multiplier tier system is adjusted.

* **Topics:** `(Symbol, Symbol)` -> `(symbol_short!("Reward"), symbol_short!("mult_set"))`
* **Payload Structure:**

| Field | Rust Type | Description |
| :--- | :--- | :--- |
| `multiplier` | `u128` | The newly established global reward multiplier threshold. |
