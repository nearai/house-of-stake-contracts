# Automatic subscription downgrade when billing period ends

**Type:** Bug / product gap  
**Component:** `staking-contract` — subscriptions (`subscriptions.rs`, `lock.rs`)  
**Labels (suggested):** `bug`, `subscription`, `enhancement`, `house-of-stake`

---

## Summary

Scheduled subscription downgrades (`schedule_downgrade_subscription`) are **not applied when the current billing period ends**. The contract only records `pending_downgrade_price_id` and waits for the subscriber to manually call `lock_for_subscription` with the lower tier after `subscription.end_ns`.

From a subscriber and product perspective, this is a **bug**: users expect “downgrade at end of period” to take effect automatically at period boundary, similar to Stripe-style cancel-at-period-end / tier changes at renewal.

---

## Current behavior (as implemented)

### Phase A — schedule only

- `schedule_downgrade_subscription(target_price_id)` sets `Subscription.pending_downgrade_price_id`.
- **`price_id` stays on the current (higher) tier** for the rest of the period.
- No refund or stake adjustment mid-cycle (documented: Phase A, no automatic refund).

### Phase B — manual renewal required

Downgrade **completes** only when the user sends a successful **`lock_for_subscription`** transaction **after** `block_timestamp >= subscription.end_ns`, with:

- `price_id` equal to the scheduled lower tier, and  
- Attached NEAR satisfying `check_near_price_lock` for the new period.

On success, `commit_catalog_lock` → `apply_pending_downgrade_before_renewal_lock`:

- Applies tier-gap prorate on `last_lock_id` (queues unstake for surplus high−low minimum stake).
- Updates `price_id` to the lower tier and clears `pending_downgrade_price_id`.
- Mints a new period lock.

There is **no on-chain cron, keeper, or callback at `end_ns`**. Period end alone does not trigger any state transition.

### Relevant code

| Step | Location |
|------|----------|
| Schedule downgrade | `subscriptions.rs` — `schedule_downgrade_subscription` |
| Renewal gate (`now >= end_ns`) | `lock.rs` — `lock_for_subscription_with_price_id` |
| Prorate at commit | `subscriptions.rs` — `apply_pending_downgrade_before_renewal_lock` |
| Docs | `docs/API.md` — “lower tier applied at next `lock_for_subscription` renewal” |

---

## Expected behavior (product)

When a user schedules a downgrade:

1. **Through the current period:** remain on the higher tier (no mid-cycle refund) — acceptable.
2. **When the current period ends (`end_ns`):** downgrade should **complete automatically** without requiring a separate `lock_for_subscription` call, including:
   - `price_id` → lower tier  
   - Tier-gap prorate applied (Phase B)  
   - Next billing period opened (or clearly defined stake/lock state for the lower tier)  
   - `pending_downgrade_price_id` cleared  

Optional: notify / index for off-chain UIs that tier changed at `end_ns`.

---

## Why this is hard on NEAR (constraints)

- Smart contracts cannot run at a timestamp without an **external transaction** (user, relayer, or cron contract).
- “Automatic at period end” implies one of:
  - **Keeper / relayer** that watches `end_ns` and calls a new contract method (e.g. `renew_subscription_downgrade`).
  - **Deferred action** pattern (if adopted) scheduled at `end_ns` (protocol/feature dependent).
  - **Virtual projection** in views only (UX shows future tier but chain state unchanged until someone calls) — **not** a full fix.

Any fix should preserve recent pipeline safety work: prorate must not run before settlement succeeds (see `apply_pending_downgrade_before_renewal_lock` in `commit_catalog_lock`).

---

## Proposed directions (for design)

### Option A — Keeper-triggered renewal (recommended for true on-chain auto)

- Add something like `apply_scheduled_downgrade_renewal(subscription_id)` (or by `(account_id, product_id)`):
  - `require!(block_timestamp >= subscription.end_ns)`  
  - `require!(pending_downgrade_price_id.is_some())`  
  - Reuse renewal + `commit_catalog_lock` path (with attached NEAR rules documented: keeper prefunds vs pull from lock surplus only).
- Document operator/relayer responsibility; optional incentive (tips) or protocol cron.

**Open questions:** Who attaches NEAR for the next period’s lock? Prorate releases surplus to unstake queue — is that enough to fund the lower-tier lock without a new deposit?

### Option B — Auto-apply tier + prorate only; lock still manual

- At period end (via keeper call): run prorate + update `price_id` + extend `start_ns`/`end_ns` **without** minting a new lock.
- User still calls `lock_for_subscription` to stake for the new period.

Smaller change but still not fully “automatic subscription renewal.”

### Option C — Off-chain automation only

- Indexer + backend calls `lock_for_subscription` on behalf of users after `end_ns` (signed meta-tx / account abstraction).

No contract change; operational burden and trust model shift.

---

## Acceptance criteria (suggested)

- [ ] After `schedule_downgrade_subscription`, when `block_timestamp >= subscription.end_ns`, tier change can complete **without** the user calling `lock_for_subscription` manually (define exact method: on-chain keeper vs documented off-chain service).
- [ ] Phase B prorate runs **once**, only after successful settlement (no regression of idempotent commit path).
- [ ] `pending_downgrade_price_id` cleared when downgrade completes; `price_id` reflects lower tier.
- [ ] Behavior documented in `API.md` / `CORE_FEATURES.md`.
- [ ] Tests: host and/or sandbox for period-end downgrade (scheduled → time advance → single apply tx → assert tier + prorate + no double-apply on retry).

---

## Related / recent fixes (context)

- Payable pipeline refunds and `Busy` release (`epoch.rs`) — separate from this issue.  
- Downgrade prorate moved to `commit_catalog_lock` to avoid double-apply on failed async renewal — **keep** when implementing auto-apply.

---

## References

- Review thread: scheduled downgrade applied only at manual `lock_for_subscription` renewal.  
- Test (happy path, manual renewal): `tests/subscription_lifecycle.rs` — `downgrade_applies_at_next_renewal`.
