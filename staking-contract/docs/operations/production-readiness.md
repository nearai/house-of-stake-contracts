# Staking contract — production readiness

Backlog for taking **`stake.dao`** (`staking-contract`) from **implemented v1** to **production on NEAR mainnet**. Design reference: [DESIGN.md](../DESIGN.md), [API.md](../API.md), [features/lazy-epoch-pipeline.md](../features/lazy-epoch-pipeline.md). Code review checklist: [review/code-review.md](../review/code-review.md).

**Status:** Core user flows (lock → pool stake → unlock → withdraw) are implemented and covered by unit + mock-pool sandbox tests. Items below are what remains before treating the contract as production-ready.

---

## Production gate (summary)

| Gate | Status |
|------|--------|
| Lazy pool pipeline + `withdraw` pays users NEAR | **Done** (code) |
| CI: build, fmt, clippy, `make test` on Ubuntu | **Done** (workflows) |
| External security audit | **Open** |
| Full user journey tested against a **real** staking pool (not only mock) | **Open** |
| Mainnet deploy + governance runbook | **Open** |
| Integrator docs (min gas, error cases, indexer events) | **Partial** ([API.md](../API.md)) |
| Testnet soak / operational playbook | **Open** |

---

## P0 — Must complete before mainnet

### Security and correctness review

- [ ] **External audit** — Third-party review of `epoch.rs`, `lock.rs`, `unlock.rs`, `withdraw.rs`, `utils.rs`, and subscription economics. Track findings to resolution before upgrade key handoff.
- [ ] **Internal review** — Walk [review/code-review.md](../review/code-review.md) and [features/lazy-epoch-pipeline.md](../features/lazy-epoch-pipeline.md) review checklist on `feat/stake-dao` / review-prep branch; sign off on callback failure paths, **`Busy`** / **`Idle`**, and net-zero settle.
- [ ] **Fast-path / cached balance** — Document and accept (or fix) that when `last_settlement_epoch >= epoch_height`, mint/unlock pricing uses **`total_staked_balance`** without a fresh pool `get_account`. Staking **rewards** can drift vs shares until the next full settlement; confirm this matches product risk tolerance ([DESIGN.md](../DESIGN.md) § accounting).

### End-to-end funds path

- [x] **Sandbox E2E (mock pool)** — Golden path in [`sandbox_golden_path.rs`](../../tests/sandbox_golden_path.rs): **`lock`** → **`epoch_settle`** → **`unlock`** → settlement epochs → **`withdraw(validator_id)`** with NEAR received by buyer. Deeper pipeline cases remain in [`sandbox_mock_pool.rs`](../../tests/sandbox_mock_pool.rs) and [`sandbox_epoch_settlement.rs`](../../tests/sandbox_epoch_settlement.rs).
- [ ] **Testnet validation on a real pool** — Deploy via [`scripts/deploy_testnet_staking_stack.sh`](../../../scripts/deploy_testnet_staking_stack.sh) (mock pool) **and** exercise at least one **production-shaped** staking pool account on testnet (allowlist, catalog, lock, unlock, withdraw). Mock pool behavior must not be the only pre-mainnet evidence.
- [ ] **Concurrent / retry behavior** — QA `tx_status == Busy` (overlapping lock/unlock/withdraw), failed pool callbacks, and **`epoch_settle`** recovery; ensure users are never permanently stuck.

### Release engineering

- [ ] **Reproducible WASM** — Ship with `cargo near build` / [Cargo.toml](../../Cargo.toml) `near.reproducible_build` pins; record artifact hash in release notes (same pattern as sibling HoS contracts).
- [ ] **CI green on merge branch** — `make test` (includes WASM build via `build_all.sh`), `cargo clippy`, `cargo fmt --check`.
- [ ] **Mainnet init config** — Finalize `Config`: `owner_account_id`, `guardians`, `min_lock_amount`, `epoch_unstake_settle_epochs`, `min_storage_deposit`, `per_lock_storage_stake`, lock duration bounds. Document who can change each via governance ([`governance.rs`](../../src/governance.rs)).
- [ ] **Upgrade path** — Dry-run `upgrade()` + `migrate_state` on testnet; confirm guardian pause works before/after upgrade.

### Operations

- [ ] **Mainnet deploy runbook** — Subaccounts, WASM upload, `new(config)`, `add_validator` for each production pool, validator-owner catalog setup. Extend or fork testnet script for mainnet naming and funding.
- [ ] **Pause / incident response** — Written procedure: guardians call `pause`, comms, root-cause, `unpause` or upgrade.
- [ ] **Indexer / observability** — Document `EVENT_JSON` (`standard: "stake.dao"`, `version: "1.0.0"`) for locks, unlocks, epoch ops, withdraws ([`events.rs`](../../src/events.rs)); alert on high `Busy` failure rate or stuck `pending_to_*`.

### Integrator-facing

