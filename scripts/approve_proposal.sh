#!/usr/bin/env bash
set -e

pushd $(dirname $0)/..

PROPOSAL_ID=$1
MAJORITY_TYPE=${2:-Simple}

if [ -z "$PROPOSAL_ID" ]; then
  echo "Usage: $0 proposal_id [Simple|Strong]"
  exit 1
fi

if [ -z "$ROOT_ACCOUNT_ID" ]; then
  echo "Please set the ROOT_ACCOUNT_ID in the environment."
  exit 1
fi

: "${CHAIN_ID:=testnet}"
export VOTING_ACCOUNT_ID="vote.$ROOT_ACCOUNT_ID"
export REVIEWER_ACCOUNT_ID="reviewer.$ROOT_ACCOUNT_ID"

echo "Approving proposal $PROPOSAL_ID with $MAJORITY_TYPE majority"
export PROPOSAL=$(near --quiet contract call-function as-transaction $VOTING_ACCOUNT_ID approve_proposal json-args '{"proposal_id": '$PROPOSAL_ID', "majority_type": "'$MAJORITY_TYPE'"}' prepaid-gas '100.0 Tgas' attached-deposit '1 yoctoNEAR' sign-as $REVIEWER_ACCOUNT_ID network-config $CHAIN_ID sign-with-keychain send)

echo $PROPOSAL | jq .
