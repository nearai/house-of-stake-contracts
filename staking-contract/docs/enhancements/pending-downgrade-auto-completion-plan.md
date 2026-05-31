# Pending Downgrade Auto-Completion

## Summary

Replace the current renewal-by-`lock` downgrade flow with a timestamped pending
downgrade model.

A scheduled downgrade records when it becomes effective with
`pending_downgrade_apply_ns = subscription.end_ns`. Once
`block_timestamp >= pending_downgrade_apply_ns`, subscription views project the
downgrade as completed without requiring the subscriber to call `lock`.
Actual stake cleanup remains lazy and runs on the next mutating transaction
that touches the subscription or its lock, because on-chain state cannot mutate
from time passing alone.

## Key Changes

### Subscription State

Extend `Subscription` with:

```rust
pub pending_downgrade_apply_ns: Option<U64>,
```

Keep the existing pending target fields:

```rust
pub pending_downgrade_price_id: Option<PriceId>,
pub pending_downgrade_target_amount: Option<NearToken>,
```

### Scheduling Downgrades

When `update_subscription` schedules a stake decrease:

- First normalize/apply any already-due pending downgrade.
- Clear any previous pending downgrade fields before writing a new downgrade.
- Set `pending_downgrade_apply_ns` to the current billing period end.
- Validate target product conflicts at schedule time so an impossible
  cross-product downgrade is rejected immediately.
- Return the same scheduled outcome shape, with `target_price_id` and
  `target_amount`.

Immediate updates must also clear stale pending downgrade state:

- Stake increase.
- Same-stake price/product change.
- No-op against the current active plan.

### View Projection

All subscription views must return the projected subscription:

- `get_subscription`
- `get_subscription_for_product`
- `get_subscription_for_price`
- `get_subscriptions_for_account`

Projection rule:

- If `pending_downgrade_apply_ns` is absent or is in the future, return the current
  projected billing window.
- If `pending_downgrade_apply_ns <= block_timestamp`, return the subscription as if
  the downgrade completed at `pending_downgrade_apply_ns`.
- The returned view has the target `product_id` and `price_id`, clears pending
  downgrade fields, and advances the billing window from the effective
  timestamp to the current virtual period.

Projection must not mutate contract state, move indexes, or queue unstake.

### Lazy Mutation Cleanup

Add an internal helper such as:

```rust
apply_due_subscription_downgrade(subscription_id: &SubscriptionId)
```

When the pending downgrade is due, the helper:

- Loads the stored subscription and target price/product.
- Clears pending downgrade fields.
- Updates stored `product_id` and `price_id`.
- Moves the `(account, product)` subscription index.
- Reduces the linked active subscription lock to
  `pending_downgrade_target_amount`.
- Queues surplus NEAR through the existing internal unstake path.
- Relabels/extends the existing subscription lock for the projected active
  billing period.
- Emits downgrade/update events.

Call this helper before mutating subscription flows:

- `update_subscription`
- `cancel_subscription`
- `resume_subscription`
- subscription renewal logic in `lock`
- subscription-lock `unlock` checks

The helper must be idempotent: if there is no pending downgrade or it is not
due, it returns without state changes.

### Recurring `lock` Renewal

Remove the legacy manual-downgrade renewal path from `lock`:

- Do not find subscriptions by `pending_downgrade_price_id`.
- Do not accept `price_id == pending_downgrade_price_id` as a special renewal.
- Do not require callers to attach the pending target amount to complete a
  scheduled downgrade.
- Same-tier renewals still use `lock` and must match the current effective
  subscription `price_id` and active lock amount.
- If `lock` touches a subscription whose pending downgrade is due, first apply
  the lazy downgrade cleanup, then validate against the current effective tier.

### Unlock Guard

If a lock belongs to an active subscription, `unlock` must check the projected
subscription state before allowing unlock.

- Reject unlock when the subscription is still active and not cancelled, even
  if the stored lock `end_ns` is in the past because lazy cleanup has not run.
- Continue allowing unlock after `cancel_at_period_end` once the final period
  has ended.

## Test Plan

- Scheduling a downgrade records `pending_downgrade_apply_ns == current end_ns`.
- Scheduling a second downgrade clears and replaces the previous pending
  downgrade.
- If the previous pending downgrade is already due, scheduling first applies it,
  then schedules from the new current period.
- Subscription views show the target price/product after
  `pending_downgrade_apply_ns` without a manual `lock`.
- Recurring `lock` no longer accepts the pending downgrade tier as a special
  manual completion path.
- Same-tier recurring renewal continues to require `lock` with the current
  effective price and matching NEAR amount.
- First later mutation applies real cleanup: clears pending fields, moves the
  product index, updates lock amount/order/end, and queues surplus unstake.
- Cross-product pending downgrade rejects immediately if the account already
  has a subscription for the target product.
- Immediate upgrade or same-amount plan change clears stale pending downgrade
  fields.
- `unlock` rejects active subscription locks after projected renewal, and still
  allows unlock after cancel-at-period-end.

## Assumptions

- "Automatic" means time-based view projection plus lazy mutation on the next
  transaction; no background transaction runs by itself on NEAR.
- The existing subscription lock is reused across projected periods instead of
  requiring a new `lock` call for downgrade completion.
- Surplus unstake is queued only once a mutating transaction applies the due
  downgrade.
