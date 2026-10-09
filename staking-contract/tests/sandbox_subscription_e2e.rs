//! Sandbox E2E for subscription tier changes (real promise chains, test-feature clock).
//!
//! Complements host-side [`subscription_lifecycle.rs`](subscription_lifecycle.rs).
//!
//! Build: `make staking-contract-test mock-staking-pool-contract`
//! Run: `cargo test -p staking-contract --test sandbox_subscription_e2e`

mod mock_pool;

use mock_pool::{
    buyer_lock_subscription, buyer_storage_deposit, buyer_update_subscription_scheduled,
    buyer_update_subscription_with_stake_increase, buyer_withdraw,
    create_recurring_price_on_product, create_subscription_product_and_price,
    drive_validator_settlement_epochs, fast_forward_subscription_for_product_to,
    json_near_token_yocto, json_u64_field, set_mock_timestamp, setup_staking_fixture,
    top_up_buyer_near,
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

#[tokio::test]
async fn sandbox_test_clock_fast_forwards_only_selected_subscription()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _oneoff_product, _oneoff_price) =
        setup_staking_fixture(&worker).await?;
    let (product_a, price_low_a) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let price_high_a =
        create_recurring_price_on_product(&staking, &owner, &product_a, "High tier", "10").await?;
    let (product_b, price_b) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;

    let buyer = worker.dev_create_account().await?;
    buyer_storage_deposit(&buyer, staking.id()).await?;
    let _lock_a = buyer_lock_subscription(&buyer, staking.id(), &price_high_a, 50).await?;
    top_up_buyer_near(&worker, &buyer, 60).await?;
    let _lock_b = buyer_lock_subscription(&buyer, staking.id(), &price_b, 50).await?;

    let sub_a_before: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_a,
        }))
        .await?
        .json()?;
    let sub_b_before: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_b,
        }))
        .await?
        .json()?;
    let subscription_id = sub_a_before["subscription_id"]
        .as_str()
        .expect("subscription_id");
    let attacker = worker.dev_create_account().await?;

    buyer_update_subscription_scheduled(
        &buyer,
        staking.id(),
        subscription_id,
        &price_low_a,
        near_yocto(25),
    )
    .await?;

    let sub_a_scheduled: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_a,
        }))
        .await?
        .json()?;
    let apply_ns =
        json_u64_field(&sub_a_scheduled["pending_update"]["apply_ns"]).expect("pending apply_ns");
    let sub_a_stored_before_clock: serde_json::Value = worker
        .view(staking.id(), "test_get_stored_subscription")
        .args_json(json!({ "subscription_id": subscription_id }))
        .await?
        .json()?;
    let sub_a_timestamp_before: serde_json::Value = worker
        .view(staking.id(), "test_get_subscription_timestamp")
        .args_json(json!({ "subscription_id": subscription_id }))
        .await?
        .json()?;
    assert!(
        json_u64_field(&sub_a_timestamp_before).expect("subscription timestamp before")
            >= json_u64_field(&sub_a_stored_before_clock["start_ns"])
                .expect("stored sub_a start_ns"),
        "unset subscription test clock should use a current block timestamp"
    );

    let unauthorized = attacker
        .call(staking.id(), "test_fast_forward_subscription_to")
        .args_json(json!({
            "subscription_id": subscription_id,
            "target_timestamp_ns": apply_ns.saturating_add(1).to_string(),
        }))
        .gas(near_workspaces::types::Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        unauthorized.is_failure(),
        "non-owner should not be allowed to modify a subscription test clock"
    );

    let target_timestamp_ns = apply_ns.saturating_add(1);
    fast_forward_subscription_for_product_to(&buyer, staking.id(), &product_a, target_timestamp_ns)
        .await?;
    let sub_a_timestamp_after: serde_json::Value = worker
        .view(staking.id(), "test_get_subscription_timestamp")
        .args_json(json!({ "subscription_id": subscription_id }))
        .await?
        .json()?;
    assert_eq!(
        json_u64_field(&sub_a_timestamp_after).expect("subscription timestamp after"),
        target_timestamp_ns,
        "subscription test clock getter should return the selected virtual timestamp"
    );

    let sub_a_after: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_a,
        }))
        .await?
        .json()?;
    let sub_b_after: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({
            "account_id": buyer.id(),
            "product_id": product_b,
        }))
        .await?
        .json()?;
    let sub_a_stored_after_clock: serde_json::Value = worker
        .view(staking.id(), "test_get_stored_subscription")
        .args_json(json!({ "subscription_id": subscription_id }))
        .await?
        .json()?;

    assert_eq!(sub_a_after["price_id"].as_str(), Some(price_low_a.as_str()));
    assert!(sub_a_after["pending_update"].is_null());
    assert_ne!(
        json_u64_field(&sub_a_after["end_ns"]).expect("sub_a end_ns"),
        json_u64_field(&sub_a_before["end_ns"]).expect("sub_a original end_ns"),
        "selected subscription should move to a later virtual billing window"
    );
    assert_eq!(
        json_u64_field(&sub_a_stored_after_clock["start_ns"]).expect("stored sub_a start_ns"),
        json_u64_field(&sub_a_stored_before_clock["start_ns"])
            .expect("stored sub_a original start_ns"),
        "test clock should not mutate stored subscription start"
    );
    assert_eq!(
        json_u64_field(&sub_a_stored_after_clock["end_ns"]).expect("stored sub_a end_ns"),
        json_u64_field(&sub_a_stored_before_clock["end_ns"]).expect("stored sub_a original end_ns"),
        "test clock should not mutate stored subscription end"
    );
    assert!(
        !sub_a_stored_after_clock["pending_update"].is_null(),
        "test clock should not commit or remove the stored pending update"
    );
    assert_eq!(
        json_u64_field(&sub_b_after["start_ns"]).expect("sub_b start_ns"),
        json_u64_field(&sub_b_before["start_ns"]).expect("sub_b original start_ns"),
        "unselected subscription start should not move"
    );
    assert_eq!(
        json_u64_field(&sub_b_after["end_ns"]).expect("sub_b end_ns"),
        json_u64_field(&sub_b_before["end_ns"]).expect("sub_b original end_ns"),
        "unselected subscription end should not move"
    );

    Ok(())
}

