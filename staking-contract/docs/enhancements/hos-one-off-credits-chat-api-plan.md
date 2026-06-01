<<<<<<< HEAD
# House-of-Stake `pay` for chat-api One-Off Credits

## Summary

Wire the staking-contract `pay` function into chat-api and the `private-assistant`
frontend for one-off credit purchases.

The contract owns the payment record. chat-api grants credits only after verifying
the on-chain `Purchase` by `purchase_id`.

Decisions:

- One-off credit purchases use `house_of_stake` only.
- Do not keep or add a `near_stake_lock` kind for this path.
- Credits are granted to the chat-api user credit balance, not to an organization
  credit balance.
- `1` purchased credit remains `1_000_000_000` nano-USD.

## Current State

### staking-contract

- `pay(price_id, product_id, quantity)` creates a direct NEAR one-off purchase
  without creating a stake lock.
- `get_purchase(purchase_id)` returns the stored `Purchase`.
- `get_price(price_id)` returns the catalog price, including `amount` and
  `price_type`.
- `withdraw_revenue(validator_id, amount)` lets the validator owner withdraw
  direct-payment revenue.

### chat-api

- `POST /v1/credits` currently follows a Stripe checkout model.
- The credit ledger is already suitable for idempotent purchases:
  `credit_transactions.reference_id` is unique for purchase transactions.
- NEAR RPC view helpers already exist for House-of-Stake subscription sync and
  can be extended for purchase verification.

### private-assistant

- Billing quick packs and custom amount call `useCreateCreditCheckout`.
- The mutation currently expects `checkout_url`.
- `src/lib/houseOfStake.ts` already connects a NEAR wallet, sends staking
  contract function calls, and syncs subscription state after wallet actions.

## Backend API Plan

### Create Credit Purchase Intent

Change `POST /v1/credits` from returning a Stripe checkout URL to returning a
House-of-Stake payment intent.

Request stays:

```json
{
  "credits": 100,
  "success_url": "https://app.example/success",
  "cancel_url": "https://app.example/cancel"
=======
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
>>>>>>> origin/feat/stake-dao
}
```

Response:

```json
{
<<<<<<< HEAD
  "kind": "house_of_stake",
  "price_id": "price_hos_credits",
  "network_id": "mainnet",
  "contract_id": "stake.dao"
}
```

Behavior:

- Validate `credits` with the existing min/max credit purchase rules.
- Require a linked NEAR account for the authenticated user.
- Require configured NEAR RPC URL, staking contract id, network id, and HoS
  one-off credits price id.
- Resolve the configured HoS price id from `credits.providers["house-of-stake"]`
  or the equivalent existing system config provider shape.
- Return only a signing intent. Do not grant credits yet.

### Confirm Credit Purchase

Add:

```text
POST /v1/credits/confirm
```

Request:

```json
{
  "purchase_id": "pay_...",
  "expected_credits": 100
}
```

Response should be the refreshed `CreditsSummary` so the frontend can update
immediately.

Verification:

- Fetch `get_purchase(purchase_id)` from the configured staking contract.
- Fetch `get_price(configured_price_id)`.
- Require `purchase.account_id` equals the authenticated user's linked NEAR
  account.
- Require `purchase.price_id` equals the configured HoS one-off credits price id.
- Require `purchase.quantity == expected_credits`.
- Require `purchase.amount_paid == price.amount * purchase.quantity`.
- Require the price is active and one-off when those fields are present in
  `get_price`.

Crediting:

- Convert to nano-USD: `amount_nano_usd = expected_credits * 1_000_000_000`.
- Use `purchase_id` as `credit_transactions.reference_id`.
- Call `try_record_purchase` before adding credits.
- If `try_record_purchase` returns duplicate, do not add credits again; return
  success with the current credit summary.
- Invalidate the in-memory credit-limit cache for the user after a new credit
  grant.

## Frontend Plan (`private-assistant`)

### API Types and Mutation

Update the credits API response type to:

```ts
type CreateCreditCheckoutResponse = {
  kind: "house_of_stake";
  price_id: string;
  network_id: string;
  contract_id: string;
};
```

Update `useCreateCreditCheckout`:

- Call `subscriptionsClient.createCreditCheckout` as today.
- When `kind === "house_of_stake"`, call a new House-of-Stake one-off payment
  helper instead of redirecting.
- On success, invalidate the credits summary query.
- On failure, keep the dialog usable and show a payment-specific error.

### House-of-Stake Payment Helper

Extend `src/lib/houseOfStake.ts` with:

```ts
payHouseOfStakeCredits(priceId: string, credits: number, networkId?: string)
```

Behavior:

- Fetch `get_price(price_id)` through the existing NEAR RPC view helper.
- Compute `deposit = price.amount * credits`.
- Sign:

```ts
pay({
  price_id: priceId,
  product_id: null,
  quantity: credits,
})
```

with the computed deposit.

- Extract `purchase_id` from the wallet transaction outcome.
- Call `POST /v1/credits/confirm` with `{ purchase_id, expected_credits:
  credits }`.
- Return the refreshed credits summary.

If the wallet library does not expose the method return value reliably, add a
small follow-up lookup path based on the signer account and recent
`get_purchases_for_account` results. The first implementation should attempt
direct transaction outcome decoding.

### Billing UI

- Keep the existing quick packs and custom amount dialog.
- Keep integer credit quantities only.
- Replace Stripe-specific loading copy such as "redirecting" with wallet/payment
  copy for the HoS flow.
- Do not add a second provider selector; this flow is HoS-only.

## Tests

### chat-api

Add or update tests for:

- `POST /v1/credits` returns a HoS intent with `kind`, `price_id`,
  `network_id`, and `contract_id`.
- Missing HoS credit config returns 503.
- Missing linked NEAR account returns the existing HoS/NEAR wallet error shape.
- Invalid credit quantity still returns 400.
- `POST /v1/credits/confirm` grants credits for a valid mocked purchase and
  price.
- Duplicate `purchase_id` is idempotent and does not double-credit.
- Confirm rejects mismatched account, price id, quantity, amount, missing
  purchase, and non-one-off or inactive price.

### private-assistant

Run:

```sh
pnpm run typecheck
```

Add focused tests where existing test patterns allow:

- HoS credit intent signs `pay` instead of redirecting.
- Successful confirmation invalidates credits summary and closes the custom
  amount dialog.
- Wallet signing failure or confirmation failure shows an error and does not
  mark credits as added.

## Acceptance Criteria

- One-off credit purchases never return `near_stake_lock`.
- `POST /v1/credits` returns a House-of-Stake `pay` intent.
- The frontend signs staking-contract `pay` with exact attached NEAR.
- chat-api grants user credits only after verifying `get_purchase`.
- Replaying the same `purchase_id` is safe and does not double-credit.
- A purchase from a different NEAR account is rejected.
- A purchase for the wrong price id, quantity, or amount is rejected.
=======
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
>>>>>>> origin/feat/stake-dao
