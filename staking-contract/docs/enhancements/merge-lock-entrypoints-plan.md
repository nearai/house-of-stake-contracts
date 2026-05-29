# Merge Contract Lock Entrypoints

## Summary

Replace `lock` and `lock` with one public payable method:

```rust
pub fn lock(
    &mut self,
    price_id: Option<PriceId>,
    product_id: Option<ProductId>,
    duration_ns: Option<U64>,
) -> PromiseOrValue<LockId>
```

The method resolves `price_id` vs `product_id` using the existing XOR/default-price rules, then infers behavior from the resolved catalog `Price.price_type`.

## Key Changes

- Remove the old public methods `lock` and `lock`.
- Keep one internal implementation path that performs the shared preamble, gas check, active price/product lookup, validator checks, settlement pipeline, and `commit_catalog_lock`.
- For `PriceType::OneOff`:
  - Require `duration_ns: Some`.
  - Reject prices with `billing_period`.
  - Preserve current duration min/max validation and one-off price check.
  - Create `OrderRef::ProductPurchase`.
  - Do not update subscription state.
- For recurring subscription prices:
  - Require `duration_ns: None`.
  - Require recurring monthly price using existing `require_recurring_monthly_price`.
  - Preserve existing subscription creation, renewal, cancellation-at-period-end reset, pending downgrade, target amount, and current amount validations.
  - Derive duration from `subscription.end_ns - now`.
  - Create `OrderRef::Subscription` and persist subscription follow-up as today.
- Update error messages to reference `lock`, not `lock` or `lock`.

## Public Interfaces And Clients

- Update contract docs/API docs to document only `lock`.
- Update sandbox helpers to expose one `buyer_lock(...)` helper, with thin test-only helpers only if they simplify test readability.
- Update chat-api HoS intent docs/comments to call `lock` instead of `lock`.
- Request payload from chat-api/client for subscription purchase should pass:
  - `price_id: Some(...)`
  - `product_id: None`
  - `duration_ns: None`
  - attached NEAR amount as before.

## Test Plan

- Update existing `lock_*` tests to call `lock(..., Some(duration))`.
- Update existing `lock_*` tests to call `lock(..., None)`.
- Add or preserve validation coverage for:
  - one-off price rejects missing `duration_ns`
  - recurring price rejects provided `duration_ns`
  - one-off product default price still resolves via `product_id`
  - recurring product default price still resolves via `product_id`
  - both `price_id` and `product_id` rejected
  - neither `price_id` nor `product_id` rejected
- Run:
  - `cargo test -p staking-contract`
  - HoS-related chat-api tests that assert lock intent method names or payloads.

## Assumptions

- Because the contract has not launched, remove the old ABI entrypoints instead of keeping deprecated wrappers.
- The unified method infers lock behavior from catalog price type, rather than accepting an explicit mode.
- One-off product purchases remain supported; only the public method name and duration argument shape change.
