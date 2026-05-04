use crate::*;
use near_sdk::json_types::{U128, U64};
use near_sdk::{env, near, require};

#[near]
impl Contract {
    #[payable]
    pub fn create_product(
        &mut self,
        validator_id: AccountId,
        name: String,
        description: String,
    ) -> ProductId {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        self.assert_validator_owner(&validator_id);

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
        id
    }

    #[payable]
    pub fn edit_product(&mut self, product_id: ProductId, name: String, description: String) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&p.validator_id);
        p.name = name;
        p.description = description;
        self.products.insert(product_id, p);
    }

    #[payable]
    pub fn archive_product(&mut self, product_id: ProductId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let mut p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&p.validator_id);
        p.status = CatalogStatus::Archived;
        self.products.insert(product_id, p);
    }

    #[payable]
    pub fn delete_product(&mut self, product_id: ProductId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let p = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&p.validator_id);
        require!(p.usage_count == 0, "Product in use");
        require!(
            p.price_ids.is_empty(),
            "Remove or delete all prices for this product first"
        );
        self.products.remove(&product_id);
    }

    #[payable]
    pub fn create_price(
        &mut self,
        product_id: ProductId,
        name: String,
        description: String,
        currency: Currency,
        amount: U128,
        price_type: PriceType,
        billing_period: Option<BillingPeriod>,
        lock_factor_near_months: U128,
    ) -> PriceId {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let mut product = self
            .products
            .get(&product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&product.validator_id);
        require!(product.status == CatalogStatus::Active, "Product archived");

        let price_id = crate::ids::next_price_id(&mut self.id_nonce);
        let price = Price {
            price_id: price_id.clone(),
            product_id: product_id.clone(),
            name,
            description,
            currency,
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

    #[payable]
    pub fn edit_price(&mut self, price_id: PriceId, name: String, description: String) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let mut pr = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Unknown price");
        let product = self
            .products
            .get(&pr.product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&product.validator_id);
        pr.name = name;
        pr.description = description;
        self.prices.insert(price_id, pr);
    }

    #[payable]
    pub fn archive_price(&mut self, price_id: PriceId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let mut pr = self
            .prices
            .get(&price_id)
            .cloned()
            .expect("Unknown price");
        let product = self
            .products
            .get(&pr.product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&product.validator_id);
        pr.status = CatalogStatus::Archived;
        self.prices.insert(price_id, pr);
    }

    #[payable]
    pub fn delete_price(&mut self, price_id: PriceId) {
        near_sdk::assert_one_yocto();
        self.assert_not_paused();
        let pr = self.prices.get(&price_id).cloned().expect("Unknown price");
        let mut product = self
            .products
            .get(&pr.product_id)
            .cloned()
            .expect("Unknown product");
        self.assert_validator_owner(&product.validator_id);
        require!(pr.usage_count == 0, "Price in use");
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
}
