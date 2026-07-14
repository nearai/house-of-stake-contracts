#!/usr/bin/env bash
set -euo pipefail

# Switch chat-api House-of-Stake agent-hosting config between the current
# staging catalog prices/credit ratio and production catalog prices/credit ratio.
#
# This only updates chat-api system config:
#   subscription_plans.{starter,basic,pro}.providers["house-of-stake"].price_id
#   subscription_plans.{basic,pro}.stake_based_monthly_credits.credits_per_staked_near_nano_usd
#
# It preserves Stripe providers, one-off credits config, agent limits, and other
# plan fields already present in chat-api.
#
# Preview staging rollback:
#   ADMIN_SESSION_TOKEN=sess_... TARGET_ENV=staging DRY_RUN=1 \
#     ./scripts/switch_chat_api_agent_hosting_prices.sh
#
# Apply staging rollback:
#   ADMIN_SESSION_TOKEN=sess_... TARGET_ENV=staging \
#     ./scripts/switch_chat_api_agent_hosting_prices.sh
#
# Preview production switch:
#   ADMIN_SESSION_TOKEN=sess_... TARGET_ENV=production DRY_RUN=1 \
#     ./scripts/switch_chat_api_agent_hosting_prices.sh
#
# Apply production switch:
#   ADMIN_SESSION_TOKEN=sess_... TARGET_ENV=production \
#     ./scripts/switch_chat_api_agent_hosting_prices.sh
#
# Optional:
#   CHAT_API_URL=http://localhost:8080
#   STAKING_ACCOUNT_ID=hos-e2e-0601144939.testnet
#   CHAIN_ID=testnet
#   VERIFY_STAKING=1
#   VERIFY_PRICE_NAMES=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

: "${TARGET_ENV:=production}"
: "${CHAT_API_URL:=http://localhost:8080}"
: "${ADMIN_SESSION_TOKEN:=}"
: "${DRY_RUN:=0}"
: "${VERIFY_STAKING:=1}"
: "${VERIFY_PRICE_NAMES:=0}"
: "${CHAIN_ID:=testnet}"
: "${STAKING_ACCOUNT_ID:=hos-e2e-0601144939.testnet}"

# Current staging price IDs on the testnet staking contract.
# Starter: 1 NEAR, Basic: 10 NEAR, Pro: 40 NEAR.
: "${STAGING_HOS_AGENT_STARTER_PRICE_ID:=price_RjiajH4KEZ43w68DgY5xVaVU}"
: "${STAGING_HOS_AGENT_BASIC_PRICE_ID:=price_h577VYQUEynPA3uQt1u1neGn}"
: "${STAGING_HOS_AGENT_PRO_PRICE_ID:=price_7EAls0E844ULR06EEl53fQoI}"
: "${STAGING_CREDITS_PER_STAKED_NEAR_NANO_USD:=500000000}"

# Production price IDs on the testnet staking contract.
# Starter: 50 NEAR, Basic: 500 NEAR, Pro: 2000 NEAR.
# Production credits ratio assumes 1 credit = $1 and maps 2000 staked NEAR to
# $20 monthly credits:
# $20 / 2000 NEAR * 1e9 nano USD = 10,000,000.
: "${PRODUCTION_HOS_AGENT_STARTER_PRICE_ID:=price_dIMZ1c88xUwvvXlP521TgZ8W}"
: "${PRODUCTION_HOS_AGENT_BASIC_PRICE_ID:=price_saAr3F0Dj2jGJzUaJFhPH6Mh}"
: "${PRODUCTION_HOS_AGENT_PRO_PRICE_ID:=price_u6GUB9EgjZEk0nbQ3fomC2k3}"
: "${PRODUCTION_CREDITS_PER_STAKED_NEAR_NANO_USD:=10000000}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 not found in PATH." >&2
    exit 1
  fi
}

require_command jq
require_command python3
if [[ "$VERIFY_STAKING" == "1" ]]; then
  require_command near
fi

if [[ -z "$ADMIN_SESSION_TOKEN" ]]; then
  echo "ADMIN_SESSION_TOKEN is required." >&2
  exit 1
fi

case "$TARGET_ENV" in
  staging)
    HOS_AGENT_STARTER_PRICE_ID="$STAGING_HOS_AGENT_STARTER_PRICE_ID"
    HOS_AGENT_BASIC_PRICE_ID="$STAGING_HOS_AGENT_BASIC_PRICE_ID"
    HOS_AGENT_PRO_PRICE_ID="$STAGING_HOS_AGENT_PRO_PRICE_ID"
    CREDITS_PER_STAKED_NEAR_NANO_USD="$STAGING_CREDITS_PER_STAKED_NEAR_NANO_USD"
    ;;
  production)
    HOS_AGENT_STARTER_PRICE_ID="$PRODUCTION_HOS_AGENT_STARTER_PRICE_ID"
    HOS_AGENT_BASIC_PRICE_ID="$PRODUCTION_HOS_AGENT_BASIC_PRICE_ID"
    HOS_AGENT_PRO_PRICE_ID="$PRODUCTION_HOS_AGENT_PRO_PRICE_ID"
    CREDITS_PER_STAKED_NEAR_NANO_USD="$PRODUCTION_CREDITS_PER_STAKED_NEAR_NANO_USD"
    ;;
  *)
    echo "TARGET_ENV must be production or staging; got: $TARGET_ENV" >&2
    exit 1
    ;;
