# Pending subscription updates at billing period end

**Type:** Historical issue / resolved design note
**Component:** `staking-contract` — subscriptions (`subscriptions.rs`, `lock.rs`)
**Status:** Superseded by `update_subscription` + `pending_update`

---

## Summary

The old split downgrade flow required a subscriber to manually renew with `lock` after
`subscription.end_ns` before a scheduled downgrade could take effect. That model has
been replaced by `update_subscription(subscription_id, target_price_id, target_amount)`.

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
5. A later mutation lazily commits the pending update, clears `pending_update`,
   advances the billing window, and queues any stake decrease through the normal
   pending-unstake accounting.

There is still no autonomous on-chain cron. A transaction must touch the
subscription before storage changes are committed, but callers no longer need to
manually renew with the lower tier.

---

## Relevant code

| Step | Location |
|------|----------|
| Schedule/update plan or stake | `subscriptions.rs` — `update_subscription` |
| Pending update projection | `subscriptions.rs` — `project_subscription_view_now` |
| Lazy commit | `subscriptions.rs` — `apply_due_subscription_update` |
| Subscription lock/renewal | `lock.rs` — `lock_recurring_subscription_with_catalog` |

---

## Remaining operational note

If product requirements need state to change exactly at `apply_ns` without any user
or backend interaction, an off-chain keeper or scheduler transaction is still
required. NEAR contracts do not execute automatically at a timestamp.
