# staking-contract (`stake.dao`)

NEAR smart contract for pooled staking tied to NEAR AI **products** and **prices** (validator-owned catalog), with Stripe-style IDs and hybrid pricing hooks.

## Documentation

| File | Purpose |
|------|---------|
| [DESIGN.md](DESIGN.md) | Readable architecture summary + pointers |
| [PLAN.md](PLAN.md) | Full detailed design (exported from planning session) |

## Build

From repo root (`house-of-stake-contracts/`):

```bash
cargo check -p staking-contract
# WASM (requires cargo-near, same as sibling crates):
cargo near build non-reproducible-wasm --manifest-path staking-contract/Cargo.toml
```

`build_all.sh` also builds this crate and copies `staking_contract.wasm` to `res/local/`.

## Implementation status (snapshot)

Implemented in code:

- Config, owner / guardians / operators governance, pause, upgrade (`upgrade()` + `migrate_state`)
- On-contract validator **allowlist** (`add_validator`, `set_validator_owner`, `pause_validator`, `remove_validator`)
- Validator-owner **catalog** (`create_product`, `create_price`, …)
- Stripe-like deterministic IDs (`prod_*`, `price_*`, `lock_*`, `sub_*`)
- Share minting helpers (`internal.rs`) and **Near-priced** `lock_for_product`
- `unlock` (user-driven); operator **`epoch_stake`**, **`epoch_unstake`**, **`epoch_withdraw`** (two-step unstaked balance query + `withdraw`, like lockup); user **`claim_unlocked_near`** → `withdrawable_balance`; **`withdraw`**
- `refresh_validator_balance` + pool callbacks; **`storage_withdraw`** (NEP-145 excess above `min_storage_deposit`)
- Lock **EVENT_JSON** on product lock (`events.rs`)

Still to wire per [PLAN.md](PLAN.md) / [ACTION_ITEMS.md](ACTION_ITEMS.md):

- USD locks via Burrow-style **`oracle_on_call`** + relay (see ACTION_ITEMS P1)
- `lock_for_subscription` + calendar-month extension ([subscriptions.rs](src/subscriptions.rs))
- More EVENT_JSON coverage and sandbox integration tests

## Workspace

Listed as a member in [../Cargo.toml](../Cargo.toml).
