use crate::*;
use near_sdk::env;
use near_sdk::store::{LookupMap, Vector};

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
        let old: ContractV1_0_1 = env::state_read().unwrap();
        old.into()
    }

    pub fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[allow(non_camel_case_types)]
#[near(serializers = [borsh])]
struct ContractV1_0_1 {
    pub config: VConfig,
    pub paused: bool,
    pub validators: LookupMap<ValidatorId, VValidator>,
    pub validator_ids: Vector<ValidatorId>,
    pub product_ids: Vector<ProductId>,
    pub products: LookupMap<ProductId, VProduct>,
    pub prices: LookupMap<PriceId, VPrice>,
    pub accounts: LookupMap<AccountId, VAccount>,
    pub subscriptions: LookupMap<SubscriptionId, VSubscription>,
    pub locks: LookupMap<LockId, VLock>,
    pub user_validator_shares: LookupMap<(AccountId, ValidatorId), u128>,
    pub user_pending_unstake: LookupMap<(AccountId, ValidatorId), Vec<PendingUnstakeTranche>>,
    pub user_lock_count: LookupMap<AccountId, u32>,
    pub purchases: LookupMap<PurchaseId, VPurchase>,
    pub purchase_ids: Vector<PurchaseId>,
    pub purchases_by_account: LookupMap<AccountId, Vec<PurchaseId>>,
    pub purchases_by_product: LookupMap<ProductId, Vec<PurchaseId>>,
    pub user_purchase_count: LookupMap<AccountId, u32>,
    pub revenue_by_validator: LookupMap<ValidatorId, NearToken>,
    pub subscription_by_account_product: LookupMap<(AccountId, ProductId), SubscriptionId>,
    pub subscriptions_by_account: LookupMap<AccountId, Vec<SubscriptionId>>,
    pub subscription_ids: Vector<SubscriptionId>,
    pub pending_update_target_price_counts: LookupMap<PriceId, u32>,
    pub pending_update_target_product_counts: LookupMap<ProductId, u32>,
    pub id_nonce: u64,
}

impl From<ContractV1_0_1> for Contract {
    fn from(old: ContractV1_0_1) -> Self {
        Self {
            config: Config::from(old.config).into(),
            paused: old.paused,
            validators: old.validators,
            validator_ids: old.validator_ids,
            product_ids: old.product_ids,
            products: old.products,
            prices: old.prices,
            accounts: old.accounts,
            subscriptions: old.subscriptions,
            locks: old.locks,
            user_validator_shares: old.user_validator_shares,
            user_pending_unstake: old.user_pending_unstake,
            user_lock_count: old.user_lock_count,
            user_farm_position_count: LookupMap::new(StorageKeys::UserFarmPositionCount),
            purchases: old.purchases,
            purchase_ids: old.purchase_ids,
            purchases_by_account: old.purchases_by_account,
            purchases_by_product: old.purchases_by_product,
            user_purchase_count: old.user_purchase_count,
            revenue_by_validator: old.revenue_by_validator,
            farm_pools: LookupMap::new(StorageKeys::FarmPools),
            farm_positions: LookupMap::new(StorageKeys::FarmPositions),
            farm_position_products_by_account: LookupMap::new(
                StorageKeys::FarmPositionProductsByAccount,
            ),
            farm_accounts: LookupMap::new(StorageKeys::FarmAccounts),
            subscription_by_account_product: old.subscription_by_account_product,
            subscriptions_by_account: old.subscriptions_by_account,
            subscription_ids: old.subscription_ids,
            pending_update_target_price_counts: old.pending_update_target_price_counts,
            pending_update_target_product_counts: old.pending_update_target_product_counts,
            id_nonce: old.id_nonce,
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
