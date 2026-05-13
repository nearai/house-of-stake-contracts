#!/usr/bin/env bash
# Build staking-contract and mock-staking-pool-contract WASM into res/local/
# (same outputs as `make staking-contract mock-staking-pool-contract`).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"
if command -v make >/dev/null 2>&1; then
  make staking-contract mock-staking-pool-contract
else
  echo "make not found; run from repo root: make staking-contract mock-staking-pool-contract" >&2
  exit 1
fi

echo "Built:"
echo "  $REPO_ROOT/res/local/staking_contract.wasm"
echo "  $REPO_ROOT/res/local/mock_staking_pool_contract.wasm"
