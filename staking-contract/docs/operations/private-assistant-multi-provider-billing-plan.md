# Private Assistant Multi-Provider Billing Plan

## Summary

Private Assistant should support both Stripe and House-of-Stake payment paths when the user has a linked NEAR wallet, and only Stripe when the user does not have a linked NEAR wallet.

The frontend currently uses `VITE_SUBSCRIPTION_PAYMENT_PROVIDER` as a global billing selector. That makes the whole UI behave as either Stripe-first or House-of-Stake-first. This should be replaced with runtime payment capability detection.

## Target Behavior

- Users with a linked NEAR account can choose either:
  - Stripe card checkout
  - House-of-Stake NEAR wallet staking checkout
- Users without a linked NEAR account can only use Stripe card checkout.
- Existing subscriptions continue through their original provider:
  - Stripe subscriptions use Stripe plan-change, cancel, resume, and portal flows.
  - House-of-Stake subscriptions use House-of-Stake staking transaction flows.
- Credits purchase follows the same rule:
  - NEAR-linked users may choose Stripe or House-of-Stake.
  - Non-NEAR users only see Stripe.

## Environment Variables

Recommended frontend environment:

```env
VITE_CHAT_API_URL=http://localhost:8080
VITE_NEAR_RPC_URL=https://test.rpc.fastnear.com
VITE_NEAR_NETWORK_ID=testnet
VITE_NEAR_STAKING_CONTRACT_ID=hos-e2e-0601144939.testnet
```

`VITE_SUBSCRIPTION_PAYMENT_PROVIDER` should not be used as the production billing selector. If kept, it should be documented as a development override only.

## Runtime Capability Logic

Add a small frontend helper or hook that derives billing capability from user data:

```ts
const hasNearAccount = user.linked_accounts?.some((account) => account.provider === "near") ?? false;
const canUseHouseOfStake = hasNearAccount && Boolean(NEAR_STAKING_CONTRACT_ID);
const canUseStripe = true;
```

This should be based on linked accounts rather than only the current login method. A Google or GitHub user who later links a NEAR account should be able to use House-of-Stake because they can sign wallet transactions.

If product policy requires "current session was NEAR login" instead of "NEAR account is linked", chat-api and the frontend need an explicit session auth-method field.

## Frontend Implementation Plan

1. Replace global provider selection.
   - Stop using `SUBSCRIPTION_PAYMENT_PROVIDER` as the source of truth in billing flows.
   - Keep provider selection local to the checkout action or active subscription.

2. Fetch provider catalogs explicitly.
   - Fetch Stripe plans by default.
   - Fetch House-of-Stake plans when `canUseHouseOfStake` is true.
   - Keep provider-specific `price_id` values separate, keyed by plan name and provider.

3. Add payment method selection.
   - Map UI method `card` to provider `stripe`.
   - Map UI method `near` to provider `house-of-stake`.
   - Hide or disable `near` when `canUseHouseOfStake` is false.

4. Update subscription checkout calls.
   - In `src/pages/BillingPage.tsx`, pass the selected provider to `createSubscription`.
   - In `src/pages/dashboard/ActivateView.tsx`, pass the selected provider to `createSubscription`.
   - Do not depend on a global provider env var.

5. Update plan changes.
   - For users with an active subscription, use `activeSubscription.provider` to choose the plan catalog and change-plan flow.
   - House-of-Stake subscriptions should continue using staking transaction outcomes.
   - Stripe subscriptions should continue using Stripe checkout or API outcomes.

6. Update credits purchase.
   - Add provider selection to the add-credits UI.
   - Pass `provider: "stripe"` or `provider: "house-of-stake"` into `useCreateCreditCheckout`.
   - Keep House-of-Stake unavailable when the user has no linked NEAR account.

7. Update frontend docs.
   - Remove `VITE_SUBSCRIPTION_PAYMENT_PROVIDER` from normal setup instructions.
   - Document required NEAR variables as enabling House-of-Stake capability, not selecting it globally.

## Backend Assumptions

chat-api should have system config entries for both providers under the same plan names:

```json
{
  "subscription_plans": {
    "starter": {
      "providers": {
        "stripe": { "price_id": "price_stripe_starter" },
        "house-of-stake": { "price_id": "price_RjiajH4KEZ43w68DgY5xVaVU" }
      },
      "agent_instances": { "max": 1 }
    },
    "basic": {
      "providers": {
        "stripe": { "price_id": "price_stripe_basic" },
        "house-of-stake": { "price_id": "price_h577VYQUEynPA3uQt1u1neGn" }
      },
      "agent_instances": { "max": 2 }
    },
    "pro": {
      "providers": {
        "stripe": { "price_id": "price_stripe_pro" },
        "house-of-stake": { "price_id": "price_7EAls0E844ULR06EEl53fQoI" }
      },
      "agent_instances": { "max": 5 }
    }
  }
}
```

chat-api already accepts provider-specific subscription checkout requests. It should continue enforcing that House-of-Stake checkout requires a linked NEAR account.

## Verification

Run frontend checks:

```bash
pnpm typecheck
pnpm test
```

Manual cases:

- Google/GitHub/email login without linked NEAR account shows only card payment.
- NEAR-linked user sees both card and NEAR stake payment options.
- Card checkout sends `provider: "stripe"`.
- NEAR stake checkout sends `provider: "house-of-stake"`.
- Existing Stripe subscription uses Stripe plan-change/cancel/resume behavior.
- Existing House-of-Stake subscription uses staking transaction plan-change/cancel/resume behavior.
- Credits purchase with card returns Stripe checkout.
- Credits purchase with NEAR stake returns House-of-Stake wallet transaction intent.

