#!/usr/bin/env bash
#
# Directly replace the code on an existing testnet staking-contract account with
# the test-feature staking WASM. This does NOT call the contract's owner-gated
# upgrade() method and does NOT run initialization; existing contract state is
# preserved by NEAR account storage.
#
# This is intended only for disposable / E2E testnet accounts. The test-feature
# WASM exposes test clock methods such as set_block_timestamp.
#
# Usage:
#   BUILD_WASM=1 CONFIRM_TEST_ONLY_DEPLOY=1 \
#     ./staking-contract/scripts/deploy_testnet_staking_test_code.sh hos-e2e-0601144939.testnet
#
# Environment:
#   CHAIN_ID=testnet
#   STAKING_WASM=res/local/staking_contract_test.wasm
#   BUILD_WASM=1                 # run `make staking-contract-test` first
#   DRY_RUN=1                    # print commands without sending txs
#   CONFIRM_TEST_ONLY_DEPLOY=1   # required unless DRY_RUN=1
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

usage() {
  sed -n '2,28p' "$0" >&2
}

STAKING_ACCOUNT_ID="${1:-${STAKING_ACCOUNT_ID:-}}"
if [[ -z "$STAKING_ACCOUNT_ID" ]]; then
  usage
  exit 1
fi

: "${CHAIN_ID:=testnet}"
: "${STAKING_WASM:=$REPO_ROOT/res/local/staking_contract_test.wasm}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 not found in PATH." >&2
    exit 1
  fi
}

require_command near

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "${DRY_RUN:-0}" != "1" ]]; then
    "$@"
  fi
}

if [[ "${BUILD_WASM:-0}" == "1" ]]; then
  if command -v make >/dev/null 2>&1; then
    make staking-contract-test
  else
    echo "make not found; build the test WASM first with: make staking-contract-test" >&2
    exit 1
  fi
fi

if [[ ! -f "$STAKING_WASM" ]]; then
  echo "Missing test staking WASM: $STAKING_WASM" >&2
  echo "Build it with: make staking-contract-test" >&2
  exit 1
fi

if [[ "${DRY_RUN:-0}" != "1" && "${CONFIRM_TEST_ONLY_DEPLOY:-0}" != "1" ]]; then
  cat >&2 <<EOF
Refusing to deploy test-feature code without explicit confirmation.

Target account: $STAKING_ACCOUNT_ID
WASM:           $STAKING_WASM
Network:        $CHAIN_ID

This directly replaces contract code on the target account and exposes
test-only methods. Re-run with CONFIRM_TEST_ONLY_DEPLOY=1 if this is intended.
EOF
  exit 1
fi

echo "== Direct test-feature staking code deploy =="
echo "Target account: $STAKING_ACCOUNT_ID"
echo "WASM:           $STAKING_WASM"
echo "Network:        $CHAIN_ID"
echo

run near contract deploy "$STAKING_ACCOUNT_ID" use-file "$STAKING_WASM" without-init-call \
  network-config "$CHAIN_ID" sign-with-keychain send

echo
echo "== Post-deploy checks =="
run near contract call-function as-read-only "$STAKING_ACCOUNT_ID" get_version \
  json-args '{}' network-config "$CHAIN_ID" now
run near contract call-function as-read-only "$STAKING_ACCOUNT_ID" get_block_timestamp \
  json-args '{}' network-config "$CHAIN_ID" now

echo
echo "Done. The target account is now running the test-feature staking WASM."
