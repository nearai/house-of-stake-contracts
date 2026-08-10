# CLAUDE.md

This file provides guidance to Claude Code when reviewing or working in this repository.

## Repository Overview

This repository contains the House-of-Stake NEAR smart contracts:

- `venear-contract`: veNEAR accounting and lockup factory.
- `lockup-contract`: user-owned lockup contract for locked and staked NEAR.
- `voting-contract`: proposal and voting logic.
- `staking-contract`: catalog, staking, subscriptions, direct payments, and settlement logic.
- `mock-staking-pool-contract` and `sandbox-staking-whitelist-contract`: test support contracts.
- `common`, `merkle-tree`, and `integration-tests`: shared code and tests.

Rust workspace settings are in `Cargo.toml`. Contracts target `wasm32-unknown-unknown`.

## Common Commands

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --workspace --all-targets --all-features --exclude integration-tests`
- Build contract WASM: `bash build_all.sh`
- Build release WASM packages: `cargo build -p <package> --target wasm32-unknown-unknown --release`
- Run tests: `make test`
- Run staking-contract tests: `make staking-contract-test`

CI uses Rust `1.86.0` and installs `cargo-near` for integration-test WASM artifacts.

## Review Priorities

Treat this as a smart-contract security and correctness repository. Prefer a short, high-signal review over broad style feedback.

Flag issues introduced by the PR in these areas:

- Access control: owner/admin checks, validator owner checks, private callbacks, and reviewer/council authority.
- Deposits: attached-deposit requirements, one-yocto guards, exact payment amounts, refunds, and storage deposits.
- Cross-contract calls: promise callback trust boundaries, ordering assumptions, gas budgets, and state updates before/after promises.
- Accounting: locked NEAR, veNEAR, delegated balances, staking shares, subscriptions, purchases, revenue, and pending unstake state.
- Units and math: yoctoNEAR values, `U128`/`U64` serialization, rounding, overflow, duration and timestamp calculations.
- Persistent state: schema compatibility, migrations, default values, index consistency, and cleanup of secondary indexes.
- Resource usage: unbounded loops over storage collections, missing pagination, storage growth, and gas-heavy operations.
- Contract APIs: backward compatibility, JSON argument shape, optional fields, and event stability.
- Tests: missing behavior tests for changed contract invariants, safety checks, callbacks, or edge cases.

Do not report issues that are merely stylistic or that rustfmt, clippy, or the Rust compiler will catch.

## Coding Guidelines

- Preserve persisted-state compatibility unless the PR explicitly includes a migration plan.
- Keep public contract APIs stable unless the PR explicitly changes API behavior and docs/tests are updated.
- Use checked reasoning for integer math and units. Be explicit about whether values are yoctoNEAR, NEAR, nanoseconds, or basis points.
- Avoid panics in user-reachable paths unless they are intentional contract rejections with clear messages.
- Keep storage indexes and primary records in sync in the same mutation path.
- Add tests through the public contract methods whenever a change affects externally observable behavior.
