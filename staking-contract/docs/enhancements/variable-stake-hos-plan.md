# Variable-Stake House-of-Stake Subscription Plan

## Summary

Update the House-of-Stake subscription model to match the HackMD design:

- Each subscription plan has a valid NEAR stake range.
- Users choose an explicit target stake amount.
- One contract method changes the subscription plan.
- Stake increases apply immediately.
- Stake decreases are scheduled for the next billing period.
- chat-api derives HoS monthly credits from the synced on-chain stake amount.

## Contract Changes

### Price Metadata

Add an optional typed metadata field to `Price`:

```rust
pub struct Price {
    // existing fields...
    pub metadata: Option<PriceMetadata>,
}

pub struct PriceMetadata {
    pub max_amount: Option<U128>,
}
```

Use it for subscription stake ranges:

- Keep `Price.amount` as the minimum allowed stake for the plan.
- Store the optional upper bound at `Price.metadata.max_amount`.
- The external JSON API still stays ergonomic because `U128` serializes/deserializes as a string:
  ```json
  {
    "max_amount": "2000000000000000000000000000"
  }
  ```
- Missing `metadata` or missing `max_amount` means no upper bound.
- Validate every subscription target amount against `[amount, metadata.max_amount]`.

This keeps the plan range on-chain, so the contract can reject invalid stake amounts directly.
It also avoids adding subscription-specific fields directly to all price rows, including one-off prices that do not need them. Because the contract enforces `max_amount`, typed metadata is preferred over raw JSON for reliable validation.

### Unified Entrypoint

Replace the public split methods:

- `upgrade_subscription`
- `schedule_downgrade_subscription`

with one method:

```rust
update_subscription(
    subscription_id: SubscriptionId,
    target_price_id: PriceId,
    target_amount: U128,
) -> PromiseOrValue<SubscriptionPlanChangeOutcome>
```

The method decides behavior from `target_amount` compared to the current subscription lock amount.

### Outcome Type

Add a tagged JSON outcome:

```rust
SubscriptionPlanChangeOutcome {
    kind: String, // "changed_immediately" | "scheduled_for_period_end" | "no_op"
    subscription_id: SubscriptionId,
    target_price_id: PriceId,
    target_amount: U128,
    lock_id: Option<LockId>,
}
```

### Direction And Timing

Use these rules:

- `target_amount > current_lock.amount_near`
  - Require `target_price.amount > current_price.amount`.
  - Immediate change.
  - Attached deposit must equal `target_amount - current_lock.amount_near`.
  - Existing validator settlement pipeline still runs.
  - Return `kind = "changed_immediately"` and `lock_id`.

- `target_amount < current_lock.amount_near`
  - Require `target_price.amount < current_price.amount`.
  - Schedule for period end.
  - Attach exactly 1 yocto.
  - Store pending target price and target amount.
  - Return `kind = "scheduled_for_period_end"` and no `lock_id`.

- `target_amount == current_lock.amount_near` and `target_price_id != current price`
  - Require `target_price.amount == current_price.amount`.
  - Change plan immediately with no stake delta.
  - Attach exactly 1 yocto.
  - Return `kind = "changed_immediately"` and no `lock_id`.

- `target_amount == current_lock.amount_near` and `target_price_id == current price`
  - Attach exactly 1 yocto.
  - Return `kind = "no_op"`.

Reject equal catalog amounts with different target stake semantics only if the target amount is invalid for the target plan range.
Reject any update where the target stake amount direction does not match the target/current catalog price amount direction.

### Pending Decreases

Extend subscription state with:

```rust
pending_downgrade_target_amount: Option<NearToken or U128>
```

At renewal:

- Require `lock` to use the pending target price.
- Require the attached lock amount to match the pending target amount.
- Apply the pending price and target amount.
- Release surplus stake from the previous subscription lock.
- Clear pending downgrade fields.

### Subscription Selection

Require `subscription_id` for every `update_subscription` call:

- Load the subscription by `subscription_id`.
- Verify the caller owns the subscription.
- Support same-product and cross-product updates through the same path.
- Do not infer the subscription from `(account, target_product_id)`.

Keep:

- `account -> subscription_ids` mapping.
- `get_subscriptions_for_account(account_id, from_index, limit)`.

### Out Of Scope

No migration is needed because the staking contract has not launched.

One-off credit purchase remains on `lock` and is not changed by this plan.

## chat-api Changes

### Plan Config

Extend HoS plan configuration with stake range and credit policy:

```json
{
  "providers": {
    "house-of-stake": {
      "price_id": "price_hos_basic"
    }
  },
  "house_of_stake": {
    "min_stake_yocto": "500000000000000000000000000",
    "max_stake_yocto": "2000000000000000000000000000",
    "fixed_monthly_credits_nano_usd": null,
    "credits_per_near_divisor": 100
  }
}
```

Starter plan:

- Fixed `$5` monthly credits.

Basic and Pro plans:

- Monthly credits = `staked_near / credits_per_near_divisor`.

### Wallet Intent

Replace split HoS change-plan outcomes:

- `NearStakingUpgrade`
- `NearStakingScheduleDowngrade`

with one wallet intent:

```rust
NearStakingChangePlan {
    subscription_id: String,
    target_price_id: String,
    target_amount: String,
    timing: "immediate" | "period_end" | "no_op",
}
```

The frontend should always call:

```rust
update_subscription({
    subscription_id,
    target_price_id,
    target_amount,
})
```

The contract remains authoritative for final validation.

### Sync And Credits

chat-api should sync actual HoS stake from chain:

- Fetch the HoS subscription.
- Read `last_lock_id`.
- Fetch the lock.
- Use `lock.amount_near` as the synced subscription stake amount.

Store the current HoS stake amount in subscription data so request-time quota checks do not need extra RPC calls.

Credit behavior:

- Starter: fixed `$5`.
- Basic/Pro: `staked_near / credits_per_near_divisor`.
- Current-period credit increases are prorated for immediate stake increases.
- Stake decreases affect next period only.

## Test Plan

### Contract Tests

Add or update host tests for:

- Initial `lock` accepts any amount within the plan range.
- Initial `lock` rejects amounts below min or above max.
- Immediate stake increase returns `changed_immediately` and a `lock_id`.
- Stake decrease returns `scheduled_for_period_end`.
- Scheduled decrease applies at renewal.
- Same stake amount with different target plan changes immediately.
- Same price and same amount returns `no_op`.
- Missing or non-owned `subscription_id` rejects.
- Same-product and cross-product updates both require `subscription_id`.
- `get_subscriptions_for_account` returns all owned subscriptions.

### chat-api Tests

Add or update tests for:

- HoS change-plan returns one `near_staking_change_plan` intent for both increases and decreases.
- Response includes `subscription_id`, `target_price_id`, `target_amount`, and `timing`.
- Dynamic HoS credits use synced stake amount and plan credit policy.
- Starter fixed credits remain `$5`.
- Basic and Pro credits are calculated from `staked_near / divisor`.

### Verification Commands

Run:

```bash
cargo check -p staking-contract
env NEAR_SANDBOX_BIN_PATH=/usr/bin/true cargo test -p staking-contract --test subscription_lifecycle
```

For chat-api, run focused subscription/change-plan tests after updating the API response shape.

## Assumptions

- The staking contract has not launched, so no migration is required.
- Stake decreases always take effect next billing period.
- Current-period credits only increase via prorated stake increases.
- Decreases do not reduce current-period credits.
- One-off credit purchase is out of scope for this change.