esac

verify_price() {
  local plan_name="$1"
  local price_id="$2"
  local price_json

  echo "Verifying $plan_name price on $STAKING_ACCOUNT_ID: $price_id"
  price_json="$(
    near contract call-function as-read-only "$STAKING_ACCOUNT_ID" get_price \
      json-args "$(jq -n --arg price_id "$price_id" '{price_id:$price_id}')" \
      network-config "$CHAIN_ID" now
  )"

  jq -e \
    --arg price_id "$price_id" \
    '.price_id == $price_id
      and .status == "Active"
      and .price_type == "Recurring"
      and .billing_period == "Monthly"' \
    >/dev/null <<<"$price_json" || {
      echo "Price verification failed for $plan_name ($price_id)." >&2
      echo "$price_json" | jq . >&2
      exit 1
    }

  if [[ "$VERIFY_PRICE_NAMES" == "1" ]]; then
    jq -e \
      --arg plan_name "$plan_name" \
      '(.name | ascii_downcase) == $plan_name' \
      >/dev/null <<<"$price_json" || {
        echo "Price name verification failed for $plan_name ($price_id)." >&2
        echo "$price_json" | jq . >&2
        exit 1
      }
  fi
}

if [[ "$VERIFY_STAKING" == "1" ]]; then
  verify_price starter "$HOS_AGENT_STARTER_PRICE_ID"
  verify_price basic "$HOS_AGENT_BASIC_PRICE_ID"
  verify_price pro "$HOS_AGENT_PRO_PRICE_ID"
  echo
fi

export TARGET_ENV
export CHAT_API_URL
export ADMIN_SESSION_TOKEN
export DRY_RUN
export HOS_AGENT_STARTER_PRICE_ID
export HOS_AGENT_BASIC_PRICE_ID
export HOS_AGENT_PRO_PRICE_ID
export CREDITS_PER_STAKED_NEAR_NANO_USD

python3 - <<'PY'
import json
import os
import sys
import urllib.error
import urllib.request

chat_api_url = os.environ["CHAT_API_URL"].rstrip("/")
admin_session_token = os.environ["ADMIN_SESSION_TOKEN"]
dry_run = os.environ.get("DRY_RUN") == "1"
target_env = os.environ["TARGET_ENV"]

target_price_ids = {
    "starter": os.environ["HOS_AGENT_STARTER_PRICE_ID"],
    "basic": os.environ["HOS_AGENT_BASIC_PRICE_ID"],
    "pro": os.environ["HOS_AGENT_PRO_PRICE_ID"],
}
credits_per_staked_near_nano_usd = int(
    os.environ["CREDITS_PER_STAKED_NEAR_NANO_USD"]
)


def request_json(method, path, body=None):
    data = None
    headers = {
        "Authorization": f"Bearer {admin_session_token}",
        "Accept": "application/json",
    }
    if body is not None:
        data = json.dumps(body, sort_keys=True).encode("utf-8")
        headers["Content-Type"] = "application/json"

    request = urllib.request.Request(
        f"{chat_api_url}{path}",
        data=data,
        headers=headers,
        method=method,
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode("utf-8")
            return json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        details = error.read().decode("utf-8", errors="replace")
        raise SystemExit(
            f"{method} {path} failed with HTTP {error.code}: {details}"
        ) from error


current = request_json("GET", "/v1/admin/configs") or {}
subscription_plans = dict(current.get("subscription_plans") or {})
changes = {}

for plan_name, price_id in target_price_ids.items():
    plan = dict(subscription_plans.get(plan_name) or {})
    providers = dict(plan.get("providers") or {})
    old_hos = dict(providers.get("house-of-stake") or {})
    old_price_id = old_hos.get("price_id")
    providers["house-of-stake"] = {**old_hos, "price_id": price_id}
    plan["providers"] = providers

    old_credits_ratio = None
    if plan_name in {"basic", "pro"}:
        stake_credits = dict(plan.get("stake_based_monthly_credits") or {})
        old_credits_ratio = stake_credits.get("credits_per_staked_near_nano_usd")
        stake_credits["credits_per_staked_near_nano_usd"] = (
            credits_per_staked_near_nano_usd
        )
        plan["stake_based_monthly_credits"] = stake_credits

    subscription_plans[plan_name] = plan
    changes[plan_name] = {
        "price_id": {"from": old_price_id, "to": price_id},
    }
    if old_credits_ratio is not None:
        changes[plan_name]["credits_per_staked_near_nano_usd"] = {
            "from": old_credits_ratio,
            "to": credits_per_staked_near_nano_usd,
        }

payload = {
    "subscription_plans": subscription_plans,
}

print(f"Switching chat-api House-of-Stake agent prices to {target_env}:")
print(json.dumps(changes, indent=2, sort_keys=True))
print("PATCH payload:")
print(json.dumps(payload, indent=2, sort_keys=True))

if dry_run:
    print("DRY_RUN=1; not PATCHing /v1/admin/configs")
    raise SystemExit(0)

updated = request_json("PATCH", "/v1/admin/configs", payload)
print("Updated /v1/admin/configs")
print(json.dumps(updated, indent=2, sort_keys=True))
PY
