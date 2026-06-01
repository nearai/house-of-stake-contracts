#!/usr/bin/env bash
#
# Deploy or upgrade staking-contract on NEAR testnet.
#
# Prerequisites:
#   - near CLI installed and keychain has the signer account(s).
#   - jq installed for deployment init JSON.
#   - WASM built at res/local/staking_contract.wasm, or run with BUILD_WASM=1.
#
# Usage:
#   BUILD_WASM=1 ./scripts/deploy_testnet_staking_contract.sh <staking-account.testnet>
#   ACTION=upgrade ./scripts/deploy_testnet_staking_contract.sh <staking-account.testnet>
#
# Common environment:
#   CHAIN_ID=testnet
#   ACTION=deploy | upgrade              # default: deploy
#   STAKING_WASM=res/local/staking_contract.wasm
#   OWNER_ACCOUNT_ID=<owner.testnet>      # default: staking account
#   GUARDIANS_JSON='["guardian.testnet"]' # default: []
#   DRY_RUN=1                             # print commands without sending txs
#
# Optional account creation for fresh testnet subaccounts:
#   CREATE_ACCOUNT=1 PARENT_ACCOUNT_ID=<parent.testnet> FUND_STAKING_NEAR='8 NEAR' ...
#
# Testnet config defaults are intentionally testing-friendly. Review every value before
# using this script for production-like deployment.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

usage() {
  sed -n '2,32p' "$0" >&2
}

STAKING_ACCOUNT_ID="${1:-${STAKING_ACCOUNT_ID:-}}"
if [[ -z "$STAKING_ACCOUNT_ID" ]]; then
  usage
  exit 1
fi

: "${CHAIN_ID:=testnet}"
: "${ACTION:=deploy}"
: "${STAKING_WASM:=$REPO_ROOT/res/local/staking_contract.wasm}"
: "${OWNER_ACCOUNT_ID:=$STAKING_ACCOUNT_ID}"
: "${GUARDIANS_JSON:=[]}"

: "${CREATE_ACCOUNT:=0}"
: "${PARENT_ACCOUNT_ID:=}"
: "${FUND_STAKING_NEAR:=8 NEAR}"

: "${MIN_LOCK_DURATION_NS:=1}"
: "${MAX_LOCK_DURATION_NS:=63072000000000000}"
: "${EPOCH_UNSTAKE_SETTLE_EPOCHS:=1}"
: "${MIN_STORAGE_DEPOSIT_YOCTO:=10000000000000000000000}"
: "${PER_LOCK_STORAGE_STAKE_YOCTO:=0}"
: "${MIN_LOCK_AMOUNT_YOCTO:=1000000000000000000000000}"

if [[ "$ACTION" != "deploy" && "$ACTION" != "upgrade" ]]; then
  echo "ACTION must be deploy or upgrade; got: $ACTION" >&2
  exit 1
fi

if ! command -v near >/dev/null 2>&1; then
  echo "near CLI not found in PATH." >&2
  exit 1
fi

if [[ "${BUILD_WASM:-0}" == "1" ]]; then
  "$REPO_ROOT/scripts/build_staking_wasm.sh"
fi

if [[ ! -f "$STAKING_WASM" ]]; then
  echo "Missing staking WASM: $STAKING_WASM" >&2
  echo "Run: BUILD_WASM=1 $0 $STAKING_ACCOUNT_ID" >&2
  exit 1
fi

if [[ "$ACTION" == "deploy" ]]; then
  if ! command -v jq >/dev/null 2>&1; then
    echo "jq not found in PATH; required for deploy init JSON." >&2
    exit 1
  fi
  if ! printf '%s' "$GUARDIANS_JSON" | jq -e 'type == "array"' >/dev/null; then
    echo "GUARDIANS_JSON must be a JSON array, got: $GUARDIANS_JSON" >&2
    exit 1
  fi
fi

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "${DRY_RUN:-0}" != "1" ]]; then
    "$@"
  fi
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

