use crate::*;
use near_sdk::borsh::BorshDeserialize;
use near_sdk::env;
use near_sdk::store::{IterableMap, IterableSet, LookupMap, Vector};
use near_sdk::{AccountId, NearToken, near};

#[cfg(target_arch = "wasm32")]
use near_sdk::{Gas, sys};

#[cfg(target_arch = "wasm32")]
const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
#[cfg(target_arch = "wasm32")]
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

#[near]
impl Contract {
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let raw = env::storage_read(b"STATE").expect("Contract state is missing");
        let mut bytes = raw.as_slice();
        let old = OldContract::deserialize(&mut bytes)
            .unwrap_or_else(|_| env::panic_str("Cannot deserialize the previous contract state"));
        let mut contract = old.into_current();
        contract.rebuild_farm_position_indexes();
        contract
    }

    pub fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[near(serializers = [borsh])]
struct OldContract {
    pub config: VConfig,
    pub paused: bool,
    pub validators: LookupMap<ValidatorId, VValidator>,
    pub validator_ids: Vector<ValidatorId>,
    pub product_ids: Vector<ProductId>,
    pub products: LookupMap<ProductId, VProduct>,
    pub prices: LookupMap<PriceId, VPrice>,
    pub accounts: LookupMap<AccountId, VAccount>,
    pub account_ids: IterableSet<AccountId>,
    pub subscriptions: LookupMap<SubscriptionId, VSubscription>,
    pub locks: LookupMap<LockId, VLock>,
    pub lock_ids: IterableSet<LockId>,
    pub locks_by_account: LookupMap<AccountId, Vector<LockId>>,
    pub user_validator_shares: LookupMap<(AccountId, ValidatorId), u128>,
    pub user_pending_unstake: LookupMap<(AccountId, ValidatorId), Vec<PendingUnstakeTranche>>,
    pub user_pending_unstake_validator_count: LookupMap<AccountId, u32>,
    pub user_lock_count: LookupMap<AccountId, u32>,
    pub purchases: LookupMap<PurchaseId, VPurchase>,
    pub purchase_ids: Vector<PurchaseId>,
    pub purchases_by_account: LookupMap<AccountId, Vector<PurchaseId>>,
    pub purchases_by_product: LookupMap<ProductId, Vector<PurchaseId>>,
    pub user_purchase_count: LookupMap<AccountId, u32>,
    pub revenue_by_validator: LookupMap<ValidatorId, NearToken>,
    pub farm_pools: LookupMap<PriceId, VFarmPool>,
    pub farm_positions: LookupMap<(AccountId, ProductId), VFarmPosition>,
    pub farm_position_products_by_account: LookupMap<AccountId, Vec<ProductId>>,
    pub user_farm_position_count: LookupMap<AccountId, u32>,
    pub farm_accounts: LookupMap<AccountId, VFarmAccount>,
    pub subscription_by_account_product: LookupMap<(AccountId, ProductId), SubscriptionId>,
    pub subscriptions_by_account: LookupMap<AccountId, Vec<SubscriptionId>>,
    pub subscriptions_by_product: LookupMap<ProductId, IterableSet<SubscriptionId>>,
    pub subscription_ids: IterableMap<SubscriptionId, ()>,
    pub pending_update_target_price_counts: LookupMap<PriceId, u32>,
    pub pending_update_target_product_counts: LookupMap<ProductId, u32>,
    pub id_nonce: u64,
}

impl OldContract {
    fn into_current(self) -> Contract {
        Contract {
            config: self.config,
            paused: self.paused,
            validators: self.validators,
            validator_ids: self.validator_ids,
            product_ids: self.product_ids,
            products: self.products,
            prices: self.prices,
            accounts: self.accounts,
            account_ids: self.account_ids,
            subscriptions: self.subscriptions,
            locks: self.locks,
            lock_ids: self.lock_ids,
            locks_by_account: self.locks_by_account,
            user_validator_shares: self.user_validator_shares,
            user_pending_unstake: self.user_pending_unstake,
            user_pending_unstake_validator_count: self.user_pending_unstake_validator_count,
            user_lock_count: self.user_lock_count,
            purchases: self.purchases,
            purchase_ids: self.purchase_ids,
            purchases_by_account: self.purchases_by_account,
            purchases_by_product: self.purchases_by_product,
            user_purchase_count: self.user_purchase_count,
            revenue_by_validator: self.revenue_by_validator,
            farm_pools: self.farm_pools,
            farm_positions: self.farm_positions,
            farm_position_keys: Vector::new(StorageKeys::FarmPositionKeys),
            farm_position_products_by_account: self.farm_position_products_by_account,
            farm_position_accounts_by_product: LookupMap::new(
                StorageKeys::FarmPositionAccountsByProduct,
            ),
            user_farm_position_count: self.user_farm_position_count,
            farm_accounts: self.farm_accounts,
            subscription_by_account_product: self.subscription_by_account_product,
            subscriptions_by_account: self.subscriptions_by_account,
            subscriptions_by_product: self.subscriptions_by_product,
            subscription_ids: self.subscription_ids,
            pending_update_target_price_counts: self.pending_update_target_price_counts,
            pending_update_target_product_counts: self.pending_update_target_product_counts,
            id_nonce: self.id_nonce,
        }
    }

