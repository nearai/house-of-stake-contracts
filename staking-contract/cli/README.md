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

Code-only deploy to the shared testnet account without running `migrate_state()`:

```bash
cargo run -p staking-cli -- deploy \
  --network testnet \
  --config staking-contract/cli/config/testnet.dev.json \
  --code-only \
  --test-feature \
  --send
```

Fresh deploy with `new(config)`:

```bash
cargo run -p staking-cli -- deploy \
  --network testnet \
  --config staking-contract/cli/config/testnet.dev.json \
  --fresh \
  --send
```

Configure validators and catalog entries from the config file:

```bash
cargo run -p staking-cli -- configure \
  --network testnet \
  --config staking-contract/cli/config/testnet.dev.json \
  --send
```

Verify deployment health:

```bash
cargo run -p staking-cli -- verify \
  --network testnet \
  --config staking-contract/cli/config/testnet.dev.json \
  --test-feature
```

The CLI supports `--network testnet` and `--network mainnet`. Mainnet deployment/configuration
requires `--yes-mainnet` in addition to `--send`.