/// A due tier downgrade is released by repeating the target update, without cancelling
/// or unlocking the subscription's remaining stake (regression for issue #169).
#[tokio::test]
async fn sandbox_due_downgrade_releases_surplus_and_withdraws_without_cancelling()
-> Result<(), Box<dyn std::error::Error>> {
    let worker = near_workspaces::sandbox().await?;
    let (staking, pool, owner, _, _) = setup_staking_fixture(&worker).await?;
    let (product_id, price_low) =
        create_subscription_product_and_price(&staking, &pool, &owner).await?;
    let price_high = create_recurring_price_on_product(
        &staking,
        &owner,
        &product_id,
        "Basic",
        &near_yocto(20).to_string(),
    )
    .await?;
    let buyer = worker.dev_create_account().await?;
    let operator = worker.dev_create_account().await?;
    buyer_storage_deposit(&buyer, staking.id()).await?;
    let lock_id = buyer_lock_subscription(&buyer, staking.id(), &price_high, 20).await?;
    drive_validator_settlement_epochs(&worker, &operator, staking.id(), pool.id(), 1).await?;
    let sub: serde_json::Value = worker
        .view(staking.id(), "get_subscription_for_product")
        .args_json(json!({ "account_id": buyer.id(), "product_id": product_id }))
        .await?
        .json()?;
    let subscription_id = sub["subscription_id"].as_str().expect("subscription_id");
    buyer_update_subscription_scheduled(
        &buyer,
        staking.id(),
        subscription_id,
        &price_low,
        near_yocto(1),
    )
    .await?;
    let end_ns = json_u64_field(&sub["end_ns"]).expect("end_ns");
    fast_forward_subscription_for_product_to(
        &buyer,
        staking.id(),
        &product_id,
        end_ns.saturating_add(1),
    )
    .await?;

    // Views project the new stake, but do not create a withdrawal entitlement.
    let raw: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id, "effective": false }))
        .await?
        .json()?;
    let effective: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id, "effective": true }))
        .await?
        .json()?;
    assert_eq!(
        json_near_token_yocto(&raw["amount_near"]),
        Some(near_yocto(20))
    );
    assert_eq!(
        json_near_token_yocto(&effective["amount_near"]),
        Some(near_yocto(1))
    );
    let before: serde_json::Value = worker
        .view(staking.id(), "get_account_pending_unstake")
        .args_json(json!({ "account_id": buyer.id(), "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(before["total_pending_yocto"], json!("0"));

    // Repeating the due target commits the decrease and queues only the surplus.
    buyer_update_subscription_scheduled(
        &buyer,
        staking.id(),
        subscription_id,
        &price_low,
        near_yocto(1),
    )
    .await?;
    let released: serde_json::Value = worker
        .view(staking.id(), "get_account_pending_unstake")
        .args_json(json!({ "account_id": buyer.id(), "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(
        released["total_pending_yocto"],
        json!(near_yocto(19).to_string())
    );
    let retained: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id, "effective": false }))
        .await?
        .json()?;
    assert_eq!(
        json_near_token_yocto(&retained["amount_near"]),
        Some(near_yocto(1))
    );
    assert_eq!(retained["status"], json!("Active"));

    // A retry must not release the retained stake or create another tranche.
    buyer_update_subscription_scheduled(
        &buyer,
        staking.id(),
        subscription_id,
        &price_low,
        near_yocto(1),
    )
    .await?;
    let retried: serde_json::Value = worker
        .view(staking.id(), "get_account_pending_unstake")
        .args_json(json!({ "account_id": buyer.id(), "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(
        retried["total_pending_yocto"],
        released["total_pending_yocto"]
    );

    drive_validator_settlement_epochs(&worker, &operator, staking.id(), pool.id(), 2).await?;
    let ready: serde_json::Value = worker
        .view(staking.id(), "get_account_pending_unstake")
        .args_json(json!({ "account_id": buyer.id(), "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(ready["can_withdraw_now"], json!(true));
    assert_eq!(
        ready["withdrawable_yocto"],
        json!(near_yocto(19).to_string())
    );
    let balance_before = buyer.view_account().await?.balance.as_yoctonear();
    buyer_withdraw(&buyer, staking.id(), pool.id()).await?;
    let balance_after = buyer.view_account().await?.balance.as_yoctonear();
    assert!(
        balance_after > balance_before + near_yocto(18),
        "buyer receives surplus less gas"
    );
    let done: serde_json::Value = worker
        .view(staking.id(), "get_account_pending_unstake")
        .args_json(json!({ "account_id": buyer.id(), "validator_id": pool.id() }))
        .await?
        .json()?;
    assert_eq!(done["total_pending_yocto"], json!("0"));
    assert_eq!(done["can_withdraw_now"], json!(false));
    let active: serde_json::Value = worker
        .view(staking.id(), "get_subscription")
        .args_json(json!({ "subscription_id": subscription_id }))
        .await?
        .json()?;
    assert_eq!(active["status"], json!("Active"));
    assert_eq!(active["cancel_at_period_end"], json!(false));
    assert_eq!(active["price_id"], json!(price_low));
    assert!(active["pending_update"].is_null());
    let final_lock: serde_json::Value = worker
        .view(staking.id(), "get_lock")
        .args_json(json!({ "lock_id": lock_id }))
        .await?
        .json()?;
    assert_eq!(
        json_near_token_yocto(&final_lock["amount_near"]),
        Some(near_yocto(1))
    );
    assert_eq!(final_lock["status"], json!("Active"));
    Ok(())
}
