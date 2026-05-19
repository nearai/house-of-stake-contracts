//! Catalog **products**: create/edit/archive/delete, pagination, default price binding, and the
//! [`ExtSelfProducts`] pool-owner callback trait.
//!
//! **Auth:** Same pool-owner promise pattern as [`crate::prices`]: public RPC → `get_owner_id` on the
//! product's validator pool → [`ExtSelfProducts`] callback with [`Contract::assert_pool_owner_callback`].
//!
//! **Prices** live in [`crate::prices`]; this module owns [`Contract::products`], the [`Product::price_ids`]
//! list, and [`Product::default_price_id`] (used by [`crate::lock::Contract::lock_for_product`] when callers pass
//! `product_id` only).

use crate::gas::callbacks;
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, Promise, env, near, require};

/// Retry id generation when a collision exists in [`Contract::products`].
fn next_unique_product_id(contract: &mut Contract) -> ProductId {
    for _ in 0..64 {
        let id = crate::ids::next_product_id(&mut contract.id_nonce);
        if !contract.products.contains_key(&id) {
            return id;
        }
    }
    env::panic_str("Could not allocate a unique product id; try again")
}

/// Self callbacks for **product** catalog after `get_owner_id` on the staking pool.
#[ext_contract(ext_self_products)]
pub trait ExtSelfProducts {
    fn create_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        validator_id: ValidatorId,
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
    fn unarchive_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    );
    fn set_product_default_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        price_id: Option<PriceId>,
        expected_caller: AccountId,
    );
}

#[near]
impl Contract {
    // -------------------------------------------------------------------------
    // Public catalog admin (pool-owner auth via promise chain)
    // -------------------------------------------------------------------------

