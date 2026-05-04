//! JSON intent for USD oracle path (versioning).

use serde_json::json;
use staking_contract::oracle_receiver::LockForProductUsdMsg;

#[test]
fn lock_for_product_usd_msg_default_schema_version() {
    let v: LockForProductUsdMsg = serde_json::from_value(json!({
        "price_id": "price_123",
        "lock_duration_ns": 86400000000000_u64
    }))
    .expect("parse");
    assert_eq!(v.schema_version, 0);
    assert_eq!(v.lock_duration_ns, 86_400_000_000_000);
}

#[test]
fn lock_for_product_usd_msg_explicit_schema_v1() {
    let v: LockForProductUsdMsg = serde_json::from_value(json!({
        "schema_version": 1,
        "price_id": "price_123",
        "lock_duration_ns": 1000
    }))
    .expect("parse");
    assert_eq!(v.schema_version, 1);
}
