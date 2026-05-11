//! `EVENT_JSON` logs for indexers (NEP-297-style payload; standard `stake.dao`).

use near_sdk::AccountId;
use near_sdk::log;

fn emit(event: &str, data: serde_json::Value) {
    let payload = serde_json::json!({
        "standard": "stake.dao",
        "version": "1.0.0",
        "event": event,
        "data": data,
    });
    log!("EVENT_JSON:{}", payload);
}

pub fn log_validator_added(pool: &AccountId) {
    emit(
        "validator_add",
        serde_json::json!({
            "pool": pool.to_string(),
        }),
    );
}

pub fn log_product_created(product_id: &str, validator_id: &AccountId) {
    emit(
        "product_create",
        serde_json::json!({
            "product_id": product_id,
            "validator_id": validator_id.to_string(),
        }),
    );
}

pub fn log_subscription_cancel(account: &AccountId, product_id: &str) {
    emit(
        "subscription_cancel",
        serde_json::json!({
            "account_id": account.to_string(),
            "product_id": product_id,
        }),
    );
}

pub fn log_subscription_upgrade(account: &AccountId, new_price_id: &str) {
    emit(
        "subscription_upgrade",
        serde_json::json!({
            "account_id": account.to_string(),
            "new_price_id": new_price_id,
        }),
    );
}

pub fn log_subscription_downgrade_scheduled(account: &AccountId, target_price_id: &str) {
    emit(
        "subscription_downgrade_scheduled",
        serde_json::json!({
            "account_id": account.to_string(),
            "target_price_id": target_price_id,
        }),
    );
}

pub fn log_subscription_downgrade_prorate(account: &AccountId, product_id: &str, near_yocto: u128) {
    emit(
        "subscription_downgrade_prorate",
        serde_json::json!({
            "account_id": account.to_string(),
            "product_id": product_id,
            "near_yocto": near_yocto.to_string(),
        }),
    );
}

pub fn log_lock(lock_id: &str, account: &AccountId) {
    emit(
        "lock_create",
        serde_json::json!({
            "lock_id": lock_id,
            "account_id": account.to_string(),
        }),
    );
}

pub fn log_unlock(lock_id: &str, account: &AccountId, validator: &AccountId) {
    emit(
        "unlock",
        serde_json::json!({
            "lock_id": lock_id,
            "account_id": account.to_string(),
            "validator_id": validator.to_string(),
        }),
    );
}

pub fn log_claim_unlocked(account: &AccountId, validator_id: &AccountId) {
    emit(
        "claim_unlocked",
        serde_json::json!({
            "account_id": account.to_string(),
            "validator_id": validator_id.to_string(),
        }),
    );
}

pub fn log_withdraw(account: &AccountId, amount_yocto: u128) {
    emit(
        "withdraw",
        serde_json::json!({
            "account_id": account.to_string(),
            "amount_yocto": amount_yocto.to_string(),
        }),
    );
}

pub fn log_epoch_operation(epoch_action: &str, validator_id: &AccountId) {
    emit(
        epoch_action,
        serde_json::json!({
            "epoch_action": epoch_action,
            "validator_id": validator_id.to_string(),
        }),
    );
}

pub fn log_pool_withdraw_in(amount_yocto: u128, validator_id: &AccountId) {
    emit(
        "pool_withdraw_in",
        serde_json::json!({
            "validator_id": validator_id.to_string(),
            "amount_yocto": amount_yocto.to_string(),
        }),
    );
}
