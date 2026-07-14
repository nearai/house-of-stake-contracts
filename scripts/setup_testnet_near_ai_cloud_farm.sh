#!/usr/bin/env bash
#
# Set up the NEAR AI Cloud staking farm product on the existing testnet
# staking-contract.
#
# Preview:
#   FARM_REWARD_RATE=3858024691358024 ./scripts/setup_testnet_near_ai_cloud_farm.sh
#
# Or derive FARM_REWARD_RATE from APY and price assumptions:
#   TARGET_APY_BPS=500 NEAR_PRICE_USD_CENTS=200 CREDIT_PRICE_USD_CENTS=100 \
#     ./scripts/setup_testnet_near_ai_cloud_farm.sh
#
# Execute:
#   FARM_REWARD_RATE=3858024691358024 EXECUTE=1 ./scripts/setup_testnet_near_ai_cloud_farm.sh
#
# Optional environment:
#   STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
#   VALIDATOR_ID=mock-pool-0.hos-e2e-0601144939.testnet
#   OWNER_ACCOUNT_ID=$STAKING_ACCOUNT_ID
#   VALIDATOR_OWNER_ACCOUNT_ID=$STAKING_ACCOUNT_ID
#   CHAIN_ID=testnet
#   PRODUCT_NAME='NEAR AI Cloud'
#   PRODUCT_DESCRIPTION='Stake NEAR to earn NEAR AI Cloud rewards'
#   PRICE_NAME='NEAR AI Cloud Farm'
#   PRICE_DESCRIPTION='Stake NEAR in the NEAR AI Cloud farm'
#   MIN_FARM_STAKE_YOCTO=1000000000000000000000000
#   MAX_FARM_STAKE_YOCTO=10000000000000000000000000
#   FARM_REWARD_RATE=<integer reward units per second per staked NEAR>
#   TARGET_APY_BPS=500                  # 5.00%; used only when FARM_REWARD_RATE is unset
#   NEAR_PRICE_USD_CENTS=200            # $2.00; used only with TARGET_APY_BPS
#   CREDIT_PRICE_USD_CENTS=100          # $1.00; used only with TARGET_APY_BPS
#   REWARD_SECONDS_PER_YEAR=31536000    # 365 days
#   PRODUCT_SCAN_LIMIT=200
#
# Reward-rate formula:
#   credits_per_near_year = (TARGET_APY_BPS / 10000) * NEAR_PRICE_USD / CREDIT_PRICE_USD
#   FARM_REWARD_RATE = floor(credits_per_near_year * 1e24 / REWARD_SECONDS_PER_YEAR)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

: "${CHAIN_ID:=testnet}"
: "${STAKING_ACCOUNT_ID:=hos-e2e-0601144939.testnet}"
: "${VALIDATOR_ID:=mock-pool-0.hos-e2e-0601144939.testnet}"
: "${OWNER_ACCOUNT_ID:=$STAKING_ACCOUNT_ID}"
: "${VALIDATOR_OWNER_ACCOUNT_ID:=$STAKING_ACCOUNT_ID}"
: "${PRODUCT_NAME:=NEAR AI Cloud Credits}"
: "${PRODUCT_DESCRIPTION:=Stake NEAR to earn NEAR AI Cloud credits}"
: "${PRICE_NAME:=NEAR AI Cloud Staking Farm}"
: "${PRICE_DESCRIPTION:=Stake NEAR in the NEAR AI Cloud staking farm}"
: "${MIN_FARM_STAKE_YOCTO:=1000000000000000000000000}"
: "${MAX_FARM_STAKE_YOCTO:=}"
: "${TARGET_APY_BPS:=}"
: "${NEAR_PRICE_USD_CENTS:=}"
: "${CREDIT_PRICE_USD_CENTS:=}"
: "${REWARD_SECONDS_PER_YEAR:=31536000}"
: "${PRODUCT_SCAN_LIMIT:=200}"
: "${EXECUTE:=0}"

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

calculate_farm_reward_rate() {
  require_command python3

  python3 - "$TARGET_APY_BPS" "$NEAR_PRICE_USD_CENTS" "$CREDIT_PRICE_USD_CENTS" "$REWARD_SECONDS_PER_YEAR" <<'PY'
import sys

apy_bps, near_cents, credit_cents, seconds_per_year = map(int, sys.argv[1:])
if apy_bps <= 0:
    raise SystemExit("TARGET_APY_BPS must be positive")
if near_cents <= 0:
    raise SystemExit("NEAR_PRICE_USD_CENTS must be positive")
if credit_cents <= 0:
    raise SystemExit("CREDIT_PRICE_USD_CENTS must be positive")
if seconds_per_year <= 0:
    raise SystemExit("REWARD_SECONDS_PER_YEAR must be positive")

rate = (apy_bps * near_cents * 10**24) // (10_000 * credit_cents * seconds_per_year)
print(rate)
PY
}

