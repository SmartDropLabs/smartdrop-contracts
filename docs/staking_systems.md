# Staking Systems Architecture & Factory Registry Guide

## 1. Overview of Staking Systems

The SmartDrop `FarmingPool` smart contract provides two staking models tailored to different campaign requirements:

1. **Lock/Unlock Position System (`Position`)**: Time-locked staking where deposits are committed for a minimum duration (`min_lock_period`).
2. **Boost / Stake System (`UserStake`)**: Continuous, non-locked staking with optional user-allocated multiplier boosts (`BoostConfig`).

Both systems can coexist within the same deployed pool instance, and users may participate in either or both concurrently.

---

## 2. Comparison & Feature Matrix

| Feature | Time-Locked System (`Position`) | Boost / Stake System (`UserStake`) |
| :--- | :--- | :--- |
| **Primary State Struct** | `Position` | `UserStake` |
| **Storage Key** | `DataKey::UserPosition(Address)` | `DataKey::UserStake(Address)` |
| **Deposit Function** | `lock_assets(user, amount)` | `stake(from, amount)` |
| **Withdraw Function** | `unlock_assets(user, amount)` | `unstake(from)` |
| **Time Lock** | Enforces `min_lock_period` ledgers | None (flexible withdrawal at any time) |
| **Partial Withdrawal** | Supported (amount <= locked amount) | Full unstake of current balance |
| **Boost Multipliers** | Fixed standard accrual | Configurable boost via `set_boost(allocation_pct)` |
| **Credit Rate Snapshot** | Checkpointed in `checkpoint_ledger` | Checkpointed in `start_ledger` |
| **Credit Calculation** | `calculate_credits(user)` | `get_credits(user)` |
| **Emitted Events** | `(pool, locked)`, `(pool, unlocked)` | None (governed via token/boost events) |

---

## 3. Accrual Formulas

### 3.1 Time-Locked Position Accrual (`Position`)

Accrued credits for locked positions grow linearly with the elapsed ledgers and pool credit rate:

$$\text{Credits} = \text{total\_credits} + (\text{amount} \times \text{credit\_rate} \times \Delta\text{ledgers})$$

Where $\Delta\text{ledgers} = \text{current\_ledger} - \text{checkpoint\_ledger}$.

### 3.2 Boosted Stake Accrual (`UserStake`)

Accrued credits incorporate the user's boost allocation percentage and global multiplier:

$$\text{effective\_stake} = (\text{amount} - \text{boosted\_amount}) + (\text{boosted\_amount} \times \text{multiplier})$$

$$\text{where } \text{boosted\_amount} = \frac{\text{amount} \times \text{allocation\_pct}}{100}$$

$$\text{Credits} = \text{credits\_banked} + (\text{effective\_stake} \times \text{credit\_rate} \times \Delta\text{ledgers})$$

Where $\Delta\text{ledgers} = \text{current\_ledger} - \text{start\_ledger}$.

---

## 4. Emergency Withdrawals & Pause Lifecycle

When unexpected market conditions or upgrades require pausing a pool (`pause()`), normal deposits and withdrawals (`lock_assets`, `unlock_assets`, `stake`, `unstake`) are halted to protect contract invariants.

- **User Self-Withdrawal**: Users can call `emergency_withdraw(user)` directly while the pool is paused.
- **Atomic Exit**: All locked tokens in `Position` and staked tokens in `UserStake` are transferred back to the user in a single atomic transaction.
- **Accrual History Preservation**: Accrued credits are preserved in `BankedCredits` under `BankedCreditTotals { position_credits, stake_credits }`, allowing users and indexers to inspect credits earned prior to the emergency exit via `get_banked_credits_split(user)` and `get_banked_credits(user)`.

---

## 4a. Why the Boost/Stake system has no minimum lock period

The flexible `UserStake` system deliberately does **not** enforce a lock period like the `Position` system does (`stake`/`unstake` vs `lock_assets`/`unlock_assets`). This is a documented design decision, not an oversight ([#169](https://github.com/SmartDropLabs/smartdrop-contracts/issues/169)):

- **Different product purpose**: the lock system commits deposits for a minimum duration; the boost/stake system exists for *continuous flexible staking* where a lock would defeat its purpose.
- **No leverage, no flash-staking reward**: a stake is not a loan — `unstake` returns only the exact staked principal. Credits accrue **linearly over elapsed ledgers** (`compute_stake_accrual`), with checkpoints on both `stake` and `unstake`, so an immediate stake→unstake round-trip banks ~0 credits. There is no fixed up-front reward to harvest.
- **Bounded exposure**: the maximum "harm" of free stake/unstake is per-transaction gas, which the caller pays. The contract never over-commits liabilities beyond staked amounts.
- **Guidance**: pools that need a commitment lock should rely on the `Position` system; pools that want flexible boosted staking use the stake system as-is. If a future product needs a locked *boosted* stake, add a separate opt-in `min_stake_lock_period` parameter rather than coupling the two model.

---

## 5. Factory Registry & Scan Gas Economics

The `Factory` contract manages pool deployments and provides query functions to locate pools by staking asset:

- `get_pools_by_asset(asset, start_id, limit)`: Scans up to `MAX_POOL_SCAN_PER_CALL` (200) pool IDs per invocation.
- `get_pools_by_asset_range(asset, start_id, scan_limit, limit)`: Allows callers to specify a custom scan window `scan_limit` (capped at 200).

### Resource Bounds & Gas Model
- **Deterministic Resource Consumption**: Bounding the scan window per call prevents transactions from exceeding Soroban's CPU instruction (100M) and persistent storage read entry footprint budgets.
- **Pagination**: Callers can iterate by passing `next_start_id` as `start_id` until `next_start_id == total`.
- **Off-Chain Indexer Best Practice**: For high-volume production frontends querying across thousands of registered pools, frontends should subscribe to and index `(symbol_short!("factory"), symbol_short!("pool_crtd"))` events rather than scanning on-chain.
