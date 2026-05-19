//! Catalog **prices**: create/edit/archive/unarchive/delete and `get_price`.
//!
//! **Auth:** Every mutating entrypoint is **validator-pool-owner** gated. The public method resolves the
//! price's product → pool, calls `get_owner_id` on that pool, then continues in [`ExtSelfPrices`] callbacks
//! via [`Contract::assert_pool_owner_callback`]. Contract owner/guardians cannot edit catalog entries directly.
//!
//! **Lifecycle:** Archive hides a tier from new locks; delete requires `usage_count == 0`. Archiving or
//! deleting clears [`Product::default_price_id`] when this price was the product default
//! ([`Contract::clear_product_default_price_field_if_matches`] in [`crate::products`]).

use crate::gas::callbacks;
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U128;
use near_sdk::{AccountId, Promise, env, near, require};

/// Retry id generation when a collision exists in [`Contract::prices`] (extremely unlikely).
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
    // -------------------------------------------------------------------------
    // Public catalog admin (pool-owner auth via promise chain)
    // -------------------------------------------------------------------------

    /// Add a price tier under an active product. Pool owner only; attach 1 yocto.
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
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
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
                )
        })
    }

    /// Update display fields only; `amount`, `price_type`, and billing metadata are fixed after create.
    #[payable]
    pub fn edit_price(&mut self, price_id: PriceId, name: String, description: String) -> Promise {
        self.promise_catalog_admin_on_price(price_id, |expected_caller, price_id| {
            ext_self_prices::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .edit_price_after_get_owner(price_id, name, description, expected_caller)
        })
    }

    /// Archive: existing locks/subscriptions are unchanged; new locks must pick an active price.
    #[payable]
    pub fn archive_price(&mut self, price_id: PriceId) -> Promise {
        self.promise_catalog_admin_on_price(price_id, |expected_caller, price_id| {
            ext_self_prices::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .archive_price_after_get_owner(price_id, expected_caller)
        })
    }

    /// Restore an archived price so it can be locked or set as product default again.
    #[payable]
    pub fn unarchive_price(&mut self, price_id: PriceId) -> Promise {
        self.promise_catalog_admin_on_price(price_id, |expected_caller, price_id| {
            ext_self_prices::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .unarchive_price_after_get_owner(price_id, expected_caller)
        })
    }

    /// Delete from storage when never locked (`usage_count == 0`); also drops the id from the parent product list.
    #[payable]
    pub fn delete_price(&mut self, price_id: PriceId) -> Promise {
        self.promise_catalog_admin_on_price(price_id, |expected_caller, price_id| {
            ext_self_prices::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .delete_price_after_get_owner(price_id, expected_caller)
        })
    }

    // -------------------------------------------------------------------------
    // Private callbacks after pool `get_owner_id`
    // -------------------------------------------------------------------------

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
        let mut product = self.require_product(&product_id);
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
        let mut price = self.require_price(&price_id);
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
        let mut price = self.require_price(&price_id);
        let product_id = price.product_id.clone();
        price.status = CatalogStatus::Archived;
        self.prices.insert(price_id.clone(), price);
        // Default must reference an active tier; see `set_product_default_price`.
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
        let price = self.require_price(&price_id);
        require!(
            price.usage_count == 0,
            "Cannot delete this price while it is in use"
        );
        let product_id = price.product_id.clone();
        let mut product = self.require_product(&price.product_id);
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
        let mut price = self.require_price(&price_id);
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