if [[ -z "${FARM_REWARD_RATE:-}" ]]; then
  if [[ -n "$TARGET_APY_BPS" || -n "$NEAR_PRICE_USD_CENTS" || -n "$CREDIT_PRICE_USD_CENTS" ]]; then
    if [[ -z "$TARGET_APY_BPS" || -z "$NEAR_PRICE_USD_CENTS" || -z "$CREDIT_PRICE_USD_CENTS" ]]; then
      echo "TARGET_APY_BPS, NEAR_PRICE_USD_CENTS, and CREDIT_PRICE_USD_CENTS must all be set to derive FARM_REWARD_RATE." >&2
      exit 1
    fi
    FARM_REWARD_RATE="$(calculate_farm_reward_rate)"
    FARM_REWARD_RATE_SOURCE="derived from TARGET_APY_BPS=$TARGET_APY_BPS, NEAR_PRICE_USD_CENTS=$NEAR_PRICE_USD_CENTS, CREDIT_PRICE_USD_CENTS=$CREDIT_PRICE_USD_CENTS, REWARD_SECONDS_PER_YEAR=$REWARD_SECONDS_PER_YEAR"
  else
    echo "FARM_REWARD_RATE is required, or derive it with TARGET_APY_BPS, NEAR_PRICE_USD_CENTS, and CREDIT_PRICE_USD_CENTS." >&2
    echo "Unit: 24-decimal reward units per second per staked NEAR." >&2
    exit 1
  fi
else
  FARM_REWARD_RATE_SOURCE="provided directly"
fi

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "$EXECUTE" == "1" ]]; then
    "$@"
  fi
}

near_tx() {
  local signer="$1"
  local contract_id="$2"
  local method_name="$3"
  local args_json="$4"

  run near --quiet contract call-function as-transaction "$contract_id" "$method_name" \
    json-args "$args_json" \
    prepaid-gas '200.0 Tgas' attached-deposit '1 yoctoNEAR' \
    sign-as "$signer" network-config "$CHAIN_ID" sign-with-keychain send
}

view_json() {
  local contract_id="$1"
  local method_name="$2"
  local args_json="$3"

  near --quiet contract call-function as-read-only "$contract_id" "$method_name" \
    json-args "$args_json" network-config "$CHAIN_ID" now
}

get_validator_json() {
  view_json "$STAKING_ACCOUNT_ID" get_validator "$(jq -n --arg v "$VALIDATOR_ID" '{validator_id:$v}')"
}

find_product_id() {
  view_json "$STAKING_ACCOUNT_ID" get_products "$(jq -n --argjson limit "$PRODUCT_SCAN_LIMIT" '{from_index:0, limit:$limit}')" \
    | jq -er --arg validator "$VALIDATOR_ID" --arg name "$PRODUCT_NAME" \
      '[.[] | select(.validator_id == $validator and .name == $name and .status == "Active")] | last.product_id'
}

find_active_farm_price_json() {
  local product_id="$1"
  local price_id price_json

  while IFS= read -r price_id; do
    [[ -z "$price_id" || "$price_id" == "null" ]] && continue
    price_json="$(view_json "$STAKING_ACCOUNT_ID" get_price "$(jq -n --arg p "$price_id" '{price_id:$p}')")"
    if jq -e '.price_type == "Farm" and .status == "Active"' >/dev/null <<<"$price_json"; then
      printf '%s\n' "$price_json"
      return 0
    fi
  done < <(view_json "$STAKING_ACCOUNT_ID" get_product "$(jq -n --arg p "$product_id" '{product_id:$p}')" | jq -r '.price_ids[]?')

  return 1
}

farm_price_matches() {
  local price_json="$1"
  local expected_max

  if [[ -n "$MAX_FARM_STAKE_YOCTO" ]]; then
    expected_max="\"$MAX_FARM_STAKE_YOCTO\""
  else
    expected_max="null"
  fi

  jq -e \
    --arg name "$PRICE_NAME" \
    --arg description "$PRICE_DESCRIPTION" \
    --arg amount "$MIN_FARM_STAKE_YOCTO" \
    --arg reward_rate "$FARM_REWARD_RATE" \
    --argjson max_amount "$expected_max" \
    '.name == $name
      and .description == $description
      and .amount == $amount
      and .price_type == "Farm"
      and .billing_period == null
      and .lock_factor_near_months == "0"
      and .metadata.max_amount == $max_amount
      and .metadata.farm_reward_rate == $reward_rate
      and .status == "Active"' \
    >/dev/null <<<"$price_json"
}

build_product_arg() {
  jq -n \
    --arg validator "$VALIDATOR_ID" \
    --arg name "$PRODUCT_NAME" \
    --arg description "$PRODUCT_DESCRIPTION" \
    '{validator_id:$validator, name:$name, description:$description}'
}

