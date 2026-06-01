# Testnet Deployment

This runbook deploys only `staking-contract` to NEAR testnet. Use
`scripts/deploy_testnet_staking_stack.sh` when you also want mock staking pools.

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
- `min_lock_amount=1 NEAR`

Override these with environment variables before running the script.

## Upgrade Dry Run

The contract exposes owner-gated `upgrade()` and private `migrate_state()`.

```bash
ACTION=upgrade \
OWNER_ACCOUNT_ID=owner.stake-dao-dev.testnet \
BUILD_WASM=1 \
./scripts/deploy_testnet_staking_contract.sh stake-dao-dev.testnet
```

For a command preview without sending transactions:

```bash
DRY_RUN=1 ACTION=upgrade ./scripts/deploy_testnet_staking_contract.sh stake-dao-dev.testnet
```

## After Deploy

Allowlist real testnet staking pools:

```bash
near contract call-function as-transaction "$STAKING_ACCOUNT_ID" add_validator \
  json-args '{"validator_id":"<pool.testnet>"}' \
  prepaid-gas '50.0 Tgas' attached-deposit '1 yoctoNEAR' \
  sign-as "$OWNER_ACCOUNT_ID" network-config testnet sign-with-keychain send
```

Then have each validator owner create catalog products and prices with
`create_product` and `create_price`.
