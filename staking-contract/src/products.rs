//! Catalog **products**: create/edit/archive/delete, pagination, default price binding, and the
//! [`ExtSelfProducts`] pool-owner callback trait.
//!
//! **Price** entrypoints, [`ExtSelfPrices`], and `get_price` are in [`crate::prices`].

use crate::epoch::ext_staking_pool;
use crate::gas::{callbacks, staking_pool};
use crate::*;
use near_sdk::ext_contract;
use near_sdk::json_types::U64;
use near_sdk::{AccountId, Promise, env, is_promise_success, near, require};

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
        require!(p.is_some(), "Product not found in the catalog");
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
        require!(p.is_some(), "Product not found in the catalog");
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
        require!(p.is_some(), "Product not found in the catalog");
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
    pub fn unarchive_product(&mut self, product_id: ProductId) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self.products.get(&product_id).cloned();
        require!(p.is_some(), "Product not found in the catalog");
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
                    .unarchive_product_after_get_owner(product_id, expected_caller),
            )
    }

    #[payable]
    pub fn set_product_default_price(
        &mut self,
        product_id: ProductId,
        price_id: Option<PriceId>,
    ) -> Promise {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self.products.get(&product_id).cloned();
        require!(p.is_some(), "Product not found in the catalog");
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
                    .set_product_default_price_after_get_owner(
                        product_id,
                        price_id,
                        expected_caller,
                    ),
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
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );

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
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
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
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
        p.default_price_id = None;
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
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
        require!(
            p.usage_count == 0,
            "Cannot delete this product while it is in use"
        );
        require!(
            p.price_ids.is_empty(),
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
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
        require!(
            p.status == CatalogStatus::Archived,
            "Product is not archived"
        );
        p.status = CatalogStatus::Active;
        self.products.insert(product_id, p);
    }

    #[private]
    pub fn set_product_default_price_after_get_owner(
        &mut self,
        #[callback] pool_owner: AccountId,
        product_id: ProductId,
        price_id: Option<PriceId>,
        expected_caller: AccountId,
    ) {
        require!(
            is_promise_success(),
            "Could not read the validator pool owner; try again later"
        );
        self.assert_not_paused();
        require!(
            pool_owner == expected_caller,
            "Only the validator owner can call this method"
        );
        let mut product = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Product not found in the catalog");
        require!(
            product.status == CatalogStatus::Active,
            "This product is archived or inactive"
        );
        match price_id {
            None => {
                product.default_price_id = None;
            }
            Some(pid) => {
                let pr = self
                    .prices
                    .get(&pid)
                    .cloned()
                    .expect("Price not found in the catalog");
                require!(
                    pr.product_id == product_id,
                    "Price does not belong to this product"
                );
                require!(
                    pr.status == CatalogStatus::Active,
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
            .and_then(|p| p.default_price_id.clone())
    }

    /// Paginated catalog rows (stable creation order in [`Contract::product_ids`]).
    pub fn get_products(&self, from_index: u64, limit: u64) -> Vec<Product> {
        let len_u64 = self.product_ids.len() as u64;
        let mut out = Vec::new();
        let mut i = from_index;
        while i < len_u64 && (out.len() as u64) < limit {
            if let Some(id) = self.product_ids.get(i as u32) {
                if let Some(p) = self.products.get(id).cloned() {
                    out.push(p);
                }
            }
            i += 1;
        }
        out
    }
}

impl Contract {
    /// Clears [`Product::default_price_id`] when it references **`price_id`** (e.g. price archived/deleted).
    pub(crate) fn clear_product_default_price_field_if_matches(
        &mut self,
        product_id: &ProductId,
        price_id: &PriceId,
    ) {
        let mut p = match self.products.get(product_id).cloned() {
            Some(x) => x,
            None => return,
        };
        if p.default_price_id.as_ref() == Some(price_id) {
            p.default_price_id = None;
            self.products.insert(product_id.clone(), p);
        }
    }

    fn remove_product_id_from_list(&mut self, product_id: &ProductId) {
        let len = self.product_ids.len();
        for i in 0..len {
            if self.product_ids.get(i).is_some_and(|s| s == product_id) {
                for j in (i + 1)..len {
                    let id = self.product_ids.get(j).cloned().unwrap_or_else(|| {
                        env::panic_str("Catalog index error while removing product id")
                    });
                    self.product_ids.set(j - 1, id);
                }
                let _ = self.product_ids.pop();
                return;
            }
        }
    }
}
