# House-of-Stake One-Off Credits for chat-api

## Summary

Move one-off credit purchases for chat-api agent hosting from a Stripe-only checkout path to a provider-aware flow that can use House of Stake.

The existing subscription flow already uses `house_of_stake` intents for recurring plans. One-off credits need a similar split:

- Stripe remains a checkout redirect when the credits provider is `stripe`.
- House of Stake returns an on-chain one-off lock intent when the credits provider is `house-of-stake`.
- chat-api credits the user's purchased balance only after it verifies the resulting on-chain lock.

The contract already has the required primitive for one-off purchases: `lock(price_id, product_id, duration_ns)`.

## Current State

### chat-api

- `POST /v1/credits` accepts `{ credits, success_url, cancel_url }`.
- The service requires Stripe to be configured before any credit purchase can proceed.
- `credits.default_provider` exists in system config, but only `stripe` is supported.
- Stripe webhook handling credits the user's purchased balance through `credit_transactions` and `user_credits`.
- Idempotency is provided by a unique purchase reference for Stripe checkout sessions.

### Frontend

- Billing quick packs and custom amount call `useCreateCreditCheckout`.
- The mutation assumes the response contains `checkout_url`.
- On success, the browser redirects to Stripe checkout.
- There is no House-of-Stake path for one-off credits.

### staking-contract

- `lock(price_id, product_id, duration_ns)` supports one-off product purchases.
- It resolves `price_id` or `product_id`, requires `PriceType::OneOff`, validates lock duration bounds, and validates the attached deposit using `check_near_price_lock`.
- `get_lock(lock_id)` exposes the lock record needed for backend verification.

## Proposed Provider Contract

Keep `credits` config provider-based:

```json
{
  "credits": {
    "default_provider": "house-of-stake",
    "providers": {
      "house-of-stake": {
        "price_id": "price_hos_credits"
      },
      "stripe": {
        "price_id": "price_stripe_credits"
      }
    }
  }
}
```

For House of Stake, `price_id` must point to a one-off catalog price.

Recommended catalog shape:

- `Price.price_type = OneOff`
- `Price.amount = one credit unit` in NEAR-denominated catalog terms
- `Price.lock_factor_near_months` defines the lock economics
- `Product.default_price_id` may be set, but chat-api can use `price_id` directly

## Backend API Changes

### Create Credit Purchase

Change `POST /v1/credits` response from Stripe-only:

```json
{ "checkout_url": "https://checkout.stripe.com/..." }
```

to a provider-aware union:

```json
{ "checkout_url": "https://checkout.stripe.com/..." }
```

or:

```json
{
  "kind": "house_of_stake",
  "price_id": "price_hos_credits",
  "credits": 100,
  "duration_ns": "31536000000000000",
  "network_id": "mainnet"
}
```

For `house-of-stake`, chat-api should:

- Validate the requested credit count using the same min/max rules as Stripe.
- Require a linked NEAR account.
- Require `NEAR_STAKING_CONTRACT_ID`.
- Resolve the configured HoS credit `price_id`.
- Return a lock duration. The safest first version is the contract `get_config().max_lock_duration_ns`.

Do not credit the user at this step. This step only creates a client-side signing intent.

### Sync Credit Purchase

Add an authenticated endpoint:

```text
POST /v1/credits/near/sync
```

Request:

```json
{
  "lock_id": "lock_...",
  "credits": 100
}
```

Response:

```json
{
  "credited": true,
  "balance": 100000000000
}
```

Backend verification must check:

- The lock exists on the configured staking contract.
- `lock.account_id` matches the user's linked NEAR account.
- `lock.status` is `Active`.
- `lock.order` is `ProductPurchase`.
- `lock.order.price_id` matches the configured HoS credit price id.
- The lock duration and amount satisfy the expected purchase quantity.
- The same `lock_id` has not already credited the account.

Use the existing credit ledger:

- `credit_transactions.type = purchase`
- `credit_transactions.reference_id = hos:{lock_id}`
- `user_credits.total_nano_usd += credits * 1_000_000_000`

