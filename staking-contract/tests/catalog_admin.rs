//! Catalog admin rules: `delete_product` / `delete_price` invariants.

mod common;

use common::{VOWNER, acct, ctx, deploy, setup_catalog_near_oneoff};
use near_sdk::{NearToken, testing_env};

#[test]
#[should_panic]
fn delete_product_fails_while_prices_attached() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_near_oneoff(&mut c);

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    c.delete_product(product_id);
}

#[test]
fn delete_product_succeeds_when_empty() {
    let mut c = deploy();
    testing_env!(ctx(acct(common::OWNER), NearToken::from_yoctonear(1)));
    c.add_validator(acct(common::POOL), acct(VOWNER));

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    let product_id = c.create_product(acct(common::POOL), "X".into(), "Y".into());

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    c.delete_product(product_id);
}

#[test]
#[should_panic]
fn delete_price_fails_when_in_use() {
    let mut c = deploy();
    let (_product_id, price_id) = setup_catalog_near_oneoff(&mut c);
    common::register_buyer(&mut c);

    let dur = c.config.min_lock_duration_ns.0.saturating_add(1);
    testing_env!(ctx(common::acct(common::BUYER), NearToken::from_near(50)));
    let _ = c.lock_for_product(price_id.clone(), near_sdk::json_types::U64(dur));

    testing_env!(ctx(acct(VOWNER), NearToken::from_yoctonear(1)));
    c.delete_price(price_id);
}