    #[cfg(test)]
    fn from_current_for_test(contract: Contract) -> Self {
        Self {
            config: contract.config,
            paused: contract.paused,
            validators: contract.validators,
            validator_ids: contract.validator_ids,
            product_ids: contract.product_ids,
            products: contract.products,
            prices: contract.prices,
            accounts: contract.accounts,
            account_ids: contract.account_ids,
            subscriptions: contract.subscriptions,
            locks: contract.locks,
            lock_ids: contract.lock_ids,
            locks_by_account: contract.locks_by_account,
            user_validator_shares: contract.user_validator_shares,
            user_pending_unstake: contract.user_pending_unstake,
            user_pending_unstake_validator_count: contract.user_pending_unstake_validator_count,
            user_lock_count: contract.user_lock_count,
            purchases: contract.purchases,
            purchase_ids: contract.purchase_ids,
            purchases_by_account: contract.purchases_by_account,
            purchases_by_product: contract.purchases_by_product,
            user_purchase_count: contract.user_purchase_count,
            revenue_by_validator: contract.revenue_by_validator,
            farm_pools: contract.farm_pools,
            farm_positions: contract.farm_positions,
            farm_position_products_by_account: contract.farm_position_products_by_account,
            user_farm_position_count: contract.user_farm_position_count,
            farm_accounts: contract.farm_accounts,
            subscription_by_account_product: contract.subscription_by_account_product,
            subscriptions_by_account: contract.subscriptions_by_account,
            subscriptions_by_product: contract.subscriptions_by_product,
            subscription_ids: contract.subscription_ids,
            pending_update_target_price_counts: contract.pending_update_target_price_counts,
            pending_update_target_product_counts: contract.pending_update_target_product_counts,
            id_nonce: contract.id_nonce,
        }
    }
}

impl Contract {
    fn rebuild_farm_position_indexes(&mut self) {
        self.farm_position_keys = Vector::new(StorageKeys::FarmPositionKeys);
        self.farm_position_accounts_by_product =
            LookupMap::new(StorageKeys::FarmPositionAccountsByProduct);

        let account_ids: Vec<AccountId> = self.account_ids.iter().cloned().collect();
        for account_id in account_ids {
            let product_ids = self
                .farm_position_products_by_account
                .get(&account_id)
                .cloned()
                .unwrap_or_default();
            for product_id in product_ids {
                if self
                    .internal_get_farm_position(&account_id, &product_id)
                    .is_some()
                {
                    self.add_farm_position_to_global_index(&account_id, &product_id);
                    self.add_farm_position_account_to_product(&account_id, &product_id);
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn upgrade() {
    env::setup_panic_hook();
    let contract: Contract = env::state_read().unwrap();
    contract.assert_owner();
    let current_account_id = env::current_account_id();
    let current_account_id = current_account_id.as_str();
    let migrate_method_name = b"migrate_state".to_vec();
    let get_config_method_name = b"get_config".to_vec();
    let empty_args = b"{}".to_vec();
    unsafe {
        sys::input(0);
        let promise_id = sys::promise_batch_create(
            current_account_id.len() as _,
            current_account_id.as_ptr() as _,
        );
        sys::promise_batch_action_deploy_contract(promise_id, u64::MAX as _, 0);

        sys::promise_batch_action_function_call_weight(
            promise_id,
            migrate_method_name.len() as _,
            migrate_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            MIGRATE_STATE_GAS.as_gas(),
            1,
        );
        sys::promise_batch_action_function_call(
            promise_id,
            get_config_method_name.len() as _,
            get_config_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            GET_CONFIG_GAS.as_gas(),
        );
        sys::promise_return(promise_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::json_types::U64;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::{AccountId, NearToken, testing_env};

    fn acct(s: &str) -> AccountId {
        s.parse().expect("valid account id")
    }

    fn test_context() {
        let account_id = acct("staking.near");
        testing_env!(
            VMContextBuilder::new()
                .current_account_id(account_id.clone())
                .predecessor_account_id(account_id.clone())
                .signer_account_id(account_id)
                .build()
        );
    }

    fn test_config() -> Config {
        Config {
            owner_account_id: acct("owner.near"),
            proposed_new_owner_account_id: None,
            guardians: vec![],
            min_lock_duration_ns: U64(1),
            max_lock_duration_ns: U64(u64::MAX / 8),
            epoch_unstake_settle_epochs: 4,
            min_storage_deposit: NearToken::from_millinear(100),
            per_lock_storage_stake: NearToken::from_near(0),
            per_farm_position_storage_stake: NearToken::from_near(0),
            per_purchase_storage_stake: NearToken::from_near(0),
            min_lock_amount: NearToken::from_near(1),
        }
    }

    #[test]
    fn migrate_state_returns_fresh_contract_state() {
        test_context();
        let mut contract = Contract::new(test_config());
        contract.paused = true;
        let old_contract = OldContract::from_current_for_test(contract);

        env::state_write(&old_contract);

        let migrated = Contract::migrate_state();

        assert!(migrated.paused);
        assert_eq!(
            migrated.config.as_ref().owner_account_id,
            acct("owner.near")
        );
    }
}
