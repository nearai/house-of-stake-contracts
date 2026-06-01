# Chat API Credits Multi-Provider Enhancement

## Summary

`chat-api` should support both Stripe and House-of-Stake credit purchases from
one `credits` system config. The config model already allows multiple provider
entries, but the current credit checkout flow only returns a House-of-Stake
payment intent and rejects other defaults.

## Current Testnet Catalog

- Network: `testnet`
- Staking contract: `hos-e2e-0601144939.testnet`
- HoS credit product: `prod_37o5G0rr2wMJ5C`
- HoS credit price: `price_z2EbTifr7Nyqwt6v5kFqSiUb`
- HoS credit unit price: `0.4 NEAR`

## Target Config Shape

The existing `credits.providers` shape can hold both providers:

```json
{
  "credits": {
    "default_provider": "house-of-stake",
    "providers": {
      "stripe": {
        "price_id": "price_stripe_credits"
      },
      "house-of-stake": {
        "price_id": "price_z2EbTifr7Nyqwt6v5kFqSiUb"
      }
    }
  }
}
```

`default_provider` controls the provider used when the client does not pass an
explicit provider.

## Required API Change

Add an optional provider selector to `POST /v1/credits`:

```json
{
  "credits": 10,
  "provider": "house-of-stake",
  "success_url": "http://localhost:3000/billing?credits=success",
  "cancel_url": "http://localhost:3000/billing?credits=cancel"
}
```

Provider resolution:

```text
request.provider ?? credits.default_provider ?? "stripe"
```

Supported providers:

- `house-of-stake`
- `stripe`

Unknown providers should return `400 InvalidProvider`.

## Response Shapes

### House of Stake

Keep the existing wallet intent response:

```json
{
  "kind": "house_of_stake",
  "price_id": "price_z2EbTifr7Nyqwt6v5kFqSiUb",
  "network_id": "testnet",
  "contract_id": "hos-e2e-0601144939.testnet",
  "quantity": 10
}
```

The frontend signs `pay(price_id, product_id: null, quantity)` and then calls
`POST /v1/credits/confirm`.

### Stripe

Return a redirect response for Stripe:

```json
{
  "kind": "stripe",
  "checkout_url": "https://checkout.stripe.com/..."
}
```

Stripe confirmation continues through the existing webhook path. Do not call
`POST /v1/credits/confirm` for Stripe.

## Backend Implementation Notes

1. Extend `CreateCreditCheckoutRequest` with `provider: Option<String>`.
2. Extend `CreateCreditPurchaseOutcome` to represent both HoS and Stripe:
   - keep HoS fields for `kind = "house_of_stake"`
   - add `checkout_url` for `kind = "stripe"`
3. Split credit provider lookup:
   - `get_credits_provider_config(provider)`
   - `get_hos_credits_price_id()` can call the generic helper.
4. For `provider = "house-of-stake"`:
   - require `NEAR_STAKING_CONTRACT_ID`
   - require linked NEAR wallet
   - return the current HoS intent.
5. For `provider = "stripe"`:
   - require Stripe secret/config
   - use the configured Stripe credit `price_id`
   - return checkout URL.
6. Keep `confirm_credit_purchase` HoS-only.

## Frontend Implementation Notes

`private-assistant` currently assumes the credit checkout response is HoS.
Update it to branch on `kind`:

- `house_of_stake`: call `payHouseOfStakeCredits(...)`.
- `stripe`: redirect `window.location.href = checkout_url`.

Optionally add a UI provider selector when both providers are configured. If no
selector is shown, omit `provider` and let `chat-api` use `default_provider`.

## Tests

Backend tests:

- `POST /v1/credits` defaults to `credits.default_provider`.
- Request provider overrides default provider.
- Config with both `stripe` and `house-of-stake` providers is accepted.
- HoS response includes `price_id`, `network_id`, `contract_id`, and `quantity`.
- Stripe response includes `checkout_url`.
- Unsupported provider returns `400`.
- Stripe default does not require a linked NEAR wallet.
- HoS provider still requires a linked NEAR wallet.
- `POST /v1/credits/confirm` remains HoS-only and validates the configured HoS price.

Frontend tests:

- HoS credit checkout signs `pay` and calls confirm.
- Stripe credit checkout redirects to `checkout_url`.
- Unsupported response kind surfaces an error.

## Rollout Plan

1. Deploy backend change with both providers configured and `default_provider`
   left unchanged.
2. Deploy frontend response-branching change.
3. Set local/testnet default to `house-of-stake` for E2E testing.
4. Keep production default as `stripe` until HoS credit flow is approved for
   production.
5. After rollout, monitor credit checkout failures by provider.
