use crate::epoch::ext_staking_pool;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, Promise, env, is_promise_success, near, require};

/// Self callbacks after `get_owner_id` on the staking pool.
#[ext_contract(ext_self_products)]
pub trait ExtSelfProducts {
    fn create_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        validator_id: AccountId,
        name: String,
        description: String,
        expected_caller: AccountId,
    ) -> ProductId;
    fn edit_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        name: String,
        description: String,
        expected_caller: AccountId,
    );
    fn archive_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    );
    fn delete_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    );
    fn create_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        name: String,
        description: String,
        amount: U128,
        price_type: PriceType,
        billing_period: Option<BillingPeriod>,
        lock_factor_near_months: U128,
        expected_caller: AccountId,
    ) -> PriceId;
    fn edit_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        name: String,
        description: String,
        expected_caller: AccountId,
    );
    fn archive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    );
    fn delete_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    );
}

#[near]
impl Contract {
    #[payable]
    pub fn create_product(
        &mut self,
        validator_id: AccountId,
        name: String,
        description: String,
    ) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        self.assert_validator_allowlisted(&validator_id);
        let expected_caller = env::predecessor_account_id();
        ext_staking_pool::ext(validator_id.clone())
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .create_product_after_get_owner(
                        validator_id,
                        name,
                        description,
                        expected_caller,
                    ),
            )
    }

    #[payable]
    pub fn edit_product(
        &mut self,
        product_id: ProductId,
        name: String,
        description: String,
    ) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self.products.get(&product_id).cloned();
        require!(p.is_some(), "Unknown product");
        let p = p.unwrap();
        self.assert_validator_allowlisted(&p.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = p.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .edit_product_after_get_owner(product_id, name, description, expected_caller),
            )
    }

    #[payable]
    pub fn archive_product(&mut self, product_id: ProductId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self.products.get(&product_id).cloned();
        require!(p.is_some(), "Unknown product");
        let p = p.unwrap();
        self.assert_validator_allowlisted(&p.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = p.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .archive_product_after_get_owner(product_id, expected_caller),
            )
    }

    #[payable]
    pub fn delete_product(&mut self, product_id: ProductId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self.products.get(&product_id).cloned();
        require!(p.is_some(), "Unknown product");
        let p = p.unwrap();
        self.assert_validator_allowlisted(&p.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = p.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .delete_product_after_get_owner(product_id, expected_caller),
            )
    }

    #[payable]
    pub fn create_price(
        &mut self,
        product_id: ProductId,
        name: String,
        description: String,
        amount: U128,
        price_type: PriceType,
        billing_period: Option<BillingPeriod>,
        lock_factor_near_months: U128,
    ) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let product = self.products.get(&product_id).cloned();
        require!(product.is_some(), "Unknown product");
        let product = product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .create_price_after_get_owner(
                        product_id,
                        name,
                        description,
                        amount,
                        price_type,
                        billing_period,
                        lock_factor_near_months,
                        expected_caller,
                    ),
            )
    }

    #[payable]
    pub fn edit_price(&mut self, price_id: PriceId, name: String, description: String) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let pr = self.prices.get(&price_id).cloned();
        require!(pr.is_some(), "Unknown price");
        let pr = pr.unwrap();
        let product = self.products.get(&pr.product_id).cloned();
        require!(product.is_some(), "Unknown product");
        let product = product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .edit_price_after_get_owner(price_id, name, description, expected_caller),
            )
    }

    #[payable]
    pub fn archive_price(&mut self, price_id: PriceId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let pr = self.prices.get(&price_id).cloned();
        require!(pr.is_some(), "Unknown price");
        let pr = pr.unwrap();
        let product = self.products.get(&pr.product_id).cloned();
        require!(product.is_some(), "Unknown product");
        let product = product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .archive_price_after_get_owner(price_id, expected_caller),
            )
    }

    #[payable]
    pub fn delete_price(&mut self, price_id: PriceId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let pr = self.prices.get(&price_id).cloned();
        require!(pr.is_some(), "Unknown price");
        let pr = pr.unwrap();
        let product = self.products.get(&pr.product_id).cloned();
        require!(product.is_some(), "Unknown product");
        let product = product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_products::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .delete_price_after_get_owner(price_id, expected_caller),
            )
    }

    #[private]
    pub fn create_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        validator_id: AccountId,
        name: String,
        description: String,
        expected_caller: AccountId,
    ) -> ProductId {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );

        let id = crate::ids::next_product_id(&mut self.id_nonce);
        let product = Product {
            product_id: id.clone(),
            validator_id: validator_id.clone(),
            name,
            description,
            status: CatalogStatus::Active,
            created_ns: U64(env::block_timestamp()),
            price_ids: Vec::new(),
            usage_count: 0,
        };
        self.products.insert(id.clone(), product);
        self.product_ids.push(id.clone());
        crate::events::log_product_created(id.as_str(), &validator_id);
        id
    }

    #[private]
    pub fn edit_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        name: String,
        description: String,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        p.name = name;
        p.description = description;
        self.products.insert(product_id, p);
    }

    #[private]
    pub fn archive_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        p.status = CatalogStatus::Archived;
        self.products.insert(product_id, p);
    }

    #[private]
    pub fn delete_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        require!(p.usage_count == 0, "Product in use");
        require!(
            p.price_ids.is_empty(),
            "Remove or delete all prices for this product first"
        );
        self.remove_product_id_from_list(&product_id);
        self.products.remove(&product_id);
    }

    #[private]
    pub fn create_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        name: String,
        description: String,
        amount: U128,
        price_type: PriceType,
        billing_period: Option<BillingPeriod>,
        lock_factor_near_months: U128,
        expected_caller: AccountId,
    ) -> PriceId {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut product = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        require!(product.status == CatalogStatus::Active, "Product archived");

        let price_id = crate::ids::next_price_id(&mut self.id_nonce);
        let price = Price {
            price_id: price_id.clone(),
            product_id: product_id.clone(),
            name,
            description,
            amount,
            price_type,
            billing_period,
            lock_factor_near_months,
            status: CatalogStatus::Active,
            usage_count: 0,
        };
        self.prices.insert(price_id.clone(), price);
        product.price_ids.push(price_id.clone());
        self.products.insert(product_id, product);
        price_id
    }

    #[private]
    pub fn edit_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        name: String,
        description: String,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut pr = self.prices.get(&price_id).cloned().expect("Unknown price");
        pr.name = name;
        pr.description = description;
        self.prices.insert(price_id, pr);
    }

    #[private]
    pub fn archive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut pr = self.prices.get(&price_id).cloned().expect("Unknown price");
        pr.status = CatalogStatus::Archived;
        self.prices.insert(price_id, pr);
    }

    #[private]
    pub fn delete_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        require!(is_promise_success(), "get_owner_id failed");
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let pr = self.prices.get(&price_id).cloned().expect("Unknown price");
        require!(pr.usage_count == 0, "Price in use");
        let mut product = self
            .products
            .get(&pr.product_id)
            .cloned()
            .expect("Unknown product");
        product.price_ids.retain(|x| x != &price_id);
        self.products.insert(pr.product_id.clone(), product);
        self.prices.remove(&price_id);
    }

    pub fn get_product(&self, product_id: ProductId) -> Option<Product> {
        self.products.get(&product_id).cloned()
    }

    pub fn get_price(&self, price_id: PriceId) -> Option<Price> {
        self.prices.get(&price_id).cloned()
    }

    /// Paginated catalog index (creation order). Pair with [`Contract::get_product`].
    pub fn list_product_ids(&self, from_index: u64, limit: u64) -> Vec<ProductId> {
        let len_u64 = self.product_ids.len() as u64;
        let mut out = Vec::new();
        let mut i = from_index;
        while i < len_u64 && (out.len() as u64) < limit {
            if let Some(id) = self.product_ids.get(i as u32) {
                out.push(id.clone());
            }
            i += 1;
        }
        out
    }
}

impl Contract {
    fn remove_product_id_from_list(&mut self, product_id: &ProductId) {
        let len = self.product_ids.len();
        for i in 0..len {
            if self.product_ids.get(i).is_some_and(|s| s == product_id) {
                self.product_ids.swap_remove(i);
                return;
            }
        }
    }
}
