# staking-cli

`staking-cli` is a small bootstrap CLI for `staking-contract` deployments. It replaces the
environment-variable-heavy shell flow for the first-pass staking use cases:

- `deploy`
- `configure`
- `verify`

Mutating commands default to dry-run. Pass `--send` to submit transactions. Mainnet mutations also
require `--yes-mainnet`.

## Build

```bash
cargo build -p staking-cli
```

## Examples

Build the test-feature contract WASM with the existing Makefile target:

```bash
make staking-contract-test
```

Environment config files live under `staking-contract/cli/config/`:

- `dev.testnet.json` targets `hos-e2e-0601144939.testnet`
- `qa.testnet.json` targets `stake-dao.testnet`
- `stg.mainnet.json` targets `stake-dao.near`
- `prod.mainnet.json` targets `stake.dao`

Code-only deploy to the shared testnet account without running `migrate_state()`:

```bash
cargo run -p staking-cli -- deploy \
  --network testnet \
  --config staking-contract/cli/config/dev.testnet.json \
  --code-only \
  --test-feature \
  --send
```

Fresh deploy with `new(config)`:

```bash
cargo run -p staking-cli -- deploy \
  --network testnet \
  --config staking-contract/cli/config/dev.testnet.json \
  --fresh \
  --send
```

Configure validators and catalog entries from the config file:

```bash
cargo run -p staking-cli -- configure \
  --network testnet \
  --config staking-contract/cli/config/dev.testnet.json \
  --send
```

For existing catalog rows, include `product_id` and/or `price_id` in the config. `configure` will
then update product and price display fields in place. Price amount, type, billing period, and lock
factor are immutable; changing those fields requires creating a new price.

Verify deployment health:

```bash
cargo run -p staking-cli -- verify \
  --network testnet \
  --config staking-contract/cli/config/dev.testnet.json \
  --test-feature
```

The CLI supports `--network testnet` and `--network mainnet`. Mainnet deployment/configuration
requires `--yes-mainnet` in addition to `--send`.
