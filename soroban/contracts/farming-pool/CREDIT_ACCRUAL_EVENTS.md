# Farming Pool Credit Accrual Events

## Purpose

Indexers can observe credit accrual when a user’s balance is checkpointed during an asset lock, unlock, stake, unstake, or boost update. This document records the existing event contract so consumers can distinguish checkpointed credits from read-only estimates.

## Checkpoint Event

When a checkpoint accrues a positive amount, the farming pool publishes:

- Topics: `("pool", "chkpt")`
- Data: `(user, accrued_delta, total_credits_after_checkpoint)`

No checkpoint event is emitted when the accrued delta is zero. Consumers should treat the absence of an event as no newly committed credit for that operation, not as a failed transaction.

## Position And Flexible Stake Semantics

`lock_assets` and `unlock_assets` checkpoint a user’s time-locked `Position`. `stake`, `unstake`, and `set_boost` checkpoint the flexible `UserStake`. Both paths publish the same checkpoint event shape when they add credits, allowing one indexer projection to consume the two accrual systems.

The event’s total is the balance for the checkpointed record after the new delta. It is not a complete wallet-wide balance; indexers that display a combined total must also reconcile the position, flexible-stake, and any banked-credit records.

## Consumer Guidance

1. Deduplicate events by transaction and event position before updating a projection.
2. Use the event delta for incremental accounting and retain the reported total for reconciliation.
3. Do not infer accrual from a successful lock or unlock alone; a transaction can checkpoint zero credits.
4. Treat the contract’s storage and explicit getters as the source of truth for recovery after an indexer outage.

## Related Access Controls

New locking and flexible staking both apply the configured whitelist when it is enabled. The current credit calculations also resolve the active global multiplier during flexible-stake accrual. These guards are contract behavior and should not be reimplemented as client assumptions.