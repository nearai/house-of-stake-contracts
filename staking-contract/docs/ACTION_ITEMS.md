# Staking contract — remaining action items

This file tracks open work relative to the intended design ([PLAN.md](PLAN.md), [DESIGN.md](DESIGN.md)) and the current implementation. Update it as items land.

---

## P0 — Core unlock → withdraw path (funds must not stick)

**Done (v1):** Lazy pipeline in [`epoch.rs`](src/epoch.rs): pool `unstake` / withdraw-from-pool chains, `claim_unlocked_near` → tranche / `withdrawable_balance` / `withdraw`, plus [`unlock.rs`](src/unlock.rs) and [`withdraw.rs`](src/withdraw.rs). Public batch `epoch_unstake` / `epoch_withdraw` entrypoints are **not** exposed; see [`LAZY_EPOCH_PIPELINE.md`](LAZY_EPOCH_PIPELINE.md).

**Follow-ups:**

- [x] **Actual vs requested withdraw** — Current implementation: on success, withdraw completion callbacks in [`epoch.rs`](src/epoch.rs) credit the requested amount into [`Validator::pending_to_withdraw`](src/validators.rs). The snapshot-based reconciliation `min(balance_after − balance_before, requested)` and a `balance_before_epoch_withdraw` field are **not** implemented in this version.

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

- [x] **`on_refresh_total_balance`** — Removed from `epoch.rs` (never scheduled; balance sync uses settlement callbacks).
- [ ] **Reconcile refresh with shares** — **Design:** reward drift vs cached `total_staked_balance`; **no automatic mint/rebase** in this version. Balance views run inside the lazy settlement pipeline ([`epoch.rs`](src/epoch.rs), [`LAZY_EPOCH_PIPELINE.md`](LAZY_EPOCH_PIPELINE.md)).

---

## P3 — Testing & docs

- [x] **Unit tests** — Pro-rata claim, share mint (`internal.rs`, `withdraw.rs`, `subscriptions.rs`).
- [x] **README** — User-driven lazy cadence (see [README.md](README.md)).
- [x] **Integration / sandbox tests** — [`integration-tests/tests/test_staking_contract.rs`](../integration-tests/tests/test_staking_contract.rs) deploy + `get_config` (requires built WASM: `make staking-contract`).

---

## Quick reference — stubs

| Location | Note |
|----------|------|
| [`src/subscriptions.rs`](src/subscriptions.rs) | Calendar **day** / end-of-month not fully modeled. |
| [`src/lock.rs`](src/lock.rs) | NEAR-only `lock_for_product` / `lock_for_subscription`. |

---

*Last updated: NEAR-only catalog (oracle removed), subscription locks, storage metering, integration smoke test.*
