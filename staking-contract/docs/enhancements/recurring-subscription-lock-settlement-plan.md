# Recurring Subscription Lock Settlement Refactor Plan

## Summary

Make recurring subscription locking follow the same atomicity model as one-off locks:
subscription and lock state should be committed only after the shared pre-user epoch
settlement pipeline reaches the lock callback.

The current implementation is only partially aligned with this model. It settles first
when an existing subscription has a due pending stake decrease, but the normal recurring
path can still resolve or mutate subscription state before the settlement promise and
lock tail have succeeded.

## Key Changes

- Route all WASM recurring subscription locks through
  `UserAction::CommitRecurringSubscriptionLock`, not only the due pending stake-decrease
  case.
- Keep the public `lock` entrypoint lightweight for recurring prices:
  - resolve active price and product;
  - validate recurring monthly price, validator activity, attached amount, and gas;
  - start `promise_validator_per_epoch_settlement_then`.
- Move recurring subscription resolution into
  `resolve_recurring_subscription_lock_after_settle`:
  - re-read active price/product by `price_id`;
  - find the current or projected subscription;
  - apply due pending updates after settlement;
  - handle stale cancelled subscription cleanup;
  - renew expired active subscription windows;
  - create a new subscription when needed;
  - validate target amount, current tier, and current stake invariants;
  - call `commit_catalog_lock`.
- Keep the host test path synchronous, but have it call the same post-settlement helper
  directly so host behavior stays aligned with WASM behavior.
- Ensure these state changes happen only in the post-settlement path:
  subscription window updates, subscription deletion, account/global index changes,
  pending update application, lock creation, usage counters, and validator
  `pending_to_stake`.

## Test Plan

- Add or adjust host tests for recurring lock renewal to confirm subscription
  `start_ns`, `end_ns`, and `last_lock_id` are updated by the post-settlement helper.
- Add a sandbox/WASM-style failure test if feasible: a failed settlement or lock tail
  must not advance subscription windows or remove stale cancelled subscriptions before
  the lock is minted.
- Run:
  - `cargo test -p staking-contract --test subscription_lifecycle`
  - `cargo test -p staking-contract --test sandbox_epoch_settlement` when the local
    environment supports near-workspaces
  - `cargo check -p staking-contract --target wasm32-unknown-unknown`

## Assumptions

- Recurring subscription lock should have the same atomicity expectation as one-off
  lock: user-visible subscription state changes only after pre-user settlement reaches
  the lock callback.
- Re-reading catalog and subscription state in the callback is preferred over passing
  precomputed subscription state through the promise, because callback-time state is the
  source of truth after settlement delay.
- Keeping a separate `CommitRecurringSubscriptionLock` action is acceptable because
  recurring locks require subscription resolution before `commit_catalog_lock`.
