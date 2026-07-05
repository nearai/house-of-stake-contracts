# Testnet Deployment

This runbook uses one script for staking contract deployment, upgrade, validator
allowlisting, and catalog product/price setup on NEAR testnet.

## Build

From the repository root:

```bash
./scripts/build_staking_wasm.sh
```

This writes:

```text
res/local/staking_contract.wasm
```

## Fresh Deploy

Use an existing testnet account with a key in the NEAR keychain:

```bash
export STAKING_ACCOUNT_ID=stake-dao-dev.testnet
export OWNER_ACCOUNT_ID=owner.stake-dao-dev.testnet
export GUARDIANS_JSON='["guardian.stake-dao-dev.testnet"]'

BUILD_WASM=1 ./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

To create the contract subaccount first:

```bash
CREATE_ACCOUNT=1 \
PARENT_ACCOUNT_ID=my-parent.testnet \
STAKING_ACCOUNT_ID=stake-dao-dev.my-parent.testnet \
OWNER_ACCOUNT_ID=my-parent.testnet \
BUILD_WASM=1 \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

The default config is testnet-friendly:

- `min_lock_duration_ns=1`
- `max_lock_duration_ns=63072000000000000` (about 2 years)
- `epoch_unstake_settle_epochs=1`
- `min_storage_deposit=0.01 NEAR`
- `per_lock_storage_stake=0`
- `per_farm_position_storage_stake=0`
- `per_purchase_storage_stake=0`
- `min_lock_amount=1 NEAR`

Override these with environment variables before running the script.

## Existing E2E Contract

The current shared E2E testnet contract is:

```bash
export STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
```

Preview an owner-gated upgrade without sending transactions:

The contract exposes owner-gated `upgrade()` and private `migrate_state()`.

```bash
DRY_RUN=1 \
ACTION=upgrade \
OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID" \
BUILD_WASM=1 \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

Run the upgrade:

```bash
ACTION=upgrade \
OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID" \
BUILD_WASM=1 \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

## Staking Farm Upgrade Script

Use the focused farm upgrade script when you only want to build the latest
staking contract WASM and call the owner-gated `upgrade()` method on the shared
testnet contract.

Preview without sending a transaction:

```bash
./scripts/upgrade_testnet_staking_farm.sh
```

Run the farm upgrade:

```bash
EXECUTE=1 ./scripts/upgrade_testnet_staking_farm.sh
```

Run staking contract tests first:

```bash
RUN_TESTS=1 EXECUTE=1 ./scripts/upgrade_testnet_staking_farm.sh
```

## Validators And Catalog

Use `ACTION=configure` to allowlist validators and create products/prices. The
script is idempotent for existing validators, products, and active matching
prices when `DRY_RUN` is not set.

```bash
export OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID"
export VALIDATORS_JSON='[
  {"validator_id":"mock-pool-0.hos-e2e-0601144939.testnet"}
]'
export CATALOG_JSON='[
  {
    "validator_id":"mock-pool-0.hos-e2e-0601144939.testnet",
    "owner_account_id":"hos-e2e-0601144939.testnet",
    "name":"NEAR AI Credits",
    "description":"One-off NEAR AI credits",
    "prices":[
      {
        "name":"NEAR AI Credits",
        "description":"One-off NEAR AI credit",
        "amount":"400000000000000000000000",
        "price_type":"OneOff",
        "billing_period":null,
        "lock_factor_near_months":"0",
        "metadata":null,
        "set_default":true
      }
    ]
  }
]'

DRY_RUN=1 ACTION=configure ./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

Remove `DRY_RUN=1` to send transactions. For a newly created product, the script
looks up the generated `product_id` before creating prices. In dry-run mode,
generated IDs are shown as placeholders in follow-up price/default calls.

### NEAR AI Credits

The script can generate the one-off credit catalog for chat-api payments:

```bash
export STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
export OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID"
export VALIDATORS_JSON='[
  {"validator_id":"mock-pool-0.hos-e2e-0601144939.testnet"}
]'

NEAR_AI_CREDITS_CATALOG=1 \
NEAR_AI_CREDITS_OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID" \
DRY_RUN=1 \
ACTION=configure \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

`NEAR_AI_CREDITS_PRICE_AMOUNT_YOCTO` defaults to
`400000000000000000000000` (0.4 NEAR per credit). The generated price is one-off
and is set as the product default.

### Agent Subscription Tiers

The script can generate the monthly agent hosting subscription catalog:

- Starter: `[1, 10]` NEAR stake, 1 agent
- Basic: `[10, 40]` NEAR stake, 2 agents
- Pro: `[40, 400]` NEAR stake, 5 agents

```bash
export STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
export OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID"
export VALIDATORS_JSON='[
  {"validator_id":"mock-pool-0.hos-e2e-0601144939.testnet"}
]'

AGENT_SUBSCRIPTION_CATALOG=1 \
AGENT_SUBSCRIPTION_OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID" \
DRY_RUN=1 \
ACTION=configure \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

`AGENT_SUBSCRIPTION_VALIDATOR_ID` is inferred when `VALIDATORS_JSON` contains
exactly one validator. Set it explicitly when configuring more than one
validator. The generated prices are recurring monthly prices; lower bounds are
stored in `amount`, and inclusive upper bounds are stored in
`metadata.max_amount`.

If the validator is a mock pool managed by this repo, a validator entry can also
create and deploy it:

```json
{
  "validator_id": "mock-pool-0.hos-e2e-0601144939.testnet",
  "owner_account_id": "hos-e2e-0601144939.testnet",
  "create_account": true,
  "deploy_mock_pool": true
}
```

`create_account=true` requires `PARENT_ACCOUNT_ID`; `deploy_mock_pool=true`
requires `res/local/mock_staking_pool_contract.wasm` or `BUILD_WASM=1`.

## Combined Upgrade And Configure

To upgrade the existing contract and then configure validators/catalog in one
run:

```bash
ACTION=upgrade-and-configure \
OWNER_ACCOUNT_ID="$STAKING_ACCOUNT_ID" \
BUILD_WASM=1 \
./scripts/deploy_testnet_staking_contract.sh "$STAKING_ACCOUNT_ID"
```

Buyers must call `storage_deposit` before `lock` or `pay` flows because the
latest contract includes NEP-145 storage management.
