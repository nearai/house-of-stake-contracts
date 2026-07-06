#!/usr/bin/env bash
#
# Deploy the shared testnet staking contract code with the PriceMetadata
# compatibility fix and verify legacy subscription price records can be read.
#
# Preview:
#   ./scripts/upgrade_testnet_staking_price_metadata_fix.sh
#
# Execute:
#   EXECUTE=1 ./scripts/upgrade_testnet_staking_price_metadata_fix.sh
#
# Optional environment:
#   STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
#   OWNER_ACCOUNT_ID=$STAKING_ACCOUNT_ID
#   CHAIN_ID=testnet
#   STAKING_WASM=res/local/staking_contract.wasm
#   BUILD_WASM=1
#   RUN_TESTS=0
#   PRICE_IDS="price_RjiajH4KEZ43w68DgY5xVaVU price_h577VYQUEynPA3uQt1u1neGn price_7EAls0E844ULR06EEl53fQoI"
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

: "${CHAIN_ID:=testnet}"
: "${STAKING_ACCOUNT_ID:=hos-e2e-0601144939.testnet}"
: "${OWNER_ACCOUNT_ID:=$STAKING_ACCOUNT_ID}"
: "${STAKING_WASM:=$REPO_ROOT/res/local/staking_contract.wasm}"
: "${BUILD_WASM:=1}"
: "${RUN_TESTS:=0}"
: "${EXECUTE:=0}"
: "${PRICE_IDS:=price_RjiajH4KEZ43w68DgY5xVaVU price_h577VYQUEynPA3uQt1u1neGn price_7EAls0E844ULR06EEl53fQoI}"

if [[ "$CHAIN_ID" != "testnet" ]]; then
  echo "Refusing to run: this script is testnet-only. Set CHAIN_ID=testnet." >&2
  exit 1
fi

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 not found in PATH." >&2
    exit 1
  fi
}

require_command near
require_command jq

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "$EXECUTE" == "1" ]]; then
    "$@"
  fi
}

view_json() {
  local contract_id="$1"
  local method_name="$2"
  local args_json="$3"

  near contract call-function as-read-only "$contract_id" "$method_name" \
    json-args "$args_json" network-config "$CHAIN_ID" now
}

wasm_hash() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$STAKING_WASM" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$STAKING_WASM" | awk '{print $1}'
  else
    echo "sha256 unavailable"
  fi
}

check_price() {
  local price_id="$1"
  local price_json
  price_json="$(view_json "$STAKING_ACCOUNT_ID" get_price "{\"price_id\":\"$price_id\"}")"
  jq -e '.price_id and .metadata' >/dev/null <<<"$price_json"
  echo "$price_json" | jq '{price_id, product_id, price_type, metadata}'
}

if [[ "$RUN_TESTS" == "1" ]]; then
  require_command cargo
  echo "== Running focused compatibility tests =="
  cargo test -p staking-contract vprice_ -- --nocapture
  echo
fi

if [[ "$BUILD_WASM" == "1" ]]; then
  echo "== Building latest staking-contract WASM =="
  "$REPO_ROOT/scripts/build_staking_wasm.sh"
  echo
fi

if [[ ! -f "$STAKING_WASM" ]]; then
  echo "Missing staking WASM: $STAKING_WASM" >&2
  echo "Run with BUILD_WASM=1 or build it with: $REPO_ROOT/scripts/build_staking_wasm.sh" >&2
  exit 1
fi

echo "== Pre-deploy on-chain checks =="
before_version="$(view_json "$STAKING_ACCOUNT_ID" get_version '{}')"
config_json="$(view_json "$STAKING_ACCOUNT_ID" get_config '{}')"
on_chain_owner="$(jq -er '.owner_account_id' <<<"$config_json")"

echo "Network:          $CHAIN_ID"
echo "Contract:         $STAKING_ACCOUNT_ID"
echo "Current version:  $before_version"
echo "On-chain owner:   $on_chain_owner"
echo "Keychain signer:  $STAKING_ACCOUNT_ID"
echo "WASM:             $STAKING_WASM"
echo "WASM sha256:      $(wasm_hash)"
echo "Price IDs:        $PRICE_IDS"
echo

if [[ "$OWNER_ACCOUNT_ID" != "$on_chain_owner" ]]; then
  echo "OWNER_ACCOUNT_ID must match get_config.owner_account_id." >&2
  exit 1
fi

echo "== Current price readability =="
for price_id in $PRICE_IDS; do
  echo "-- $price_id"
  if check_price "$price_id"; then
    echo "readable before upgrade"
  else
    echo "not readable before upgrade; expected on affected contracts"
  fi
done
echo

if [[ "$EXECUTE" != "1" ]]; then
  echo "Preview only. Re-run with EXECUTE=1 to deploy the contract code."
  echo
fi

echo "== Deploy contract code without init =="
run near contract deploy "$STAKING_ACCOUNT_ID" \
  use-file "$STAKING_WASM" \
  without-init-call \
  network-config "$CHAIN_ID" sign-with-keychain send

if [[ "$EXECUTE" == "1" ]]; then
  echo
  echo "== Post-deploy checks =="
  view_json "$STAKING_ACCOUNT_ID" get_version '{}'
  for price_id in $PRICE_IDS; do
    echo "-- $price_id"
    check_price "$price_id"
  done
else
  echo
  echo "No transaction sent."
fi
