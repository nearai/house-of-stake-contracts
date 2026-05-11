# staking-contract (`stake.dao`)

NEAR smart contract for pooled staking tied to NEAR AI **products** and **prices** (validator-owned catalog), with Stripe-style IDs. Catalog amounts are **NEAR (yocto)** only; there is no oracle or USD conversion path.

## Documentation

| File | Purpose |
|------|---------|
| [docs/API.md](docs/API.md) | Public contract methods (RPC-facing API) |
| [docs/DESIGN.md](docs/DESIGN.md) | Readable architecture summary + pointers |
| [docs/PLAN.md](docs/PLAN.md) | Full detailed design (exported from planning session) |
| [docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md) | Open work / backlog vs design |

## Build

From repo root (`house-of-stake-contracts/`):

```bash
cargo check -p staking-contract
# WASM (requires cargo-near, same as sibling crates):
cargo near build non-reproducible-wasm --manifest-path staking-contract/Cargo.toml
```

`build_all.sh` also builds this crate and copies `staking_contract.wasm` to `res/local/`.

For **sandbox integration tests** that exercise real pool cross-contract calls, build the mock pool (`../mock-staking-pool-contract`) as well: `make mock-staking-pool-contract` (repo root `Makefile`, alias `make mock-pool`) or run `build_all.sh`. Tests live in [`tests/sandbox_mock_pool.rs`](tests/sandbox_mock_pool.rs) (helpers in [`tests/mock_pool/mod.rs`](tests/mock_pool/mod.rs)) and load WASM from `res/local/`, `target/near/…`, or `target/wasm32-unknown-unknown/release/…`.

## Operator / user cadence (unlock → cash)

Typical sequence after locks exist:

1. **`epoch_stake`** — Move `pending_to_stake` into the pool (when safe).
2. User **`unlock`** — After lock period; queues NEAR into `user_pending_unstake` / `pending_to_unstake`.
3. **`epoch_unstake`** — Drain `pending_to_unstake` into the pool’s unstaked bucket.
4. Wait **`epoch_unstake_settle_epochs`** (config).
5. **`epoch_withdraw`** — Pull unstaked NEAR from the pool into `pending_to_withdraw` on the validator record.
6. User **`claim_unlocked_near`** — Pro-rata claim into `withdrawable_balance`.
7. User **`withdraw`** — Transfer `withdrawable_balance` out.

## Implementation status (snapshot)

Implemented in code:

- Config, owner / guardians / operators governance, pause, upgrade (`upgrade()` + `migrate_state`)
- On-contract validator **allowlist** (`add_validator`, `pause_validator`, `remove_validator`)
- Validator-owner **catalog** (`create_product`, `create_price`, …)
- Stripe-like deterministic IDs (`prod_*`, `price_*`, `lock_*`, `sub_*`)
- Share minting helpers (`internal.rs`) and NEAR-denominated `lock_for_product` / `lock_for_subscription`
- Subscriptions keyed by `(account_id, product_id)` with tier = [`Subscription::price_id`](src/types.rs): **`cancel_subscription`**, **`upgrade_subscription`**, **`schedule_downgrade_subscription`**. On renewal with a scheduled downgrade, **Phase B prorate** releases catalog tier-gap stake into the normal unstake queue ([`lock.rs`](src/lock.rs) / [`unlock.rs`](src/unlock.rs)).
- `unlock` (user-driven); operator **`epoch_stake`**, **`epoch_unstake`**, **`epoch_withdraw`**; user **`claim_unlocked_near`** → **`withdraw`**
- `refresh_validator_balance` + pool callbacks; **`storage_withdraw`**
- **EVENT_JSON** for lock/unlock, catalog, validators, epoch ops, claim/withdraw, pool withdraw-in ([`events.rs`](src/events.rs)) — `standard: "stake.dao"`, `version: "1.0.0"`, nested `data`
- **`list_product_ids`** (+ [`get_product`](src/products.rs)) for catalog discovery

Still to refine per [docs/PLAN.md](docs/PLAN.md) / [docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md):

- **Calendar-accurate** subscription billing (average-month linear helper only in [`subscriptions.rs`](src/subscriptions.rs); **`lock_for_subscription`** exists but uses linear months)
- Longer **sandbox E2E** (unlock → `epoch_unstake` → `epoch_withdraw` → `claim_unlocked_near` → `withdraw`) — see [`tests/sandbox_mock_pool.rs`](tests/sandbox_mock_pool.rs); extend as needed

## Workspace

Listed as a member in [../Cargo.toml](../Cargo.toml).
