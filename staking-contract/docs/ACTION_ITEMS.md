# Staking contract — remaining action items

This file tracks open work relative to the intended design ([PLAN.md](PLAN.md), [DESIGN.md](DESIGN.md)) and the current implementation. Update it as items land.

---

## P0 — Core unlock → withdraw path (funds must not stick)

**Done (v1):** `epoch_unstake`, `epoch_withdraw` (get unstaked + `withdraw`), `on_epoch_withdraw_transfer_done` → `pending_to_withdraw`, `claim_unlocked_near` → `withdrawable_balance`, `withdraw`. See `src/epoch.rs`, `src/pool_callbacks.rs`, `src/withdraw.rs`, `src/unlock.rs`.

**Follow-ups:**

- [x] **Actual vs requested withdraw** — `on_epoch_withdraw_transfer_done` credits `min(balance_after − balance_before, requested)` using a pre-withdraw balance snapshot on [`Validator::balance_before_epoch_withdraw_yocto`](src/validators.rs).

---

## P1 — ~~Oracle & USD-priced locks~~ (removed)

The contract is **NEAR-only**: no oracle, no USD catalog path, no `oracle-relay-contract`. See [README.md](../README.md). [PLAN.md](PLAN.md) and [DESIGN.md](DESIGN.md) describe NEAR-only pricing; superseded USD/oracle ideas were removed from those docs.

---

## P1 — Subscriptions

- [x] **`lock_for_subscription`** — NEAR + monthly recurring catalog prices; persists [`Subscription`](src/types.rs) and index `(account_id, product_id)` → `subscription_id`.
- [x] **Lifecycle** — [`cancel_subscription`](src/lock.rs), [`upgrade_subscription`](src/lock.rs), [`schedule_downgrade_subscription`](src/lock.rs); events in [`events.rs`](src/events.rs).
- [x] **Month stacking helper** — [`add_months_stripe_style`](src/subscriptions.rs); **calendar-accurate** end dates still future work (anchor_day recorded; linear months only).
- [x] **Downgrade prorate (Phase B)** — at renewal when a scheduled downgrade applies, tier-gap NEAR (`min_locked(high)` − `min_locked(low)` for the completed billing window) is released via [`Contract::queue_shares_unstake`](src/unlock.rs) (same path as `unlock` → epoch → `claim_unlocked_near`). See [`apply_downgrade_prorate_at_renewal`](src/lock.rs).

---

## P2 — Accounts & storage (NEP-145)

- [x] **`storage_withdraw`**
- [x] **Per-lock bounds** — [`Config::per_lock_storage_stake`](src/config.rs) × [`Contract::user_lock_count`](src/lib.rs); govern via [`set_per_lock_storage_stake`](src/governance.rs).

---

## P2 — Observability & UX

- [x] **EVENT_JSON** — `events.rs`: lock, unlock, product create, validator add, claim, withdraw, epoch ops, pool withdraw-in.
- [x] **User paths** — `require!` / `env::panic_str` on user-facing entrypoints.
- [x] **Broader require sweep** — catalog/admin paths in [`products.rs`](src/products.rs) use `require!` for missing entities.

---

## P2 — Accounting & edge cases

- [x] **`on_refresh_total_balance` note** — Module doc in `pool_callbacks.rs` (share vs pool balance; future share true-up).
- [ ] **Reconcile refresh with shares** — **Design:** periodic operator refresh vs reward drift; **no automatic mint/rebase** in this version (documented in [`pool_callbacks.rs`](src/pool_callbacks.rs)).

---

## P3 — Testing & docs

- [x] **Unit tests** — Pro-rata claim, share mint (`internal.rs`, `withdraw.rs`, `subscriptions.rs`).
- [x] **README** — Operator cadence + status (see [README.md](README.md)).
- [x] **Integration / sandbox tests** — [`integration-tests/tests/test_staking_contract.rs`](../integration-tests/tests/test_staking_contract.rs) deploy + `get_config` (requires built WASM: `make staking-contract`).

---

## Quick reference — stubs

| Location | Note |
|----------|------|
| [`src/subscriptions.rs`](src/subscriptions.rs) | Calendar **day** / end-of-month not fully modeled. |
| [`src/lock.rs`](src/lock.rs) | NEAR-only `lock_for_product` / `lock_for_subscription`. |

---

*Last updated: NEAR-only catalog (oracle removed), subscription locks, storage metering, integration smoke test.*
