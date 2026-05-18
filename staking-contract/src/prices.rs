//! Catalog **prices**: create/edit/archive/unarchive/delete and `get_price`. Cross-contract routing uses
//! [`ExtSelfPrices`] / [`ext_self_prices`] after `get_owner_id` on the pool.

use crate::epoch::ext_staking_pool;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U128;
use near_sdk::{AccountId, Promise, env, near, require};

fn next_unique_price_id(contract: &mut Contract) -> PriceId {
    for _ in 0..64 {
        let id = crate::ids::next_price_id(&mut contract.id_nonce);
        if !contract.prices.contains_key(&id) {
            return id;
        }
    }
    env::panic_str("Could not allocate a unique price id; try again")
}

/// Self callbacks for **price** catalog after `get_owner_id` on the staking pool.
#[ext_contract(ext_self_prices)]
pub trait ExtSelfPrices {
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
    fn unarchive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    );
}

#[near]
impl Contract {
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
        require!(product.is_some(), "Product not found in the catalog");
        let product = product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_prices::ext(env::current_account_id())
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
        let maybe_price = self.prices.get(&price_id).cloned();
        require!(maybe_price.is_some(), "Price not found in the catalog");
        let price = maybe_price.unwrap();
        let maybe_product = self.products.get(&price.product_id).cloned();
        require!(maybe_product.is_some(), "Product not found in the catalog");
        let product = maybe_product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_prices::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .edit_price_after_get_owner(price_id, name, description, expected_caller),
            )
    }

    #[payable]
    pub fn archive_price(&mut self, price_id: PriceId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let maybe_price = self.prices.get(&price_id).cloned();
        require!(maybe_price.is_some(), "Price not found in the catalog");
        let price = maybe_price.unwrap();
        let maybe_product = self.products.get(&price.product_id).cloned();
        require!(maybe_product.is_some(), "Product not found in the catalog");
        let product = maybe_product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_prices::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .archive_price_after_get_owner(price_id, expected_caller),
            )
    }

    #[payable]
    pub fn unarchive_price(&mut self, price_id: PriceId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let maybe_price = self.prices.get(&price_id).cloned();
        require!(maybe_price.is_some(), "Price not found in the catalog");
        let price = maybe_price.unwrap();
        let maybe_product = self.products.get(&price.product_id).cloned();
        require!(maybe_product.is_some(), "Product not found in the catalog");
        let product = maybe_product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_prices::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .unarchive_price_after_get_owner(price_id, expected_caller),
            )
    }

    #[payable]
    pub fn delete_price(&mut self, price_id: PriceId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let maybe_price = self.prices.get(&price_id).cloned();
        require!(maybe_price.is_some(), "Price not found in the catalog");
        let price = maybe_price.unwrap();
        let maybe_product = self.products.get(&price.product_id).cloned();
        require!(maybe_product.is_some(), "Product not found in the catalog");
        let product = maybe_product.unwrap();
        self.assert_validator_allowlisted(&product.validator_id);
        let expected_caller = env::predecessor_account_id();
        let validator_id = product.validator_id.clone();
        ext_staking_pool::ext(validator_id)
            .with_static_gas(staking_pool::GET_OWNER_ID)
            .get_owner_id()
            .then(
                ext_self_prices::ext(env::current_account_id())
                    .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                    .delete_price_after_get_owner(price_id, expected_caller),
            )
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
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut product = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
        require!(
            product.status == CatalogStatus::Active,
            "This product is archived or inactive"
        );

        let price_id = next_unique_price_id(self);
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
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut price = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Price not found in the catalog");
        price.name = name;
        price.description = description;
        self.prices.insert(price_id, price);
    }

    #[private]
    pub fn archive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut price = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Price not found in the catalog");
        let product_id = price.product_id.clone();
        price.status = CatalogStatus::Archived;
        self.prices.insert(price_id.clone(), price);
        self.clear_product_default_price_field_if_matches(&product_id, &price_id);
    }

    #[private]
    pub fn delete_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let price = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Price not found in the catalog");
        require!(
            price.usage_count == 0,
            "Cannot delete this price while it is in use"
        );
        let product_id = price.product_id.clone();
        let mut product = self
            .products
            .get(&price.product_id)
            .cloned()
            .expect("Product not found in the catalog");
        product.price_ids.retain(|x| x != &price_id);
        self.products.insert(price.product_id.clone(), product);
        self.prices.remove(&price_id);
        self.clear_product_default_price_field_if_matches(&product_id, &price_id);
    }

    #[private]
    pub fn unarchive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut price = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Price not found in the catalog");
        require!(
            price.status == CatalogStatus::Archived,
            "Price is not archived"
        );
        price.status = CatalogStatus::Active;
        self.prices.insert(price_id, price);
    }

    pub fn get_price(&self, price_id: PriceId) -> Option<Price> {
        self.prices.get(&price_id).cloned()
    }
}
