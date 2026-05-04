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

pub fn log_claim_unlocked(account: &AccountId, validator_pool: &AccountId) {
    emit(
        "claim_unlocked",
        serde_json::json!({
            "account_id": account.to_string(),
            "validator_id": validator_pool.to_string(),
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

pub fn log_epoch_operation(event: &str, validator_pool: &AccountId) {
    emit(
        event,
        serde_json::json!({
            "validator_id": validator_pool.to_string(),
        }),
    );
}

pub fn log_pool_withdraw_in(amount_yocto: u128, validator_pool: &AccountId) {
    emit(
        "pool_withdraw_in",
        serde_json::json!({
            "validator_id": validator_pool.to_string(),
            "amount_yocto": amount_yocto.to_string(),
        }),
    );
}
