//! Sandbox E2E for subscription tier changes (real promise chains, test-feature clock).
//!
//! Complements host-side [`subscription_lifecycle.rs`](subscription_lifecycle.rs).
//!
//! Build: `make staking-contract-test mock-staking-pool-contract`
//! Run: `cargo test -p staking-contract --test sandbox_subscription_e2e`

mod mock_pool;

use mock_pool::{
    buyer_lock_subscription, buyer_storage_deposit, buyer_update_subscription_scheduled,
    buyer_update_subscription_with_stake_increase, create_recurring_price_on_product,
    create_subscription_product_and_price, json_near_token_yocto, json_u64_field,
    set_mock_timestamp, setup_staking_fixture, top_up_buyer_near,
};
use serde_json::json;

fn near_yocto(near: u128) -> u128 {
    near_sdk::NearToken::from_near(near).as_yoctonear()
}

#[tokio::test]
async fn sandbox_update_subscription_raises_tier_and_lock_amount()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _oneoff_product, _oneoff_price) =
        setup_staking_fixture(&worker).await?;
    let (product_id, price_low) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let price_high =
        create_recurring_price_on_product(&staking, &owner, &product_id, "High tier", "10").await?;

    let buyer = worker.dev_create_account().await?;
    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id = buyer_lock_subscription(&buyer, staking.id(), &price_low, 50).await?;

    let lock_before: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let amount_before = json_near_token_yocto(&lock_before["amount_near"]).unwrap_or(0);
    let sub_before: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_id,
        }))
        .await?
        .json()?;
    let subscription_id = sub_before["subscription_id"]
        .as_str()
        .expect("subscription_id");

    top_up_buyer_near(&worker, &buyer, 50).await?;
    let _same_lock = buyer_update_subscription_with_stake_increase(
        &buyer,
        staking.id(),
        subscription_id,
        &price_high,
        near_yocto(90),
        40,
    )
    .await?;

    let sub: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_id,
        }))
        .await?
        .json()?;
    assert_eq!(sub["price_id"].as_str(), Some(price_high.as_str()));

    let lock_after: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    let amount_after = json_near_token_yocto(&lock_after["amount_near"]).unwrap_or(0);
    assert!(
        amount_after > amount_before,
        "update should increase locked NEAR on the subscription lock"
    );
    assert_eq!(lock_after["status"], json!("Active"));

    Ok(())
}

#[tokio::test]
async fn sandbox_scheduled_update_projects_without_manual_lock()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _oneoff_product, _oneoff_price) =
        setup_staking_fixture(&worker).await?;
    let (product_id, price_low) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let price_high =
        create_recurring_price_on_product(&staking, &owner, &product_id, "High tier", "10").await?;

    let buyer = worker.dev_create_account().await?;
    buyer_storage_deposit(&buyer, staking.id()).await?;
    let _lock_high = buyer_lock_subscription(&buyer, staking.id(), &price_high, 50).await?;
    let sub_before: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_id,
        }))
        .await?
        .json()?;
    let subscription_id = sub_before["subscription_id"]
        .as_str()
        .expect("subscription_id");

    buyer_update_subscription_scheduled(
        &buyer,
        staking.id(),
        subscription_id,
        &price_low,
        near_yocto(25),
    )
    .await?;

    let sub: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_id,
        }))
        .await?
        .json()?;
    assert_eq!(
        sub["pending_update"]["target_price_id"].as_str(),
        Some(price_low.as_str())
    );

    let end_ns = json_u64_field(&sub["end_ns"]).expect("end_ns");
    set_mock_timestamp(&buyer, staking.id(), end_ns.saturating_add(1)).await?;

    let sub_after: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_id,
        }))
        .await?
        .json()?;
    assert_eq!(sub_after["price_id"].as_str(), Some(price_low.as_str()));
    assert_eq!(
        json_u64_field(&sub_after["start_ns"]).expect("start_ns"),
        end_ns
    );
    assert!(sub_after["pending_update"].is_null());
    assert_eq!(sub_after["status"], json!("Active"));

    Ok(())
}
