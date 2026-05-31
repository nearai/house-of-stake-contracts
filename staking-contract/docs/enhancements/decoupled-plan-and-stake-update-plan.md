# Decoupled Plan And Stake Subscription Updates

## Summary

`update_subscription(subscription_id, target_price_id, target_amount)` currently
treats the catalog price direction and the staked NEAR direction as the same
thing. That is too restrictive for variable-stake plans.

The target price describes the desired plan tier. The target amount describes
the desired stake. They can move in different directions:

- Plan upgrade with stake decrease.
- Plan downgrade with stake increase.
- Same plan with stake increase or decrease.
- Plan change with no stake delta.

The contract should validate and execute these as two independent dimensions:

- Plan tier change: derived from `target_price.amount` compared with the
  current price amount.
- Stake delta: derived from `target_amount` compared with the current
  subscription lock amount.

## Goals

- Support every valid combination of plan direction and stake direction.
- Keep stake increases immediate, because the user attaches the additional NEAR.
- Keep stake decreases at period end, because they release existing locked NEAR.
- Keep plan upgrades immediate when the immediate lock amount is valid for the
  target plan.
- Keep plan downgrades at period end.
- Preserve on-chain validation of the final target amount against the target
  price range.
- Make mixed-timing outcomes explicit for chat-api and frontend callers.

## Direction Model

Compute both directions independently.

```rust
let plan_direction = target_price.amount.cmp(&current_price.amount);
let stake_direction = target_amount.cmp(&current_lock.amount_near);
```

Interpretation:

- `plan_direction == Greater`: plan upgrade.
- `plan_direction == Less`: plan downgrade.
- `plan_direction == Equal`: same catalog tier amount. The price/product may
  still change.
- `stake_direction == Greater`: stake increase.
- `stake_direction == Less`: stake decrease.
- `stake_direction == Equal`: no stake delta.

Remove the current validation that requires both directions to match.

## Validation

Always validate the final desired state first:

- The caller owns `subscription_id`.
- The target price is active, recurring, and monthly.
- The target product validator matches the active subscription lock validator.
- The account does not already own another subscription for the target product.
- `target_amount` is within the target price range:
  `[target_price.amount, target_price.metadata.max_amount]`.
- The current billing period has not ended.
- Any already-due pending period-end update is applied before evaluating the new
  request.

Then validate the immediate/interim state:

- If stake increases immediately, attached deposit must equal
  `target_amount - current_lock.amount_near`.
- If no stake increase happens immediately, attached deposit must be exactly
  1 yocto.
- If the plan changes immediately while stake decrease is scheduled for period
  end, the current lock amount must also be valid under the immediate target
  price. If it is not valid, schedule the plan change together with the stake
  decrease instead of applying the plan immediately.
- If stake increases immediately while plan downgrade is scheduled for period
  end, the increased lock amount must be valid under the current effective price
  for the interim period and valid under the target price for the final period.

## Timing Matrix

| Plan direction | Stake direction | Immediate action | Period-end action |
| --- | --- | --- | --- |
| Same | Same | no-op or same-tier price/product change | none |
| Same | Increase | stake additional NEAR | none |
| Same | Decrease | none | reduce stake |
| Upgrade | Same | apply target plan | none |
| Upgrade | Increase | stake additional NEAR and apply target plan | none |
| Upgrade | Decrease | apply target plan if current stake is valid there | reduce stake |
| Downgrade | Same | none | apply target plan |
| Downgrade | Increase | stake additional NEAR | apply target plan |
| Downgrade | Decrease | none | apply target plan and reduce stake |

For "same" plan direction with a different price/product ID, treat the plan
change as immediate unless a stake decrease requires scheduling and the
immediate target price cannot support the current stake amount.

## State Model

The current pending fields are downgrade-specific:

```rust
pending_downgrade_price_id: Option<PriceId>,
pending_downgrade_target_amount: Option<NearToken>,
pending_downgrade_apply_ns: Option<U64>,
```

Rename them to generalized period-end update state:

```rust
pub pending_update: Option<PendingSubscriptionUpdate>,

pub struct PendingSubscriptionUpdate {
    pub target_price_id: Option<PriceId>,
    pub target_amount: Option<NearToken>,
    pub apply_ns: U64,
}
```

Semantics:

- `target_price_id = Some(...)` means apply a deferred plan change at
  `apply_ns`.
- `target_amount = Some(...)` means reduce the active subscription lock to that
  amount at `apply_ns`.
- Both fields can be present for a plan downgrade plus stake decrease.
- Only `target_price_id` can be present for a deferred plan downgrade with no
  stake decrease.
- Only `target_amount` can be present for a stake decrease when the plan either
  stays the same or was already upgraded immediately.

Because the contract has not launched, no migration is required. The rename is
preferred over preserving `pending_downgrade_*` because the pending state now
represents deferred plan changes as well as deferred stake decreases.

## Execution Plan

### 1. Normalize Existing Pending Work

At the start of `update_subscription`, call the pending apply helper for the
current `subscription_id`.

If an existing period-end update is due:

- Apply the deferred price/product change if present.
- Apply the deferred stake reduction if present.
- Move the account/product subscription index if the product changes.
- Sync the active subscription lock window/order.
- Clear pending update state.

If an existing period-end update is not due, replace it only after validating
the new request. A new scheduled update should clear the old pending update and
write the new one.

### 2. Build An Update Decision

Add an internal decision struct, for example:

