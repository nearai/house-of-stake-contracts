# Lazy epoch pipeline (user-driven pool operations)

This document is the **in-repo** design and **implementation tracker** for moving stake.dao’s validator pool work (`deposit_and_stake`, `unstake`, withdraw-from-pool, balance refresh) off a separate **operator** role and off public **`epoch_*` / `refresh_validator_balance`** methods. Users drive settlement through **`lock`**, **`unlock`**, and **`claim_unlocked_near`**.

The authoritative narrative (risks, mitigations, test matrix) was captured in the Cursor plan **lazy_validator_epoch_ops** (see your workspace `.cursor/plans/` copy if present).

## Goals

| Topic | Decision |
|--------|-----------|
| Public `epoch_stake` / `epoch_unstake` / `epoch_withdraw` / `refresh_validator_balance` | **Removed** from the contract ABI. |
| `operators` / `set_operators` / `assert_operator` | **Removed** (no live deployments with old `Config`). |
| Balance before mint / unlock | **`get_account_total_balance`** when **`last_settlement_epoch` < `epoch_height`** (then withdraw-if-ready and **`try_epoch_settle`** on existing pending). When **`last_settlement_epoch` ≥ `epoch_height`**, **skip** that entire pre-user pipeline and mint / unlock immediately using cached **`total_staked_balance`**. |
| Withdraw before new unstake | If settle allows and the pool still has withdrawable unstaked NEAR for this contract, **pull from pool first**, then `unstake`. |
| First delegation to an empty validator | **Minimum 1 NEAR** on first lock when `total_shares == 0`, hardcoded as [`MIN_FIRST_VALIDATOR_DEPOSIT_NEAR_YOCTO`](../src/config.rs) (not configurable). |
| Subscription downgrade prorate | **Does not** schedule pool unstake; user **`unlock`** drives unstake. |
| `claim_unlocked_near` | May chain internal withdraw-from-pool when batches are missing or depleted (see plan §2b). |
| User-facing errors | No “operator” / “run refresh” / “run epoch_*” wording. |
| Pool mutating actions per NEAR epoch | Per **allowlisted pool account** (`validator_id` = pool contract), at most **one** successful **`deposit_and_stake`** **or** **`unstake`** per `epoch_height` (**`Validator.last_settlement_epoch`**). **`try_epoch_settle`** compares **`pending_to_stake`** vs **`pending_to_unstake`** in yocto: deposits **`max(0, stake − unstake)`**, unstakes **`max(0, unstake − stake)`**, or clears both without a pool call when equal (still bumps **`last_settlement_epoch`**). User tranche liability is reduced when unstake is absorbed by stake. Withdraw-from-pool is separate and does not consume that slot. Catalog **`lock`** / **`unlock`**: when settlement for the epoch is **due** (`last_settlement_epoch` < current height), at most one **`get_account_total_balance`** per pool per NEAR epoch, then pre-user withdraw-if-ready and settle on **existing** pending before minting a lock or (when due) before setting **`Busy`** and queueing an unlock; when the pool **already** settled this epoch, **none** of those pre-user steps run before mint / unlock. |

## Major concerns (pre-ship)

1. **Prepaid gas** on long cross-contract chains — document floors in `docs/API.md`; consider `require!(env::prepaid_gas() >= …)` on hot paths.
2. **Callback payload / continuation** — keep args small; prefer reloading catalog rows by id in callbacks.
3. **`MIN_FIRST_VALIDATOR_DEPOSIT_NEAR_YOCTO` vs `min_lock_amount`** — first lock on an empty pool must satisfy both the hardcoded 1 NEAR floor and `min_lock_amount` from config.
4. **`tx_status == Busy`** — expect retries; clear copy.
5. **Tests / scripts** — sandbox and deploy scripts that called `epoch_*` must switch to user flows.

## Implementation status

| Area | Status |
|------|--------|
| This doc | **Done** |
| `Config`: drop `operators`; first-pool minimum is hardcoded (`MIN_FIRST_VALIDATOR_DEPOSIT_NEAR_YOCTO` in `config.rs`) | **Done** |
| `governance`: remove operator APIs | **Done** |
| `epoch.rs`: `pub(crate) try_epoch_settle` / `try_epoch_withdraw`, remove public epoch/refresh, withdraw-before-unstake pipeline | **Done** |
| `unlock.rs`: refresh → queue → withdraw-first → unstake | **Done** |
| `lock.rs`: refresh → mint → `try_epoch_settle`, first-deposit floor; production `PromiseOrValue`, unit tests `#[cfg(test)]` sync path | **Done** |
| `epoch.rs`: public **`epoch_settle`** (manual / retry net settle) | **Done** |
| `withdraw.rs`: claim prefetches pool withdraw when bucket empty but settle allows (§2b) | **Done** |
| `docs/API.md`, `README.md` | **Done** (lazy user cadence, `epoch_settle`, no public batch `epoch_*`) |
| `docs/DESIGN.md`, `docs/PLAN.md` | **Done** (aligned with lazy pipeline; PLAN retains historical YAML todos) |
| Sandbox / unit / integration tests | **In progress** (host needs `near-workspaces` sandbox where supported) |
| `scripts/deploy_testnet_staking_stack.sh` | **Done** (init JSON matches `Config` without `operators`) |

Update the table above as work lands.

## Unit tests vs WASM

`cargo test -p staking-contract` pulls `near-workspaces` (sandbox binary); unsupported host targets cannot compile tests. Host-side `tests/*.rs` use `#[cfg(test)]` synchronous lock minting and `tests/common::unwrap_sync_lock_id` because `lock_for_product` / `lock_for_subscription` return `PromiseOrValue<LockId>` on the real contract ABI.

## Related sources

- [`src/epoch.rs`](../src/epoch.rs) — pool promises and callbacks.
- [`src/unlock.rs`](../src/unlock.rs) — user unlock entrypoint.
- [`src/lock.rs`](../src/lock.rs) — lock / mint / `pending_to_stake`.
- [`src/withdraw.rs`](../src/withdraw.rs) — `claim_unlocked_near`.
