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
}
```

Response:

```json
{
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
