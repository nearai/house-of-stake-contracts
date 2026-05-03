# Staking Contract — Detailed Design

This document is the authoritative design for `stake.dao` (the `staking-contract` crate). It was copied from the Cursor plan `staking-contract-detailed-design_0143a7b4.plan.md`.

See the repository plan file or ask the team for the latest edits; implementation status may lag this document.

---

The plan below specifies the on-chain design of the new contract that lives at [staking-contract/](.). It is intentionally written so the `README.md` in this folder can be aligned with this content (or distilled into it).

## 1. Goals and non-goals

Goals:
- Allow a NEAR account (the "staker") to purchase a NEAR AI product or subscribe to a plan by **locking** NEAR for a chosen duration. The locked NEAR is staked into the product's validator pool; the validator's commission funds the product (typically 100% commission on `nearai.poolv1.near`).
- Be the single on-chain entrypoint for NEAR AI billing: products, prices, subscriptions, locks.
- Use a pooled meta-validator model: `stake.dao` is the only delegator on each whitelisted validator pool; per-user accounting is internal via shares.
- Be governed by HoS DAO (initially a security multisig), upgradable in the same pattern as the sibling contracts.
- Share patterns/types with the existing workspace ([common/](../common/), [lockup-contract/](../lockup-contract/), [venear-contract/](../venear-contract/)).

Non-goals (for v1):
- Granting veNEAR voting power for `stake.dao` locks (kept independent of `venear-contract`; can be added later via a "register lock with veNEAR" hook).
- Liquid staking tokens (no fungible share token issued; shares are internal).
- Cross-validator rebalancing / autocompounding (stake stays where the user purchased).
- On-chain credit redemption — "credits" are an off-chain billing concept driven by `lock` events.

## 2. System architecture

```mermaid
flowchart LR
    user[User wallet]
    stakeDao[stake.dao - staking-contract]
    oracle[oracle - NEAR price]
    allowlist[validator allowlist]
    poolA[validator A pool e.g. nearai.poolv1.near]
    poolB[validator B pool]
    venearDao[venear.dao optional later]

    user -- "lock / unlock / withdraw" --> stakeDao
    stakeDao -- "deposit_and_stake / unstake / withdraw" --> poolA
    stakeDao -- "..." --> poolB
    stakeDao -- "ft_price NEAR/USD" --> oracle
    stakeDao -- "listed?" --> allowlist
    stakeDao -. "future: register_lock" .-> venearDao
```

Key roles:
- **Contract owner** — HoS DAO (initially a multisig). Onboards validators (adds them to the on-contract allowlist), assigns each validator's owner, sets oracle/operators/global parameters, upgrades the contract.
- **Guardians** — can pause the contract (same pattern as [venear-contract/src/pause.rs](../venear-contract/src/pause.rs)).
- **Operator(s)** — drive `epoch_stake`/`epoch_unstake`/`epoch_withdraw`. Restricted by `Config.operators` (empty list ⇒ permissionless).
- **Validator owner** (e.g., `nearai.sputnik-dao.near`) — owner of an on-contract `Validator` entry. Manages that validator's products and prices on stake.dao, and (separately, off this contract) controls the underlying staking pool itself (commission, etc.). The contract owner does **not** manage products/prices.
- **Stakers** — end users buying products/subscriptions.

## 3. Crate layout

See source files under [src/](src/). Key modules: `config`, `types`, `ids`, `validators`, `products`, `accounts`, `governance`, `pause`, `upgrade`, `oracle`, `lock`, `unlock`, `withdraw`, `epoch`, `pool_callbacks`, `subscriptions`, `events`, `gas`, `internal`.

## 4. Data model (summary)

- **Contract state**: `config`, `paused`, `validators` (allowlist + pool accounting), catalog maps (`products`, `prices` by Stripe-style string id), `accounts`, `subscriptions`, `locks`, `user_validator_shares`, `user_lock_ids`, `id_nonce`.
- **Config**: owner, guardians, operators, oracle id + max age, lock duration bounds, epoch settle epochs, min storage / min lock.
- **Validator**: `pool_account_id`, `owner_account_id`, status, `total_shares`, `total_staked_balance`, pending stake/unstake/withdraw, `tx_status` (Idle/Busy).
- **IDs**: `prod_*`, `price_*`, `sub_*`, `lock_*` with deterministic base62 suffixes (see plan §4.6).
- **Unlock**: Solution 1 only — user calls `unlock(lock_id)` after `end_ns`; share→NEAR at unlock time (captures rewards flexibility).

## 5. Governance

- **Contract owner**: allowlist (`add_validator`, `set_validator_owner`, `pause_validator`, `remove_validator`), oracle, operators, guardians, upgrade.
- **Validator owner**: `create_product`, `edit_product`, `archive_product`, `delete_product`, `create_price`, `edit_price`, `archive_price`, `delete_price` for their validator only.

## 6. External interfaces

- `ext_staking_pool`: `deposit_and_stake`, `unstake`, `withdraw_all`, balance views.
- `ext_oracle`: `get_price`, `get_price_30d_avg` returning `OraclePrice { near_per_usd: Fraction, timestamp_ns }`.

No HoS staking-pool whitelist cross-call — stake.dao allowlist is internal.

## 7. Open items

See full plan for `lock_factor_near_months` decimals, Stripe ID suffix lengths, and subscription duration-equivalent divisor details.

---

_For the complete specification (lifecycle diagrams, formulas, testing strategy), refer to the Cursor plan file `staking-contract-detailed-design_0143a7b4.plan.md` in your plans directory or export from the team wiki._
