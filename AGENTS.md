# AGENTS.md

## Cursor Cloud specific instructions

### Overview

This is the **House-of-Stake (HoS)** smart contracts repository — NEAR Protocol smart contracts (Rust, edition 2024) implementing a veNEAR governance and staking system. The workspace has 3 deployable contracts (`venear-contract`, `lockup-contract`, `voting-contract`), 2 shared libraries (`merkle-tree`, `common`), 1 test helper contract (`sandbox-staking-whitelist-contract`), and an integration test suite.

### Toolchain

- Rust 1.86 is pinned via `rust-toolchain.toml` (components: `rustfmt`, `clippy`, `rust-analyzer`).
- The `wasm32-unknown-unknown` target is required for contract compilation.
- `cargo-near` (v0.17.0) is required for `cargo near build non-reproducible-wasm`. The latest cargo-near versions require Rust >=1.88, so pin to 0.17.0 which is compatible with the 1.86 toolchain.

### Build, Test, and Lint

- **Build all contracts**: `./build_all.sh` — compiles all four contracts to WASM and copies artifacts to `res/local/`.
- **Run all tests**: `./test_all.sh` — builds contracts then runs `cargo test -- --nocapture`. Integration tests use `near-workspaces` which automatically downloads and runs a local NEAR sandbox node; no external services needed.
- **Run tests only** (if already built): `cargo test -- --nocapture`
- **Clippy**: `cargo clippy --all-targets`
- **Format check**: `cargo fmt --check` — note: the existing codebase has pre-existing formatting diffs as of the initial commit.

### Gotchas

- Integration tests (in `integration-tests/`) take several minutes to run because they spin up NEAR sandbox nodes and execute full contract lifecycle flows.
- The `test_voting_mainnet` test is `#[ignore]`d by default (requires mainnet RPC access at `rpc.intea.rs`).
- System dependencies needed for compilation: `libssl-dev`, `libudev-dev`, `pkg-config`, and a working `libstdc++.so` symlink (the update script handles these).
- `cargo-near` must be installed with `CXX=g++ CC=gcc` flags and `--locked` to avoid build issues with the default clang toolchain in the VM.
