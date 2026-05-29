# Direct `pay` for One-Off Prices

## Summary

Add a direct-payment path for one-off catalog prices. The new `pay` method lets a user transfer NEAR directly to the staking contract for a one-off product purchase, without creating a stake lock.

This is separate from `lock`:

- `lock` remains the stake-backed House-of-Stake purchase/subscription path.
- `pay` becomes the direct NEAR payment path for one-off prices such as chat-api credit packs.

The contract will store Stripe-like purchase records so downstream services can verify exact purchases by `purchase_id`.

## Contract Changes

### Purchase Types

Add:

```rust
pub type PurchaseId = String;

pub struct Purchase {
    pub purchase_id: PurchaseId,
    pub account_id: AccountId,
    pub product_id: ProductId,
    pub price_id: PriceId,
    pub quantity: U64,
    pub amount_paid: NearToken,
    pub created_ns: U64,
}
```

Add a versioned wrapper:

```rust
pub enum VPurchase {
    V0(Purchase),
}
```

Add an id helper:

```rust
ids::next_purchase_id(...) -> "pay_*"
```

### Contract Storage

Add these storage fields:

```rust
pub purchases: LookupMap<PurchaseId, VPurchase>,
pub purchase_ids: Vector<PurchaseId>,
pub purchases_by_account: LookupMap<AccountId, Vec<PurchaseId>>,
pub purchases_by_product: LookupMap<ProductId, Vec<PurchaseId>>,
pub user_purchase_count: LookupMap<AccountId, u32>,
pub revenue_by_validator: LookupMap<ValidatorId, NearToken>,
pub revenue_by_product: LookupMap<ProductId, NearToken>,
```

Add corresponding `StorageKeys`.

### Storage Deposit Accounting

Add:

```rust
pub per_purchase_storage_stake: NearToken
```

to `Config`.

The staking contract is not deployed yet, so no state migration or `VConfig::V1` upgrade path is needed for this change. Update the initial `Config` shape directly and adjust tests/deploy config defaults accordingly.

Update storage helpers so required prepaid storage includes:

```text
min_storage_deposit
+ per_lock_storage_stake * user_lock_count
+ per_purchase_storage_stake * user_purchase_count
```

Before creating a purchase, require prepaid storage for one more purchase.

### Public Method

Add:

```rust
#[payable]
pub fn pay(
    &mut self,
    price_id: Option<PriceId>,
    product_id: Option<ProductId>,
    quantity: U64,
) -> PurchaseId
```

Behavior:

- Reject when paused.
- Resolve exactly one of `price_id` or `product_id`, matching `lock` behavior.
- If `product_id` is used, resolve through `Product.default_price_id`.
- Require an active product and active one-off price.
- Require `PriceType::OneOff`.
- Require `price.billing_period.is_none()`.
- Require `quantity > 0`.
- Require attached deposit exactly equals `price.amount * quantity`.
- Require caller has prepaid storage for one additional purchase.
- Do not stake, mint shares, create locks, or touch validator balances.
- Store a `Purchase`.
- Append indexes for global, account, and product purchase lookup.
- Increment `user_purchase_count`.
- Increment `Product.usage_count` and `Price.usage_count`.
- Increase withdrawable direct-payment revenue for `product.validator_id` and `product_id`.
- Return `purchase_id`.

### Revenue Withdrawal

Direct payments stay in contract escrow until withdrawn by the validator owner.

Add:

```rust
#[payable]
pub fn withdraw_revenue(
    &mut self,
    validator_id: ValidatorId,
    amount: Option<NearToken>,
) -> Promise
```

Behavior:

- Attach exactly 1 yocto.
- Reject when paused.
- Require `amount > 0` when provided.
- Resolve authorization through the existing validator-owner pattern: call the staking pool `get_owner_id()` for `validator_id`, then verify the callback `pool_owner == expected_caller`.
- If `amount` is `None`, withdraw the full `revenue_by_validator[validator_id]` balance.
- Require the requested amount does not exceed the validator revenue balance.
- Decrement `revenue_by_validator[validator_id]` before transferring funds.
- Transfer withdrawn NEAR to the verified validator owner account.
- Do not modify purchase records or usage counts.

Add private callback:

```rust
pub fn withdraw_revenue_after_get_owner(
    &mut self,
    #[callback] pool_owner: AccountId,
    validator_id: ValidatorId,
    amount: Option<NearToken>,
    expected_caller: AccountId,
) -> Promise
```

Add views:

```rust
pub fn get_revenue_balance_for_validator(&self, validator_id: ValidatorId) -> NearToken
pub fn get_revenue_balance_for_product(&self, product_id: ProductId) -> NearToken
```

Keep `revenue_by_product` as an accounting view only in v1. Withdrawal is validator-level so the verified validator owner can withdraw aggregate revenue for all products attached to that validator.

### Views

Add:

```rust
pub fn get_purchase(&self, purchase_id: PurchaseId) -> Option<Purchase>
pub fn get_purchases_for_account(
    &self,
    account_id: AccountId,
    from_index: u64,
    limit: u64,
) -> Vec<Purchase>
pub fn get_purchases_for_product(
    &self,
    product_id: ProductId,
    from_index: u64,
    limit: u64,
) -> Vec<Purchase>
```

### Event

Add an event:

```text
payment_create
```

Payload fields:

- `purchase_id`
- `account_id`
- `product_id`
- `price_id`
- `quantity`
- `amount_paid`

Add another event:

```text
revenue_withdraw
```

Payload fields:

- `validator_id`
- `account_id`
- `amount`

## Integration Behavior

chat-api should use `pay` for direct House-of-Stake one-off credits:

1. Frontend signs `pay(price_id, null, quantity)` with attached NEAR.
2. Contract returns `purchase_id`.
3. chat-api verifies `get_purchase(purchase_id)`.
4. chat-api grants credits only when:
   - `account_id` matches the linked NEAR account.
   - `price_id` matches the configured one-off credit price.
   - `quantity` matches the expected credit count.
   - `amount_paid == price.amount * quantity`.
5. chat-api records the purchase idempotently using `purchase_id` as the external reference.

Validator owners can withdraw accumulated direct-payment revenue from the contract with `withdraw_revenue`.

## Tests

Add contract tests for:

- Happy path stores a purchase, returns `pay_*`, emits `payment_create`, increments usage counts, and creates no lock.
- `product_id` resolution through `default_price_id`.
- Rejects both `price_id` and `product_id`.
- Rejects neither `price_id` nor `product_id`.
- Rejects recurring prices.
- Rejects archived prices and products.
- Rejects zero quantity.
- Rejects insufficient deposit.
- Rejects excess deposit.
- Rejects paused contract.
- Rejects missing prepaid storage.
- Required storage increases after each purchase.
- `get_purchase` returns the stored purchase.
- Account and product purchase list views paginate correctly.
- `pay` increases validator and product revenue balances.
- `withdraw_revenue` rejects non-owners through the callback authorization path.
- `withdraw_revenue` transfers full balance when `amount` is omitted.
- `withdraw_revenue` transfers partial balance when `amount` is provided.
- `withdraw_revenue` rejects zero or over-balance amounts.
- Revenue withdrawal emits `revenue_withdraw` and leaves purchase records unchanged.
- Existing `lock` one-off and subscription tests continue to pass.

## Assumptions

- Direct `pay` means exact NEAR payment, not duration-weighted staking.
- Purchase records are required on-chain; events alone are not enough.
- Multiple units are supported with `quantity`.
- Direct payments accrue to the product's `validator_id`.
- Validator owner withdrawal uses the same cross-contract pool-owner authorization pattern as catalog administration.