- [ ] **Minimum prepaid gas** — Publish recommended TGas for `lock`, `update_subscription`, `unlock`, `withdraw` (see [`gas.rs`](../../src/gas.rs) `EPOCH_SETTLEMENT_MIN_GAS` and callback budgets). Verify `require_enough_gas_for_epoch_settlement` thresholds against worst-case chains on testnet.
- [ ] **Wallet / SDK copy** — Clear errors for: insufficient storage deposit, below `min_lock_amount`, lock not ended, withdraw before tranche `available_epoch_height`, validator paused/removed.

---

## P1 — Strongly recommended for launch (or fast follow)

### Testing

- [ ] **Accounting invariants** — Property or scripted tests: Σ user shares vs `Validator.total_shares`; `pending_user_unstake_total` vs tranches; after net-zero, `pending_to_unstake` re-rooted correctly.
- [x] **Subscription lifecycle (sandbox)** — [`sandbox_subscription_e2e.rs`](../../tests/sandbox_subscription_e2e.rs): `update_subscription` immediate and scheduled updates + renewal. Host coverage: [`subscription_lifecycle.rs`](../../tests/subscription_lifecycle.rs) (incl. Phase B prorate on `user_pending_unstake`).

### Product / economics

- [ ] **Automatic period-end updates** — Due subscription updates are projected in views after `apply_ns` and lazily committed by later mutations. Decide whether v1 also needs an off-chain keeper for exact-at-boundary storage commits.
- [ ] **Calendar-accurate billing** — Replace linear [`add_months_stripe_style`](../../src/subscriptions.rs) with true calendar month / anchor-day end dates (anchor_day is stored; logic is approximate today).
- [ ] **Stranded `pending_to_withdraw` dust** — Rounding can leave bucket balance with zero user liability ([DESIGN.md](../DESIGN.md) §7). Either implement owner-only **`sweep_stranded_withdraw_bucket`**, or accept dust and document for governance.

### Pool withdraw hardening (optional for v1)

- [ ] **Withdraw amount reconciliation** — Optional `balance_before` snapshot and `min(balance_after − balance_before, requested)` on pool withdraw callbacks ([`epoch.rs`](../../src/epoch.rs)); today credits the requested amount on success.

---

## P2 — Post-launch / v1.1+

- [ ] **veNEAR opt-in** — Per-lock voting power via `venear-contract` ([features/venear-integration.md](../features/venear-integration.md)); not in v1 scope.
- [ ] **Reward drift handling** — Automatic share rebase or periodic forced refresh if product requires strict share↔NEAR parity (explicitly **out of scope** for v1; see open item below).
- [ ] **Mainnet monitoring** — Dashboards for per-validator `pending_to_stake`, `pending_to_unstake`, `pending_to_withdraw`, `tx_status`.
---

## Completed in v1 (no further work required for baseline)

| Area | Notes |
|------|--------|
| Lazy pool pipeline | `lock` / `unlock` / `withdraw` / `epoch_settle`; no public batch `epoch_*` ([`features/lazy-epoch-pipeline.md`](../features/lazy-epoch-pipeline.md)) |
| Unlock → withdraw | Pro-rata tranches, pool withdraw chain, NEAR transfer to user |
| NEAR-only catalog | No oracle / USD path |
| Subscriptions | `lock`, cancel / resume / update subscription, Phase B prorate at renewal |
| Storage | NEP-145 `storage_deposit` / `storage_withdraw`, per-lock storage stake |
| Governance | Owner, guardians, pause, upgrade, validator allowlist, catalog via pool `get_owner_id` |
| Events | `EVENT_JSON` for indexing |
| Unit + sandbox tests | Host `testing_env!` modules + mock-pool sandbox suites |
| Testnet deploy script | [`deploy_testnet_staking_stack.sh`](../../../scripts/deploy_testnet_staking_stack.sh) |
| CI workflows | build / format / clippy / test |

---

## Explicitly out of scope for v1

- **Oracle / USD-priced locks** — removed; NEAR yocto catalog only.
- **Liquid staking token** — internal shares only.
- **Cross-validator rebalancing** — stake stays on purchased pool.
- **Automatic reward rebase** — cached `total_staked_balance`; no on-chain correction when pool rewards accrue between refreshes (document for users/operators).

---

## Known limitations (document, do not forget)

| Topic | Location / doc |
|--------|----------------|
| Linear-month subscription periods | [`subscriptions.rs`](../../src/subscriptions.rs) |
| Exact-at-boundary subscription storage commits | Due updates are projected in views, then lazily committed by later mutations |
| Stranded withdraw bucket dust | [DESIGN.md](../DESIGN.md) §7, [API.md](../API.md) withdraw note |
| Host tests use sync lock path | [`tests/README.md`](../../tests/README.md) |

---

*Last updated: production-readiness pass — post `feat/stake-dao` / review-prep doc merge.*
