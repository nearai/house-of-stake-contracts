# Pending subscription updates at billing period end

**Type:** Historical issue / resolved design note
**Component:** `staking-contract` — subscriptions (`subscriptions.rs`, `lock.rs`)
**Status:** Superseded by `update_subscription` + `pending_update`

---

## Summary

<<<<<<< HEAD
Scheduled subscription downgrades (`schedule_downgrade_subscription`) are **not applied when the current billing period ends**. The contract only records `pending_downgrade_price_id` and waits for the subscriber to manually call `lock` with the lower tier after `subscription.end_ns`.
=======
The old split downgrade flow required a subscriber to manually renew with `lock` after
`subscription.end_ns` before a scheduled downgrade could take effect. That model has
been replaced by `update_subscription(subscription_id, target_price_id, target_amount)`.
>>>>>>> origin/feat/stake-dao

The current model stores a `Subscription.pending_update` with:

- optional `target_price_id` for a deferred plan change
- optional `target_amount` for a deferred stake decrease
- `apply_ns`, normally the current billing period end

Views project a due pending update after `apply_ns`, and mutation paths can lazily
commit the due update. This means the subscription can move to the next period
without requiring the user to call `lock` with the target tier.

---

## Current behavior

1. `update_subscription` decides whether plan and stake changes apply immediately
   or at period end.
2. Immediate stake increases attach the exact NEAR delta and run through the
   validator settlement pipeline before shares are minted.
3. Deferred stake decreases and deferred plan changes are stored in
   `Subscription.pending_update`.
4. After `apply_ns`, subscription views project the target plan.
5. A later mutation lazily commits the pending update and clears
   `pending_update`. If the pending update includes a stake decrease, the
   mutation first runs the validator settlement preamble, then queues the
   surplus through the normal pending-unstake accounting.

There is still no autonomous on-chain cron. A transaction must touch the
subscription before storage changes are committed, but callers no longer need to
manually renew with the lower tier.

---

<<<<<<< HEAD
Downgrade **completes** only when the user sends a successful **`lock`** transaction **after** `block_timestamp >= subscription.end_ns`, with:

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
| Renewal gate (`now >= end_ns`) | `lock.rs` — `lock_with_price_id` |
| Prorate at commit | `subscriptions.rs` — `apply_pending_downgrade_before_renewal_lock` |
| Docs | `docs/API.md` — “lower tier applied at next `lock` renewal” |
=======
## Relevant code

| Step | Location |
|------|----------|
| Schedule/update plan or stake | `subscriptions.rs` — `update_subscription` |
| Pending update projection | `subscriptions.rs` — `project_subscription_view_now` |
| Lazy commit | `subscriptions.rs` — `apply_due_subscription_update` |
| Subscription lock/renewal | `lock.rs` — `lock_recurring_subscription_with_catalog` |
>>>>>>> origin/feat/stake-dao

---

## Remaining operational note

<<<<<<< HEAD
When a user schedules a downgrade:

1. **Through the current period:** remain on the higher tier (no mid-cycle refund) — acceptable.
2. **When the current period ends (`end_ns`):** downgrade should **complete automatically** without requiring a separate `lock` call, including:
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
- User still calls `lock` to stake for the new period.

Smaller change but still not fully “automatic subscription renewal.”

### Option C — Off-chain automation only

- Indexer + backend calls `lock` on behalf of users after `end_ns` (signed meta-tx / account abstraction).

No contract change; operational burden and trust model shift.

---

## Acceptance criteria (suggested)

- [ ] After `schedule_downgrade_subscription`, when `block_timestamp >= subscription.end_ns`, tier change can complete **without** the user calling `lock` manually (define exact method: on-chain keeper vs documented off-chain service).
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

- Review thread: scheduled downgrade applied only at manual `lock` renewal.  
- Test (happy path, manual renewal): `tests/subscription_lifecycle.rs` — `downgrade_applies_at_next_renewal`.
=======
If product requirements need state to change exactly at `apply_ns` without any user
or backend interaction, an off-chain keeper or scheduler transaction is still
required. NEAR contracts do not execute automatically at a timestamp.
>>>>>>> origin/feat/stake-dao
