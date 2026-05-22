# PR: `feat/stake-dao` → `main`

> Review prep branch: `chore/stake-dao-review-prep` (doc alignment, dead-code cleanup, review checklist).

## Summary

This PR introduces **`staking-contract` (`stake.dao`)** — a new NEAR smart contract in the House of Stake workspace that lets users lock NEAR against validator-owned product catalogs, delegate through allowlisted staking pools via an internal share model, and complete the full lifecycle (lock → stake → unlock → withdraw) without a separate operator role.

**Base branch:** `main`  
**Feature branch:** `feat/stake-dao`  
**Commits:** 1 (`feat: staking contract v1`) + review-prep fixes on `chore/stake-dao-review-prep`  
**Scope:** ~87 files, +11.8k / −780 lines (mostly new `staking-contract/` crate)

### What's new

- **Core contract** (`staking-contract/src/`): validator allowlist, NEAR-only product/price catalog (Stripe-style IDs), share-based locks, subscriptions (cancel / upgrade / schedule downgrade with Phase B prorate at renewal), lazy pool pipeline (`epoch.rs`), unlock → withdraw with pro-rata claims, NEP-145-style storage, pause/upgrade/governance, `EVENT_JSON` (`standard: "stake.dao"`, v1.0.0).
- **Mock staking pool** (`mock-staking-pool-contract/`) for sandbox cross-contract tests.
- **Tests**: 17 test modules under `staking-contract/tests/`; workspace smoke test in `integration-tests/tests/test_staking_contract.rs`.
- **Docs**: API, design, lazy epoch pipeline, epoch settlement chain, veNEAR integration (v2), PLAN (historical), ACTION_ITEMS, [`REVIEW.md`](REVIEW.md).
- **Tooling**: root `Makefile`, `build_all.sh`, deploy scripts, CI workflows.

### Design highlights

- **NEAR-only pricing** — no oracle or USD path.
- **Lazy pipeline** — pool work from `lock` / `unlock` / `withdraw` and optional **`epoch_settle(validator_id)`** retry; no public `epoch_stake` / `epoch_unstake` / `epoch_withdraw` batch APIs.
- **veNEAR** — v1 does not grant veNEAR power; `VENEAR_INTEGRATION.md` documents v2 opt-in.

### Sibling crate changes

Minor formatting/import reordering across `common`, `lockup-contract`, `venear-contract`, and `voting-contract`. No functional changes to voting/lockup/venear behavior.

### Known follow-ups (documented, not blocking v1)

- Calendar-accurate subscription billing (linear month helper only today).
- Reward drift vs cached `total_staked_balance` — no automatic rebase.
- Longer sandbox E2E: unlock → wait `epoch_unstake_settle_epochs` → `withdraw`.

---

## Test plan

- [ ] `make check-staking-contract`
- [ ] `make test-staking-contract` (CI on Ubuntu; needs built WASM + near-workspaces-supported host locally)
- [ ] `make staking-contract` then integration smoke test (`test_staking_contract.rs`)
- [ ] `cargo clippy -p staking-contract` (workspace clippy may flag `common`; staking crate should be clean)
- [ ] Review per [`docs/REVIEW.md`](REVIEW.md) and [`docs/CORE_FEATURES.md`](CORE_FEATURES.md)

---

## Reviewer pointers

| Area | Files |
|------|--------|
| Pool callbacks & settlement | `src/epoch.rs`, `docs/LAZY_EPOCH_PIPELINE.md` |
| Share math & lock sufficiency | `src/utils.rs`, `src/lock.rs` |
| Subscriptions & downgrade prorate | `src/subscriptions.rs`, `src/lock.rs` |
| Sandbox E2E | `tests/sandbox_mock_pool.rs`, `tests/mock_pool/mod.rs` |
| Public API | `docs/API.md` |
