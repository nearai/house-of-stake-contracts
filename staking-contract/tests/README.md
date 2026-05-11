# Tests layout

## Host-side (`near_sdk::testing_env!`)

Most files here link [`staking-contract`](../../) as a library and drive [`near_sdk::testing_env!`](https://docs.rs/near-sdk) with synthetic contexts (including catalog callbacks simulated via `PromiseResult`). They do **not** deploy WASM.

## Sandbox (`near-workspaces`) + mock pool

[`sandbox_mock_pool.rs`](sandbox_mock_pool.rs) deploys built **`staking_contract.wasm`** and **`mock_staking_pool_contract.wasm`** and exercises real cross-contract calls (`epoch_stake`, pool views, unlock → withdraw → `claim_unlocked_near`, etc.). Helpers live in [`mock_pool/mod.rs`](mock_pool/mod.rs).

Build WASMs from repo root (`house-of-stake-contracts/`):

```bash
make staking-contract
make mock-staking-pool-contract
```

Run only the sandbox integration binary:

```bash
cargo test -p staking-contract --test sandbox_mock_pool
```

(`near-workspaces` installs a local NEAR sandbox binary; supported hosts are **linux-x86** and **darwin-arm**.)