echo "== House of Stake staking-contract testnet deploy =="
echo "Action:             $ACTION"
echo "Network:            $CHAIN_ID"
echo "Staking account:    $STAKING_ACCOUNT_ID"
echo "Owner account:      $OWNER_ACCOUNT_ID"
echo "WASM:               $STAKING_WASM"
echo "WASM sha256:        $(wasm_hash)"
echo

if [[ "$CHAIN_ID" != "testnet" ]]; then
  echo "Refusing to run: this script is for testnet only. Set CHAIN_ID=testnet." >&2
  exit 1
fi

if [[ "$CREATE_ACCOUNT" == "1" ]]; then
  if [[ -z "$PARENT_ACCOUNT_ID" ]]; then
    echo "CREATE_ACCOUNT=1 requires PARENT_ACCOUNT_ID=<parent.testnet>." >&2
    exit 1
  fi
  echo "== Creating staking account =="
  run near --quiet account create-account fund-myself "$STAKING_ACCOUNT_ID" "$FUND_STAKING_NEAR" \
    autogenerate-new-keypair save-to-keychain sign-as "$PARENT_ACCOUNT_ID" \
    network-config "$CHAIN_ID" sign-with-keychain send
  echo
fi

if [[ "$ACTION" == "deploy" ]]; then
  staking_init=$(
    jq -n \
      --arg owner "$OWNER_ACCOUNT_ID" \
      --argjson guardians "$GUARDIANS_JSON" \
      --arg min_lock_d "$MIN_LOCK_DURATION_NS" \
      --arg max_lock_d "$MAX_LOCK_DURATION_NS" \
      --argjson epoch_unstake "$EPOCH_UNSTAKE_SETTLE_EPOCHS" \
      --arg min_storage "$MIN_STORAGE_DEPOSIT_YOCTO" \
      --arg per_lock "$PER_LOCK_STORAGE_STAKE_YOCTO" \
<<<<<<< HEAD
      --arg per_purchase "0" \
=======
>>>>>>> origin/feat/stake-dao
      --arg min_lock_amt "$MIN_LOCK_AMOUNT_YOCTO" \
      '{
        config: {
          owner_account_id: $owner,
          proposed_new_owner_account_id: null,
          guardians: $guardians,
          min_lock_duration_ns: $min_lock_d,
          max_lock_duration_ns: $max_lock_d,
          epoch_unstake_settle_epochs: $epoch_unstake,
          min_storage_deposit: $min_storage,
          per_lock_storage_stake: $per_lock,
<<<<<<< HEAD
          per_purchase_storage_stake: $per_purchase,
=======
>>>>>>> origin/feat/stake-dao
          min_lock_amount: $min_lock_amt
        }
      }'
  )

  echo "== Deploying and initializing staking-contract =="
  echo "$staking_init" | jq .
  run near --quiet contract deploy "$STAKING_ACCOUNT_ID" use-file "$STAKING_WASM" \
    with-init-call new json-args "$staking_init" \
    prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
    network-config "$CHAIN_ID" sign-with-keychain send
else
  echo "== Upgrading staking-contract through owner-gated upgrade() =="
  echo "The owner account must match get_config.owner_account_id on-chain."
  run near --quiet contract call-function as-transaction "$STAKING_ACCOUNT_ID" upgrade \
    file-args "$STAKING_WASM" \
    prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
    sign-as "$OWNER_ACCOUNT_ID" network-config "$CHAIN_ID" sign-with-keychain send
fi

echo
echo "== Post-deploy checks =="
run near contract call-function as-read-only "$STAKING_ACCOUNT_ID" get_version \
  json-args '{}' network-config "$CHAIN_ID" now
run near contract call-function as-read-only "$STAKING_ACCOUNT_ID" get_config \
  json-args '{}' network-config "$CHAIN_ID" now

echo
echo "Done."
echo "Useful next steps:"
echo "  Add validator: add_validator({ validator_id }) signed by $OWNER_ACCOUNT_ID with 1 yoctoNEAR"
echo "  Create catalog: create_product/create_price signed by each pool owner with 1 yoctoNEAR"
