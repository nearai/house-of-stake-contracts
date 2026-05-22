# Tests layout

## Host-side (`near_sdk::testing_env!`)

Most files here link [`staking-contract`](../../) as a library and drive [`near_sdk::testing_env!`](https://docs.rs/near-sdk) with synthetic contexts (including catalog callbacks simulated via `PromiseResult`). They do **not** deploy WASM.

On the **host** triple, `lock_for_*` uses a synchronous mint path (`not(target_arch = "wasm32")` in `lock.rs`) because `testing_env!` does not run promise chains. **WASM** builds always use the real async pipeline. Integration tests link the library **without** `cfg(test)`; they still get the host path when compiled for the host target.

## Sandbox (`near-workspaces`) + mock pool

[`sandbox_mock_pool.rs`](sandbox_mock_pool.rs) and [`sandbox_epoch_settlement.rs`](sandbox_epoch_settlement.rs) deploy built **`staking_contract.wasm`** and **`mock_staking_pool_contract.wasm`** and exercise real cross-contract calls (catalog, `lock_for_product`, pool views, `epoch_settle`, unlock → unstake, net-zero settle, `withdraw(validator_id)`, etc.). Shared helpers live in [`mock_pool/mod.rs`](mock_pool/mod.rs). Requires a **near-workspaces**–supported host (linux-x86 or darwin-arm) to run.

Build WASMs from repo root (`house-of-stake-contracts/`):

```bash
make staking-contract
make mock-staking-pool-contract
```

Run sandbox integration tests:

```bash
cargo test -p staking-contract --test sandbox_mock_pool
cargo test -p staking-contract --test sandbox_epoch_settlement
```

(`near-workspaces` installs a local NEAR sandbox binary; supported hosts are **linux-x86** and **darwin-arm**.)