build_price_arg() {
  local product_id="$1"

  jq -n \
    --arg product_id "$product_id" \
    --arg name "$PRICE_NAME" \
    --arg description "$PRICE_DESCRIPTION" \
    --arg amount "$MIN_FARM_STAKE_YOCTO" \
    --arg reward_rate "$FARM_REWARD_RATE" \
    --arg max_amount "$MAX_FARM_STAKE_YOCTO" \
    '{
      product_id: $product_id,
      name: $name,
      description: $description,
      amount: $amount,
      price_type: "Farm",
      billing_period: null,
      lock_factor_near_months: "0",
      metadata: {
        max_amount: (if $max_amount == "" then null else $max_amount end),
        farm_reward_rate: $reward_rate
      }
    }'
}

echo "== NEAR AI Cloud staking farm setup =="
echo "Network:          $CHAIN_ID"
echo "Contract:         $STAKING_ACCOUNT_ID"
echo "Validator:        $VALIDATOR_ID"
echo "Contract owner:   $OWNER_ACCOUNT_ID"
echo "Validator owner:  $VALIDATOR_OWNER_ACCOUNT_ID"
echo "Product:          $PRODUCT_NAME"
echo "Farm price:       $PRICE_NAME"
echo "Min stake yocto:  $MIN_FARM_STAKE_YOCTO"
echo "Max stake yocto:  ${MAX_FARM_STAKE_YOCTO:-null}"
echo "Reward rate:      $FARM_REWARD_RATE"
echo "Reward source:    $FARM_REWARD_RATE_SOURCE"
echo

config_json="$(view_json "$STAKING_ACCOUNT_ID" get_config '{}')"
on_chain_owner="$(jq -er '.owner_account_id' <<<"$config_json")"
if [[ "$OWNER_ACCOUNT_ID" != "$on_chain_owner" ]]; then
  echo "OWNER_ACCOUNT_ID must match get_config.owner_account_id to add validators." >&2
  exit 1
fi

pool_owner="$(view_json "$VALIDATOR_ID" get_owner_id '{}' | jq -er '.')"
if [[ "$VALIDATOR_OWNER_ACCOUNT_ID" != "$pool_owner" ]]; then
  echo "VALIDATOR_OWNER_ACCOUNT_ID must match $VALIDATOR_ID get_owner_id." >&2
  echo "Expected: $pool_owner" >&2
  exit 1
fi

if [[ "$EXECUTE" != "1" ]]; then
  echo "Preview only. Re-run with EXECUTE=1 to send transactions."
  echo
fi

echo "== Ensure validator is allowlisted =="
validator_json="$(get_validator_json)"
if [[ "$validator_json" == "null" ]]; then
  near_tx "$OWNER_ACCOUNT_ID" "$STAKING_ACCOUNT_ID" add_validator \
    "$(jq -n --arg validator "$VALIDATOR_ID" '{validator_id:$validator}')"
else
  echo "Validator already allowlisted: $VALIDATOR_ID"
fi
echo

echo "== Ensure product exists =="
if product_id="$(find_product_id 2>/dev/null)"; then
  echo "Product already exists: $PRODUCT_NAME ($product_id)"
else
  near_tx "$VALIDATOR_OWNER_ACCOUNT_ID" "$STAKING_ACCOUNT_ID" create_product "$(build_product_arg)"
  if [[ "$EXECUTE" == "1" ]]; then
    product_id="$(find_product_id)"
    echo "Created product: $PRODUCT_NAME ($product_id)"
  else
    product_id="<product_id returned by create_product>"
  fi
fi
echo

echo "== Ensure farm price exists =="
if [[ "$product_id" == "<product_id returned by create_product>" ]]; then
  near_tx "$VALIDATOR_OWNER_ACCOUNT_ID" "$STAKING_ACCOUNT_ID" create_price "$(build_price_arg "$product_id")"
else
  if active_farm_price_json="$(find_active_farm_price_json "$product_id" 2>/dev/null)"; then
    active_price_id="$(jq -r '.price_id' <<<"$active_farm_price_json")"
    if farm_price_matches "$active_farm_price_json"; then
      echo "Matching active farm price already exists: $active_price_id"
    else
      echo "Active farm price already exists with different fields: $active_price_id" >&2
      echo "$active_farm_price_json" | jq . >&2
      echo "Archive or edit the existing price before creating a replacement." >&2
      exit 1
    fi
  else
    near_tx "$VALIDATOR_OWNER_ACCOUNT_ID" "$STAKING_ACCOUNT_ID" create_price "$(build_price_arg "$product_id")"
  fi
fi

if [[ "$EXECUTE" == "1" && "$product_id" != "<product_id returned by create_product>" ]]; then
  echo
  echo "== Final product =="
  view_json "$STAKING_ACCOUNT_ID" get_product "$(jq -n --arg p "$product_id" '{product_id:$p}')"
fi

echo
echo "Done."