This keeps credit grants backend-authoritative and idempotent.

## Frontend Changes

Update `useCreateCreditCheckout`:

- If the response has `checkout_url`, keep the current redirect behavior.
- If `kind === "house_of_stake"`, sign `lock`.
- Pass `{ price_id, product_id: null, duration_ns }`.
- Attach the required deposit for the requested credit count.
- Extract `lock_id` from the transaction outcome.
- Call `POST /v1/credits/near/sync`.
- Invalidate credits and billing queries after sync.

Add a helper in `houseOfStake.ts`:

- `lockHouseOfStakeCreditPurchase(priceId, credits, lockDurationNs, networkId)`
- It should fetch `get_price(price_id)`.
- It should compute or request the deposit required for `credits`.
- It should return the created `lock_id`.

The Billing UI can keep the current quick packs and custom amount. Only the mutation behavior changes.

## Open Design Decision

There are two possible ways to decide the attached NEAR deposit:

### Option A: Backend returns required deposit

`POST /v1/credits` returns:

```json
{
  "kind": "house_of_stake",
  "price_id": "price_hos_credits",
  "credits": 100,
  "duration_ns": "31536000000000000",
  "deposit": "..."
}
```

Pros:

- Frontend stays simple.
- Backend and sync validation use the same math.
- Fewer wallet failures from client-side calculation mistakes.

Cons:

- Backend must duplicate `check_near_price_lock` minimum-lock math.

### Option B: Frontend computes required deposit

Frontend fetches `get_price`, reads `amount` and `lock_factor_near_months`, then computes:

```text
required_near_months = price.amount * credits * lock_factor_near_months / LOCK_FACTOR_DENOM
deposit = ceil(required_near_months * AVG_MONTH_NS / duration_ns)
```

Pros:

- Backend API stays smaller.

Cons:

- Client must duplicate contract math.
- Backend still needs the same validation to prevent forged sync requests.

Recommendation: use Option A.

## Contract Impact

No contract changes are required for the first version.

The current contract already provides:

- `lock` for one-off purchases
- `get_price` for catalog validation
- `get_config` for lock duration bounds
- `get_lock` for sync verification

Potential future contract enhancement:

- Add a view helper that returns the minimum required lock deposit for `(price_id, quantity, duration_ns)`.
- This would remove duplicated pricing math from chat-api and frontend.

## Implementation Sequence

1. Extend chat-api credit purchase outcome to return either `checkout_url` or HoS lock intent.
2. Add backend helper to compute minimum required HoS lock deposit for a credit quantity.
3. Add `POST /v1/credits/near/sync` with on-chain lock verification and idempotent crediting.
4. Add backend tests for provider selection, invalid HoS locks, duplicate sync, and successful credit grant.
5. Update frontend credit purchase mutation to branch on `house_of_stake`.
6. Add frontend HoS one-off signing helper and query invalidation.
7. Update Billing copy only where it is Stripe-specific.
8. Run focused backend tests and frontend typecheck.

## Risks

- Wallet account mismatch: the signer may not be the linked NEAR account. Backend sync must reject this clearly.
- Transaction succeeded but sync failed: frontend should show a retryable sync error and keep the lock id when possible.
- Price configuration mismatch: HoS credit price must be a one-off price, not a recurring subscription price.
- Idempotency: repeated sync attempts must not double-credit.
- Lock math drift: duplicated pricing math must match contract `check_near_price_lock`.

## Acceptance Criteria

- Stripe credit purchases continue to work unchanged when `credits.default_provider = "stripe"`.
- HoS credit purchases sign `lock` when `credits.default_provider = "house-of-stake"`.
- chat-api credits the user only after verifying the on-chain lock.
- Replaying the same HoS lock sync does not double-credit.
- A lock from a different NEAR account is rejected.
- A lock for the wrong price id is rejected.
- Billing quick packs and custom credit amount work for the active provider.