```rust
struct SubscriptionUpdateDecision {
    immediate_price_id: Option<PriceId>,
    immediate_stake_increase: Option<NearToken>,
    pending_price_id: Option<PriceId>,
    pending_target_amount: Option<NearToken>,
}
```

This separates "what changes now" from "what changes at period end".

### 3. Apply Immediate Changes

Immediate plan changes:

- Clear stale pending state only after the new pending decision is ready.
- Update `sub.product_id` and `sub.price_id`.
- Move the `(account, product)` index if the product changes.
- Update the active lock `OrderRef::Subscription.price_id`.
- Increment target price/product usage counts.

Immediate stake increases:

- Require exact attached deposit.
- Run the existing validator settlement pipeline.
- Call the existing internal stake flow.
- Increase lock amount and shares.
- Keep the billing window unchanged.

If both immediate plan and stake changes happen, they should commit in the same
callback after settlement.

### 4. Store Period-End Changes

If either a deferred plan change or deferred stake decrease is required:

- Set `pending_update.apply_ns = sub.end_ns`.
- Store `pending_update.target_price_id` when the plan change is deferred.
- Store `pending_update.target_amount` when stake decrease is deferred.
- Do not mutate the current lock amount for stake decreases until apply time.
- Do not move the subscription product index until the deferred plan change is
  applied.
- Do not increment target price/product `usage_count` for deferred plan changes
  until the pending update becomes effective.

Views should project due period-end updates without requiring a manual `lock`.

### 5. Apply Period-End Changes

Generalize `apply_due_subscription_downgrade` to
`apply_due_subscription_update`.

When due:

- If `target_amount` is present and lower than the active lock amount, queue
  surplus unstake through the existing internal unstake path.
- If `target_price_id` is present, update `sub.product_id` and `sub.price_id`.
- Move the account/product index only when product changes.
- Increment target price/product `usage_count` only when a deferred plan change
  becomes effective.
- Advance the subscription and lock billing window to the projected current
  period.
- Clear `pending_update`.
- Emit events for the plan update and stake reduction.

The helper must be idempotent.

## Outcome Shape

The current outcome kind is not expressive enough for mixed timing. Expose
expanded outcome fields so clients can render immediate and period-end effects
without inferring from the inputs.

Use:

```rust
pub struct SubscriptionPlanChangeOutcome {
    pub kind: String,
    pub subscription_id: SubscriptionId,
    pub target_price_id: PriceId,
    pub target_amount: U128,
    pub lock_id: Option<LockId>,
    pub immediate_plan_change: bool,
    pub immediate_stake_increase: Option<U128>,
    pub pending_plan_change: bool,
    pub pending_stake_decrease: Option<U128>,
    pub pending_apply_ns: Option<U64>,
}
```

This lets chat-api/frontend explain mixed cases such as "plan upgraded now,
stake decrease scheduled for period end".

## View Projection

Update subscription views to project `pending_update`:

- If pending update is absent or not due, project only the active billing
  window.
- If pending update is due, return the subscription as if the deferred
  price/product and/or stake amount have applied.
- Do not mutate indexes or queue unstake from a view.
- If only stake amount changes, the projected subscription price/product stays
  unchanged.
- If only plan changes, the projected subscription amount is read from the
  current lock amount.

## Client Behavior

chat-api and frontend should keep sending the final desired pair:

```json
{
  "subscription_id": "...",
  "target_price_id": "...",
  "target_amount": "..."
}
```

They should not infer timing from only `target_amount` or only
`target_price_id`. The contract response should drive user messaging.

Examples:

- Plan upgrade + stake decrease:
  - "Your plan changes now. Your stake decrease applies at period end."
- Plan downgrade + stake increase:
  - "Your additional stake is locked now. Your plan changes at period end."
- Plan downgrade + stake decrease:
  - "Your plan and stake decrease apply at period end."

## Test Plan

Add focused host tests for the full matrix:

- Same plan, stake increase applies immediately.
- Same plan, stake decrease schedules period-end amount update.
- Plan upgrade, same stake applies plan immediately.
- Plan upgrade, stake increase applies both immediately.
- Plan upgrade, stake decrease applies plan immediately and schedules stake
  decrease when current stake is valid under target price.
- Plan upgrade, stake decrease schedules both when current stake is not valid
  under target price.
- Plan downgrade, same stake schedules plan change.
- Plan downgrade, stake increase stakes immediately and schedules plan change.
- Plan downgrade, stake decrease schedules both plan and stake changes.

Add validation tests:

- Reject final `target_amount` outside target price range.
- Reject immediate stake increase if deposit does not equal the delta.
- Reject mixed update if interim stake amount is invalid under the interim
  effective price.
- Reject archive/delete of a price or product referenced by a pending update
  even though deferred target usage counts have not incremented yet.
- Replacing an existing pending update clears the previous pending price and
  amount.
- Applying a due pending update is idempotent.

Add view tests:

- Due pending plan-only update projects target product/price.
- Due pending amount-only update projects the same product/price and target
  amount semantics.
- Due pending plan+amount update projects both.

Add sandbox coverage for at least one mixed-timing case:

- Plan upgrade + stake decrease.
- Plan downgrade + stake increase.

## Usage Count Decision

Usage counts increment when a plan becomes effective, not when it is scheduled.

- Immediate plan changes increment immediately.
- Deferred plan changes increment in `apply_due_subscription_update`.
- Replaced pending updates require no usage-count adjustment.
- Archive/delete must reject any price or product referenced by pending update
  state, so delayed counting cannot leave dangling pending targets.
