#!/usr/bin/env bash
#
# Deploy mock staking pool contract(s) + staking-contract to NEAR testnet for manual / integration testing.
#
# Prerequisites:
#   - near CLI installed (https://docs.near.org/tools/near-cli)
#   - Logged in for testnet:  near account import-account …  OR  keys for your parent account in keychain
#   - jq installed (for JSON init payloads)
#   - WASM files in res/local/ (run ./scripts/build_staking_test_wasms.sh first)
#
# Usage:
#   ./scripts/deploy_testnet_staking_stack.sh <parent.testnet>
#
# Example:
#   ./scripts/build_staking_test_wasms.sh
#   ./scripts/deploy_testnet_staking_stack.sh mylab.testnet
#
# Environment (optional):
#   CHAIN_ID=testnet
#   NUM_POOLS=2                    # mock-pool-0.<parent>, …
#   STAKING_ACCOUNT_ID=…          # default: house-stake.<parent>
#   OWNER_ACCOUNT_ID=…            # default: stake-owner.<parent>  (staking contract owner; signs add_validator)
#   VALIDATOR_OWNER_ACCOUNT_ID=…  # default: stake-validator-owner.<parent> (mock pool get_owner_id; signs create_product)
#   STAKING_WASM / MOCK_POOL_WASM # override paths to .wasm files
#   SKIP_ACCOUNT_CREATE=1         # skip subaccount creation (reuse existing accounts)
#   FUND_OWNER_NEAR / FUND_VALIDATOR_OWNER_NEAR / FUND_STAKING_NEAR / FUND_POOL_NEAR
#
# Pool work on the staking contract is user-driven (lock / unlock / withdraw / epoch_settle);
# there is no operators list in Config — see staking-contract/docs/LAZY_EPOCH_PIPELINE.md.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ROOT_ACCOUNT_ID="${1:-}"
if [[ -z "$ROOT_ACCOUNT_ID" ]]; then
  echo "Usage: $0 <parent_account.testnet>" >&2
  echo "  parent must be able to create subaccounts and pay for them (sign-as parent)." >&2
  exit 1
fi

if ! command -v near >/dev/null 2>&1; then
  echo "near CLI not found in PATH." >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq not found in PATH (needed for init JSON)." >&2
  exit 1
fi

: "${CHAIN_ID:=testnet}"
: "${NUM_POOLS:=2}"

: "${STAKING_ACCOUNT_ID:=house-stake.$ROOT_ACCOUNT_ID}"
: "${OWNER_ACCOUNT_ID:=stake-owner.$ROOT_ACCOUNT_ID}"
: "${VALIDATOR_OWNER_ACCOUNT_ID:=stake-validator-owner.$ROOT_ACCOUNT_ID}"

: "${STAKING_WASM:=$REPO_ROOT/res/local/staking_contract.wasm}"
: "${MOCK_POOL_WASM:=$REPO_ROOT/res/local/mock_staking_pool_contract.wasm}"

: "${FUND_OWNER_NEAR:=2 NEAR}"
: "${FUND_VALIDATOR_OWNER_NEAR:=1 NEAR}"
: "${FUND_STAKING_NEAR:=6 NEAR}"
: "${FUND_POOL_NEAR:=3 NEAR}"

if [[ ! -f "$STAKING_WASM" || ! -f "$MOCK_POOL_WASM" ]]; then
  echo "Missing WASM. Expected:" >&2
  echo "  $STAKING_WASM" >&2
  echo "  $MOCK_POOL_WASM" >&2
  echo "Run:  $REPO_ROOT/scripts/build_staking_test_wasms.sh" >&2
  exit 1
fi

if [[ "$NUM_POOLS" -lt 1 ]]; then
  echo "NUM_POOLS must be >= 1" >&2
  exit 1
fi

POOL_IDS=()
for ((i = 0; i < NUM_POOLS; i++)); do
  POOL_IDS+=("mock-pool-${i}.$ROOT_ACCOUNT_ID")
done

echo "== House of Stake — testnet staking stack =="
echo "Parent (pays / signs account creation): $ROOT_ACCOUNT_ID"
echo "Staking contract account:               $STAKING_ACCOUNT_ID"
echo "Staking owner (allowlist / governance): $OWNER_ACCOUNT_ID"
echo "Validator owner (pool get_owner_id):  $VALIDATOR_OWNER_ACCOUNT_ID"
echo "Mock pool accounts:                    ${POOL_IDS[*]}"
echo "Network:                              $CHAIN_ID"
echo

if [[ "${SKIP_ACCOUNT_CREATE:-}" != "1" ]]; then
  echo "== Creating subaccounts (sign-as $ROOT_ACCOUNT_ID) =="

  echo "Creating $OWNER_ACCOUNT_ID ($FUND_OWNER_NEAR)"
  near --quiet account create-account fund-myself "$OWNER_ACCOUNT_ID" "$FUND_OWNER_NEAR" \
    autogenerate-new-keypair save-to-keychain sign-as "$ROOT_ACCOUNT_ID" \
    network-config "$CHAIN_ID" sign-with-keychain send

  echo "Creating $VALIDATOR_OWNER_ACCOUNT_ID ($FUND_VALIDATOR_OWNER_NEAR)"
  near --quiet account create-account fund-myself "$VALIDATOR_OWNER_ACCOUNT_ID" "$FUND_VALIDATOR_OWNER_NEAR" \
    autogenerate-new-keypair save-to-keychain sign-as "$ROOT_ACCOUNT_ID" \
    network-config "$CHAIN_ID" sign-with-keychain send

  echo "Creating $STAKING_ACCOUNT_ID ($FUND_STAKING_NEAR)"
  near --quiet account create-account fund-myself "$STAKING_ACCOUNT_ID" "$FUND_STAKING_NEAR" \
    autogenerate-new-keypair save-to-keychain sign-as "$ROOT_ACCOUNT_ID" \
    network-config "$CHAIN_ID" sign-with-keychain send

  for pid in "${POOL_IDS[@]}"; do
    echo "Creating $pid ($FUND_POOL_NEAR)"
    near --quiet account create-account fund-myself "$pid" "$FUND_POOL_NEAR" \
      autogenerate-new-keypair save-to-keychain sign-as "$ROOT_ACCOUNT_ID" \
      network-config "$CHAIN_ID" sign-with-keychain send
  done
