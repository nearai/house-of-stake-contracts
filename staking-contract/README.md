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

## User cadence (lazy pool pipeline)

Typical sequence after locks exist (no public `epoch_*`; the contract schedules pool calls from user methods; see [`docs/LAZY_EPOCH_PIPELINE.md`](docs/LAZY_EPOCH_PIPELINE.md)):

1. **`lock_for_product` / `lock_for_subscription`** — Mints shares, queues `pending_to_stake`, then [`try_epoch_settle_pool`](src/epoch.rs) runs after balance refresh (one pool **`deposit_and_stake`** or **`unstake`** per NEAR epoch, net of pending buckets).
2. User **`unlock`** — After lock period; refresh balance, queue unstake, then **`unstake`** / withdraw-from-pool as needed.
3. Wait **`epoch_unstake_settle_epochs`** (config) after each successful pool **`unstake`**.
4. User **`claim_unlocked_near`** — May pull unstaked NEAR from the pool into `pending_to_withdraw` when allowed, then pro-rata claim into `withdrawable_balance`.
5. User **`withdraw`** — Transfer `withdrawable_balance` out.

**Per pool and NEAR epoch (matches the staking pool contract):** the pool accepts **at most one** successful **`deposit_and_stake`** **or** **`unstake`** per `epoch_height` for that pool account. The contract records the epoch of the last such success in **`Validator.last_stake_epoch`**, so a second success in the **same** epoch is rejected.

**Net settlement:** before calling the pool, the contract compares **`pending_to_stake`** and **`pending_to_unstake`** in yocto. It stakes only the excess stake, unstakes only the excess unstake, or (when the two are equal and non-zero) clears both buckets and user unstake liability **without** a pool mutating call, still bumping **`last_stake_epoch`**. **`commit_pending_pool_stake`** and **`settle_validator_pool`** (each **1 yocto**) both invoke the same settle step for manual retries. Withdraw-from-pool does **not** use this stake/unstake slot.

## Implementation status (snapshot)

Implemented in code:

- Config, owner / guardians governance, pause, upgrade (`upgrade()` + `migrate_state`)
- On-contract validator **allowlist** (`add_validator`, `pause_validator`, `remove_validator`)
- Validator-owner **catalog** (`create_product`, `create_price`, …)
- Stripe-like deterministic IDs (`prod_*`, `price_*`, `lock_*`, `sub_*`)
- Share minting helpers (`internal.rs`) and NEAR-denominated `lock_for_product` / `lock_for_subscription`
- Subscriptions keyed by `(account_id, product_id)` with tier = [`Subscription::price_id`](src/types.rs): **`cancel_subscription`**, **`upgrade_subscription`**, **`schedule_downgrade_subscription`** ([`subscriptions.rs`](src/subscriptions.rs)). On renewal with a scheduled downgrade, **Phase B prorate** releases catalog tier-gap stake into the normal unstake queue ([`subscriptions.rs`](src/subscriptions.rs) / [`lock.rs`](src/lock.rs) / [`unlock.rs`](src/unlock.rs)).
- `unlock` (user-driven pool unstake path); **`lock_for_*`** schedules refresh + net pool settle; **`claim_unlocked_near`** may chain pool withdraw then claim; **`commit_pending_pool_stake`** / **`settle_validator_pool`** retry settle; user **`withdraw`** from **`withdrawable_balance`**
- Pool callbacks in [`epoch.rs`](src/epoch.rs); **`storage_withdraw`**
- **EVENT_JSON** for lock/unlock, catalog, validators, epoch ops, claim/withdraw, pool withdraw-in ([`events.rs`](src/events.rs)) — `standard: "stake.dao"`, `version: "1.0.0"`, nested `data`
- **`get_products`**, **`get_product_default_price`**, catalog **`unarchive_*`**, **`set_product_default_price`**; **`lock_for_product`** / **`lock_for_subscription`** accept explicit **`price_id`** or **`product_id`** (uses **`Product.default_price_id`**) ([`products.rs`](src/products.rs), [`lock.rs`](src/lock.rs))

Still to refine per [docs/PLAN.md](docs/PLAN.md) / [docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md):

- **Calendar-accurate** subscription billing (average-month linear helper only in [`subscriptions.rs`](src/subscriptions.rs); **`lock_for_subscription`** exists but uses linear months)
- Longer **sandbox E2E** (unlock → `epoch_unstake` → `epoch_withdraw` → `claim_unlocked_near` → `withdraw`) — see [`tests/sandbox_mock_pool.rs`](tests/sandbox_mock_pool.rs); extend as needed

## Workspace

Listed as a member in [../Cargo.toml](../Cargo.toml).
