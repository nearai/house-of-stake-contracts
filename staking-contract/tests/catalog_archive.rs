//! Archived catalog entries reject new locks.

mod common;

use common::{
    BUYER, VALIDATOR_OWNER_ACCOUNT, acct, ctx, deploy, register_buyer, setup_catalog_near_oneoff,
    testing_env_catalog_callback,
};
use near_sdk::json_types::U64;
use near_sdk::{NearToken, testing_env};

#[test]
#[should_panic(expected = "Price not active")]
fn lock_for_product_rejects_archived_price() {
    let mut c = deploy();
    let (_pid, price_id) = setup_catalog_near_oneoff(&mut c);
    register_buyer(&mut c);

    testing_env_catalog_callback(acct(VALIDATOR_OWNER_ACCOUNT));
    c.archive_price_after_get_owner(
        acct(VALIDATOR_OWNER_ACCOUNT),
        price_id.clone(),
        acct(VALIDATOR_OWNER_ACCOUNT),
    );

    let dur = c.config.min_lock_duration_ns.0.saturating_add(10_000);
    testing_env!(ctx(acct(BUYER), NearToken::from_near(50)));
    c.lock_for_product(Some(price_id), U64(dur), None);
}
