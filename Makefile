# House of Stake per-contract NEAR WASM builds.

.DEFAULT_GOAL := help

.PHONY: help all-contracts \
	sandbox-staking-whitelist-contract venear-contract lockup-contract voting-contract \
	whitelist venear lockup voting \
	check-sandbox-staking-whitelist-contract check-venear-contract check-lockup-contract \
	check-voting-contract check-whitelist check-venear check-lockup check-voting \
	test test-integration

ROOT := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
RES_LOCAL := $(ROOT)res/local
INTEGRATION_TEST_ARGS := $(shell find "$(ROOT)integration-tests/tests" -maxdepth 1 -name 'test_*.rs' ! -name 'test_lockup.rs' -printf '--test %f\n' | sed 's/\.rs$$//' | sort)

help:
	@echo "WASM builds (cargo near build non-reproducible-wasm; copies .wasm to res/local/):"
	@echo "  make sandbox-staking-whitelist-contract   (alias: make whitelist)"
	@echo "  make venear-contract                      (alias: make venear)"
	@echo "  make lockup-contract                      (alias: make lockup)"
	@echo "  make voting-contract                      (alias: make voting; sandbox feature for integration tests)"
	@echo "  make all-contracts                        all of the above, in order"
	@echo ""
	@echo "Fast compile checks:"
	@echo "  make check-<name>   e.g. make check-voting-contract, make check-whitelist"
	@echo ""
	@echo "Tests:"
	@echo "  make test                                 run workspace tests and integration tests except test_lockup"
	@echo "  make test-integration                     run integration tests except test_lockup"

sandbox-staking-whitelist-contract:
	cd "$(ROOT)sandbox-staking-whitelist-contract" && cargo near build non-reproducible-wasm
	mkdir -p "$(RES_LOCAL)"
	cp "$(ROOT)target/near/sandbox_staking_whitelist_contract/sandbox_staking_whitelist_contract.wasm" "$(RES_LOCAL)/"

venear-contract:
	cd "$(ROOT)venear-contract" && cargo near build non-reproducible-wasm
	mkdir -p "$(RES_LOCAL)"
	cp "$(ROOT)target/near/venear_contract/venear_contract.wasm" "$(RES_LOCAL)/"

lockup-contract:
	cd "$(ROOT)lockup-contract" && cargo near build non-reproducible-wasm
	mkdir -p "$(RES_LOCAL)"
	cp "$(ROOT)target/near/lockup_contract/lockup_contract.wasm" "$(RES_LOCAL)/"

voting-contract:
	cd "$(ROOT)voting-contract" && cargo near build non-reproducible-wasm --features sandbox
	mkdir -p "$(RES_LOCAL)"
	cp "$(ROOT)target/near/voting_contract/voting_contract.wasm" "$(RES_LOCAL)/"

all-contracts: sandbox-staking-whitelist-contract venear-contract lockup-contract voting-contract

whitelist: sandbox-staking-whitelist-contract
venear: venear-contract
lockup: lockup-contract
voting: voting-contract

check-sandbox-staking-whitelist-contract check-whitelist:
	cd "$(ROOT)" && cargo check -p sandbox-staking-whitelist-contract

check-venear-contract check-venear:
	cd "$(ROOT)" && cargo check -p venear-contract

check-lockup-contract check-lockup:
	cd "$(ROOT)" && cargo check -p lockup-contract

check-voting-contract check-voting:
	cd "$(ROOT)" && cargo check -p voting-contract

test:
	$(MAKE) all-contracts
	cd "$(ROOT)" && cargo test --workspace --exclude integration-tests
	$(MAKE) test-integration

test-integration:
	cd "$(ROOT)" && cargo test -p integration-tests $(INTEGRATION_TEST_ARGS)