    /// Register a sellable product on an allowlisted validator pool. Pool owner only; attach 1 yocto.
    #[payable]
    pub fn create_product(
        &mut self,
        validator_id: ValidatorId,
        name: String,
        description: String,
    ) -> Promise {
        self.promise_catalog_admin_on_pool(&validator_id, |expected_caller, validator_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .create_product_after_get_owner(validator_id, name, description, expected_caller)
        })
    }

    /// Update product display metadata; does not move the product to another pool.
    #[payable]
    pub fn edit_product(
        &mut self,
        product_id: ProductId,
        name: String,
        description: String,
    ) -> Promise {
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .edit_product_after_get_owner(product_id, name, description, expected_caller)
        })
    }

    /// Archive: blocks new locks; clears default price; existing locks/subscriptions unchanged.
    #[payable]
    pub fn archive_product(&mut self, product_id: ProductId) -> Promise {
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .archive_product_after_get_owner(product_id, expected_caller)
        })
    }

    /// Remove from storage when never locked and all prices for this product are gone.
    #[payable]
    pub fn delete_product(&mut self, product_id: ProductId) -> Promise {
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .delete_product_after_get_owner(product_id, expected_caller)
        })
    }

    /// Restore an archived product for new locks and default-price binding.
    #[payable]
    pub fn unarchive_product(&mut self, product_id: ProductId) -> Promise {
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .unarchive_product_after_get_owner(product_id, expected_caller)
        })
    }

    /// Bind or clear the tier used when users call `lock_for_*` with `product_id` only (no `price_id`).
    #[payable]
    pub fn set_product_default_price(
        &mut self,
        product_id: ProductId,
        price_id: Option<PriceId>,
    ) -> Promise {
        self.promise_catalog_admin_on_product(product_id, |expected_caller, product_id| {
            ext_self_products::ext(env::current_account_id())
                .with_static_gas(callbacks::ON_VALIDATOR_OWNER_CHECK)
                .set_product_default_price_after_get_owner(product_id, price_id, expected_caller)
        })
    }

    // -------------------------------------------------------------------------
    // Private callbacks after pool `get_owner_id`
    // -------------------------------------------------------------------------

    #[private]
    pub fn create_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        validator_id: ValidatorId,
        name: String,
        description: String,
        expected_caller: AccountId,
    ) -> ProductId {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);

        let id = next_unique_product_id(self);
        let product = Product {
            product_id: id.clone(),
            validator_id: validator_id.clone(),
            name,
            description,
            status: CatalogStatus::Active,
            created_ns: U64(env::block_timestamp()),
            price_ids: Vec::new(),
            default_price_id: None,
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
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut product = self.require_product(&product_id);
        product.name = name;
        product.description = description;
        self.products.insert(product_id, product);
    }

    #[private]
    pub fn archive_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut product = self.require_product(&product_id);
        // Archived products cannot serve as default; clear so lock-by-product fails fast.
        product.default_price_id = None;
        product.status = CatalogStatus::Archived;
        self.products.insert(product_id, product);
    }

    #[private]
    pub fn delete_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let product = self.require_product(&product_id);
        require!(
            product.usage_count == 0,
            "Cannot delete this product while it is in use"
        );
        require!(
            product.price_ids.is_empty(),
            "Remove or delete all prices for this product first"
        );
        self.remove_product_id_from_list(&product_id);
        self.products.remove(&product_id);
    }

    #[private]
    pub fn unarchive_product_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut product = self.require_product(&product_id);
        require!(
            product.status == CatalogStatus::Archived,
            "Product is not archived"
        );
        product.status = CatalogStatus::Active;
        self.products.insert(product_id, product);
    }

    #[private]
    pub fn set_product_default_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        price_id: Option<PriceId>,
        expected_caller: AccountId,
    ) {
        self.assert_pool_owner_callback(pool_owner, &expected_caller);
        let mut product = self.require_product(&product_id);
        require!(
            product.status == CatalogStatus::Active,
            "This product is archived or inactive"
        );
        match price_id {
            None => {
                product.default_price_id = None;
            }
            Some(pid) => {
                let catalog_price = self.require_price(&pid);
                require!(
                    catalog_price.product_id == product_id,
                    "Price does not belong to this product"
                );
                require!(
                    catalog_price.status == CatalogStatus::Active,
                    "Only an active (unarchived) price can be the default"
                );
                require!(
                    product.price_ids.iter().any(|x| x == &pid),
                    "Price not listed on product"
                );
                product.default_price_id = Some(pid);
            }
        }
        self.products.insert(product_id, product);
    }

    pub fn get_product(&self, product_id: ProductId) -> Option<Product> {
        self.products.get(&product_id).cloned()
    }

    pub fn get_product_default_price(&self, product_id: ProductId) -> Option<PriceId> {
        self.products
            .get(&product_id)
            .and_then(|product| product.default_price_id.clone())
    }

    /// Paginated products (stable creation order in [`Contract::product_ids`]).
    pub fn get_products(&self, from_index: u64, limit: u64) -> Vec<Product> {
        let len_u64 = self.product_ids.len() as u64;
        let mut out = Vec::new();
        let mut i = from_index;
        while i < len_u64 && (out.len() as u64) < limit {
            if let Some(id) = self.product_ids.get(i as u32) {
                if let Some(catalog_product) = self.products.get(id).cloned() {
                    out.push(catalog_product);
                }
            }
            i += 1;
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (also used from [`crate::prices`])
// ---------------------------------------------------------------------------

impl Contract {
    /// Clears [`Product::default_price_id`] when it references **`price_id`** (e.g. price archived/deleted).
    pub(crate) fn clear_product_default_price_field_if_matches(
        &mut self,
        product_id: &ProductId,
        price_id: &PriceId,
    ) {
        let mut product = match self.products.get(product_id).cloned() {
            Some(existing) => existing,
            None => return,
        };
        if product.default_price_id.as_ref() == Some(price_id) {
            product.default_price_id = None;
            self.products.insert(product_id.clone(), product);
        }
    }

    /// Keeps [`Contract::product_ids`] in sync when a product is removed from storage.
    fn remove_product_id_from_list(&mut self, product_id: &ProductId) {
        let len = self.product_ids.len();
        for i in 0..len {
            if self.product_ids.get(i).is_some_and(|id| id == product_id) {
                self.product_ids.drain(i..i + 1);
                return;
            }
        }
    }
}
