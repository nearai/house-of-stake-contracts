//! Catalog admin rules: `delete_product` / `delete_price` invariants.

mod common;

use common::{VOWNER, acct, ctx, deploy, setup_catalog_near_oneoff, testing_env_catalog_callback};
use near_sdk::{NearToken, testing_env};

#[test]
#[should_panic]
fn delete_product_fails_while_prices_attached() {
    let mut c = deploy();
    let (product_id, _price_id) = setup_catalog_near_oneoff(&mut c);

    testing_env_catalog_callback(acct(VOWNER));
    c.delete_product_after_get_owner(acct(VOWNER), product_id, acct(VOWNER));
}

#[test]
fn delete_product_succeeds_when_empty() {
    let mut c = deploy();
    testing_env!(ctx(acct(common::OWNER), NearToken::from_yoctonear(1)));
    c.add_validator(acct(common::POOL), acct(VOWNER));

    testing_env_catalog_callback(acct(VOWNER));
    let product_id = c.create_product_after_get_owner(
        acct(VOWNER),
        acct(common::POOL),
        "X".into(),
        "Y".into(),
        acct(VOWNER),
    );

    testing_env_catalog_callback(acct(VOWNER));
    c.delete_product_after_get_owner(acct(VOWNER), product_id, acct(VOWNER));
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

    testing_env_catalog_callback(acct(VOWNER));
    c.delete_price_after_get_owner(acct(VOWNER), price_id, acct(VOWNER));
}
