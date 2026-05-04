# staking-contract (`stake.dao`)

NEAR smart contract for pooled staking tied to NEAR AI **products** and **prices** (validator-owned catalog), with Stripe-style IDs and hybrid pricing hooks.

## Documentation

| File | Purpose |
|------|---------|
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

## Operator / user cadence (unlock → cash)

Typical sequence after locks exist:

1. **`epoch_stake`** — Move `pending_to_stake` into the pool (when safe).
2. User **`unlock`** — After lock period; queues NEAR into `user_pending_unstake` / `pending_to_unstake`.
3. **`epoch_unstake`** — Drain `pending_to_unstake` into the pool’s unstaked bucket.
4. Wait **`epoch_unstake_settle_epochs`** (config).
5. **`epoch_withdraw`** — Pull unstaked NEAR from the pool into `pending_to_withdraw` on the validator record.
6. User **`claim_unlocked_near`** — Pro-rata claim into `withdrawable_balance`.
7. User **`withdraw`** — Transfer `withdrawable_balance` out.

USD-priced catalog entries use **`oracle_on_call`** (see [`src/oracle_receiver.rs`](src/oracle_receiver.rs)) with a Burrow-style oracle relay, not `lock_for_product`.

## Implementation status (snapshot)

Implemented in code:

- Config, owner / guardians / operators governance, pause, upgrade (`upgrade()` + `migrate_state`)
- On-contract validator **allowlist** (`add_validator`, `set_validator_owner`, `pause_validator`, `remove_validator`)
- Validator-owner **catalog** (`create_product`, `create_price`, …)
- Stripe-like deterministic IDs (`prod_*`, `price_*`, `lock_*`, `sub_*`)
- Share minting helpers (`internal.rs`) and **Near-priced** `lock_for_product`; **USD** via **`oracle_on_call`** ([`oracle_receiver.rs`](src/oracle_receiver.rs)) + relay
- `unlock` (user-driven); operator **`epoch_stake`**, **`epoch_unstake`**, **`epoch_withdraw`**; user **`claim_unlocked_near`** → **`withdraw`**
- `refresh_validator_balance` + pool callbacks; **`storage_withdraw`**
- **EVENT_JSON** for lock/unlock, catalog, validators, epoch ops, claim/withdraw, pool withdraw-in ([`events.rs`](src/events.rs)) — `standard: stakedao`, `version: 1.0.0`, nested `data`
- **`list_product_ids`** (+ [`get_product`](src/products.rs)) for catalog discovery

Still to refine per [docs/PLAN.md](docs/PLAN.md) / [docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md):

- **Calendar-accurate** subscription billing (average-month linear helper only in [`subscriptions.rs`](src/subscriptions.rs); **`lock_for_subscription`** exists but uses linear months)
- Deeper integration tests (full epoch/pool loop beyond smoke deploy / storage)

## Workspace

Listed as a member in [../Cargo.toml](../Cargo.toml).
