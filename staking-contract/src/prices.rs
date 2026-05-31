//! Catalog prices: CRUD + archive lifecycle for product tiers.
//! Mutating RPCs are pool-owner gated via `get_owner_id` + [`Contract::assert_validator_owner`].
//! Archiving/deleting a default tier clears [`Product::default_price_id`] through product helpers.

use crate::gas::callbacks;
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U128;
use near_sdk::{AccountId, Promise, env, near, require};

/// Retry id generation when a collision exists in [`Contract::prices`] (extremely unlikely).
fn next_unique_price_id(contract: &mut Contract) -> PriceId {
    crate::ids::next_unique_generated_id(
        &mut contract.id_nonce,
        crate::ids::next_price_id,
        |id| contract.prices.contains_key(id),
        "Could not allocate a unique price id; try again",
    )
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
        metadata: Option<PriceMetadata>,
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
        metadata: Option<PriceMetadata>,
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
                    metadata,
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

    // Private callbacks after pool `get_owner_id`.

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
        metadata: Option<PriceMetadata>,
        expected_caller: AccountId,
    ) -> PriceId {
        self.assert_validator_owner(pool_owner, &expected_caller);
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
            metadata,
            status: CatalogStatus::Active,
            usage_count: 0,
        };
        self.internal_set_price(price_id.clone(), price);
        product.price_ids.push(price_id.clone());
        self.internal_set_product(product_id, product);
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
        self.assert_validator_owner(pool_owner, &expected_caller);
        let mut price = self.require_price(&price_id);
        price.name = name;
        price.description = description;
        self.internal_set_price(price_id, price);
    }

    #[private]
    pub fn archive_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        price_id: PriceId,
        expected_caller: AccountId,
    ) {
        self.assert_validator_owner(pool_owner, &expected_caller);
        let mut price = self.require_price(&price_id);
        self.assert_no_pending_update_references_price(&price_id);
        let product_id = price.product_id.clone();
        price.status = CatalogStatus::Archived;
        self.internal_set_price(price_id.clone(), price);
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
        self.assert_validator_owner(pool_owner, &expected_caller);
        let price = self.require_price(&price_id);
        require!(
            price.usage_count == 0,
            "Cannot delete this price while it is in use"
        );
        self.assert_no_pending_update_references_price(&price_id);
        let product_id = price.product_id.clone();
        let mut product = self.require_product(&price.product_id);
        product.price_ids.retain(|x| x != &price_id);
        self.internal_set_product(price.product_id.clone(), product);
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
        self.assert_validator_owner(pool_owner, &expected_caller);
        let mut price = self.require_price(&price_id);
        require!(
            price.status == CatalogStatus::Archived,
            "Price is not archived"
        );
        price.status = CatalogStatus::Active;
        self.internal_set_price(price_id, price);
    }

    // Public price view functions.

    pub fn get_price(&self, price_id: PriceId) -> Option<Price> {
        self.internal_get_price(&price_id)
    }
}

impl Contract {
    pub(crate) fn internal_get_price(&self, id: &PriceId) -> Option<Price> {
        self.prices.get(id).cloned().map(Into::into)
    }

    pub(crate) fn internal_set_price(&mut self, id: PriceId, price: Price) {
        self.prices.insert(id, price.into());
    }

    pub(crate) fn require_price(&self, price_id: &PriceId) -> Price {
        self.internal_get_price(price_id)
            .unwrap_or_else(|| env::panic_str("Price not found in the catalog"))
    }

    pub(crate) fn require_price_and_product(&self, price_id: &PriceId) -> (Price, Product) {
        let price = self.require_price(price_id);
        let product = self.require_product(&price.product_id);
        (price, product)
    }

    pub(crate) fn get_active_price_and_product(&self, price_id: &PriceId) -> (Price, Product) {
        let price = self.require_price(price_id);
        require!(
            price.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        let product = self.require_product(&price.product_id);
        require!(
            product.status == CatalogStatus::Active,
            "This product is not active; pick an active product"
        );
        (price, product)
    }

    pub(crate) fn require_recurring_monthly_price(&self, price: &Price) {
        require!(
            price.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );
        require!(
            price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );
    }

    /// Resolve price → product → pool, run catalog admin preamble, then `get_owner_id` → `build_tail(caller, price_id)`.
    pub(crate) fn promise_catalog_admin_on_price(
        &self,
        price_id: PriceId,
        build_tail: impl FnOnce(AccountId, PriceId) -> Promise,
    ) -> Promise {
        let (_, product) = self.require_price_and_product(&price_id);
        let (validator_id, expected_caller) =
            self.catalog_admin_entry_for_pool(&product.validator_id);
        Self::promise_pool_get_owner_id_then(validator_id, build_tail(expected_caller, price_id))
    }
}