else
  echo "SKIP_ACCOUNT_CREATE=1 — assuming accounts already exist and keys are in keychain."
fi

echo
echo "== Deploying mock staking pool(s) (init: new { owner_id }) =="
for pid in "${POOL_IDS[@]}"; do
  pool_init=$(jq -n --arg owner "$VALIDATOR_OWNER_ACCOUNT_ID" '{owner_id: $owner}')
  near --quiet contract deploy "$pid" use-file "$MOCK_POOL_WASM" \
    with-init-call new json-args "$pool_init" \
    prepaid-gas '50.0 Tgas' attached-deposit '0 NEAR' \
    network-config "$CHAIN_ID" sign-with-keychain send
done

echo
echo "== Deploying staking-contract (init: new { config }) =="
# Defaults mirror staking-contract/tests/mock_pool/mod.rs staking_new_args_e2e (test-friendly).
: "${MIN_LOCK_DURATION_NS:=1}"
: "${MAX_LOCK_DURATION_NS:=10000000000000000000}"
: "${EPOCH_UNSTAKE_SETTLE_EPOCHS:=1}"
: "${MIN_STORAGE_DEPOSIT_YOCTO:=10000000000000000000000}"
: "${PER_LOCK_STORAGE_STAKE_YOCTO:=0}"
: "${MIN_LOCK_AMOUNT_YOCTO:=1000000000000000000000000}"

staking_init=$(
  jq -n \
    --arg owner "$OWNER_ACCOUNT_ID" \
    --arg min_lock_d "$MIN_LOCK_DURATION_NS" \
    --arg max_lock_d "$MAX_LOCK_DURATION_NS" \
    --argjson epoch_unstake "$EPOCH_UNSTAKE_SETTLE_EPOCHS" \
    --arg min_storage "$MIN_STORAGE_DEPOSIT_YOCTO" \
    --arg per_lock "$PER_LOCK_STORAGE_STAKE_YOCTO" \
    --arg min_lock_amt "$MIN_LOCK_AMOUNT_YOCTO" \
    '{
      config: {
        owner_account_id: $owner,
        proposed_new_owner_account_id: null,
        guardians: [],
        min_lock_duration_ns: $min_lock_d,
        max_lock_duration_ns: $max_lock_d,
        epoch_unstake_settle_epochs: $epoch_unstake,
        min_storage_deposit: $min_storage,
        per_lock_storage_stake: $per_lock,
        min_lock_amount: $min_lock_amt
      }
    }'
)

near --quiet contract deploy "$STAKING_ACCOUNT_ID" use-file "$STAKING_WASM" \
  with-init-call new json-args "$staking_init" \
  prepaid-gas '100.0 Tgas' attached-deposit '0 NEAR' \
  network-config "$CHAIN_ID" sign-with-keychain send

echo
echo "== Allowlisting mock pool(s) on staking (owner + 1 yocto each) =="
for pid in "${POOL_IDS[@]}"; do
  validator_arg=$(jq -n --arg v "$pid" '{validator_id: $v}')
  near --quiet contract call-function as-transaction "$STAKING_ACCOUNT_ID" add_validator \
    json-args "$validator_arg" \
    prepaid-gas '50.0 Tgas' attached-deposit '1 yoctoNEAR' \
    sign-as "$OWNER_ACCOUNT_ID" network-config "$CHAIN_ID" sign-with-keychain send
done

echo
echo "== Done =="
echo "View staking config:"
echo "  near contract call-function as-read-only $STAKING_ACCOUNT_ID get_config json-args {} network-config $CHAIN_ID now"
echo "View pool owner:"
echo "  near contract call-function as-read-only <pool> get_owner_id json-args {} network-config $CHAIN_ID now"
echo
echo "Example — create a product (must sign as validator owner; pool get_owner_id must match):"
echo "  near contract call-function as-transaction $STAKING_ACCOUNT_ID create_product json-args \\"
echo "    '{\"validator_id\": \"${POOL_IDS[0]}\", \"name\": \"Test\", \"description\": \"\"}' \\"
echo "    prepaid-gas '200.0 Tgas' attached-deposit '1 yoctoNEAR' \\"
echo "    sign-as $VALIDATOR_OWNER_ACCOUNT_ID network-config $CHAIN_ID sign-with-keychain send"
echo
echo "Export for other scripts:"
echo "export ROOT_ACCOUNT_ID=$ROOT_ACCOUNT_ID"
echo "export STAKING_ACCOUNT_ID=$STAKING_ACCOUNT_ID"
echo "export OWNER_ACCOUNT_ID=$OWNER_ACCOUNT_ID"
echo "export VALIDATOR_OWNER_ACCOUNT_ID=$VALIDATOR_OWNER_ACCOUNT_ID"
echo "export MOCK_POOL_IDS=\"${POOL_IDS[*]}\""
i=0
for pid in "${POOL_IDS[@]}"; do
  echo "export MOCK_POOL_${i}=$pid"
  i=$((i + 1))
done
