# Testnet Contract Snapshot

Last updated: 2026-06-01

## Contract

- Network: `testnet`
- Contract account: `hos-e2e-0601144939.testnet`
- Contract version: `1.0.3`
- Owner: `hos-e2e-0601144939.testnet`
- Validator: `mock-pool-0.hos-e2e-0601144939.testnet`

## Config

```json
{
  "owner_account_id": "hos-e2e-0601144939.testnet",
  "proposed_new_owner_account_id": null,
  "guardians": [],
  "min_lock_duration_ns": "1",
  "max_lock_duration_ns": "63072000000000000",
  "epoch_unstake_settle_epochs": 1,
  "min_storage_deposit": "10000000000000000000000",
  "per_lock_storage_stake": "0",
  "per_purchase_storage_stake": "0",
  "min_lock_amount": "1000000000000000000000000"
}
```

## Products

### NEAR AI Agents

- Product ID: `prod_5lklj46roIwKZK`
- Validator ID: `mock-pool-0.hos-e2e-0601144939.testnet`
- Description: `Monthly agent hosting subscription tiers`
- Default price ID: `price_RjiajH4KEZ43w68DgY5xVaVU`

| Tier | Price ID | Type | Range | Agents |
| --- | --- | --- | --- | --- |
| Starter | `price_RjiajH4KEZ43w68DgY5xVaVU` | Recurring monthly | `[1, 10] NEAR` | 1 |
| Basic | `price_h577VYQUEynPA3uQt1u1neGn` | Recurring monthly | `[10, 40] NEAR` | 2 |
| Pro | `price_7EAls0E844ULR06EEl53fQoI` | Recurring monthly | `[40, 400] NEAR` | 5 |

```json
[
  {
    "price_id": "price_RjiajH4KEZ43w68DgY5xVaVU",
    "product_id": "prod_5lklj46roIwKZK",
    "name": "Starter",
    "description": "1 agent; stake range [1, 10] NEAR",
    "amount": "1000000000000000000000000",
    "price_type": "Recurring",
    "billing_period": "Monthly",
    "lock_factor_near_months": "1000000000000000000000000",
    "metadata": {
      "max_amount": "10000000000000000000000000"
    },
    "status": "Active",
    "usage_count": 0
  },
  {
    "price_id": "price_h577VYQUEynPA3uQt1u1neGn",
    "product_id": "prod_5lklj46roIwKZK",
    "name": "Basic",
    "description": "2 agents; stake range [10, 40] NEAR",
    "amount": "10000000000000000000000000",
    "price_type": "Recurring",
    "billing_period": "Monthly",
    "lock_factor_near_months": "1000000000000000000000000",
    "metadata": {
      "max_amount": "40000000000000000000000000"
    },
    "status": "Active",
    "usage_count": 0
  },
  {
    "price_id": "price_7EAls0E844ULR06EEl53fQoI",
    "product_id": "prod_5lklj46roIwKZK",
    "name": "Pro",
    "description": "5 agents; stake range [40, 400] NEAR",
    "amount": "40000000000000000000000000",
    "price_type": "Recurring",
    "billing_period": "Monthly",
    "lock_factor_near_months": "1000000000000000000000000",
    "metadata": {
      "max_amount": "400000000000000000000000000"
    },
    "status": "Active",
    "usage_count": 0
  }
]
```

### NEAR AI Credits

- Product ID: `prod_37o5G0rr2wMJ5C`
- Validator ID: `mock-pool-0.hos-e2e-0601144939.testnet`
- Description: `One-off NEAR AI credits for chat-api payments`
- Default price ID: `price_z2EbTifr7Nyqwt6v5kFqSiUb`

```json
{
  "price_id": "price_z2EbTifr7Nyqwt6v5kFqSiUb",
  "product_id": "prod_37o5G0rr2wMJ5C",
  "name": "NEAR AI Credit",
  "description": "One NEAR AI credit",
  "amount": "400000000000000000000000",
  "price_type": "OneOff",
  "billing_period": null,
  "lock_factor_near_months": "0",
  "metadata": null,
  "status": "Active",
  "usage_count": 0
}
```

## Consumer Environment

```bash
export NEAR_NETWORK_ID=testnet
export NEAR_STAKING_CONTRACT_ID=hos-e2e-0601144939.testnet

export HOS_AGENT_SUBSCRIPTION_PRODUCT_ID=prod_5lklj46roIwKZK
export HOS_AGENT_STARTER_PRICE_ID=price_RjiajH4KEZ43w68DgY5xVaVU
export HOS_AGENT_BASIC_PRICE_ID=price_h577VYQUEynPA3uQt1u1neGn
export HOS_AGENT_PRO_PRICE_ID=price_7EAls0E844ULR06EEl53fQoI

export HOS_CREDIT_PRODUCT_ID=prod_37o5G0rr2wMJ5C
export HOS_CREDIT_PRICE_ID=price_z2EbTifr7Nyqwt6v5kFqSiUb
```

## Notes

- Buyers must call `storage_deposit` before `lock` or `pay`.
- Minimum storage deposit is `0.01 NEAR`.
- Credit price is `0.4 NEAR` per credit.
