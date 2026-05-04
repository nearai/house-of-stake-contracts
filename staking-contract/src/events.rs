//! `EVENT_JSON` logs for indexers (standard: `stakedao`).

use near_sdk::log;
use near_sdk::AccountId;

pub fn log_validator_added(pool: &AccountId) {
    log!("EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"validator_add\",\"pool\":\"{}\"}}", pool);
}

pub fn log_product_created(product_id: &str, validator_id: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"product_create\",\"product_id\":\"{}\",\"validator\":\"{}\"}}",
        product_id,
        validator_id
    );
}

pub fn log_lock(lock_id: &str, account: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"lock_create\",\"lock_id\":\"{}\",\"account\":\"{}\"}}",
        lock_id,
        account
    );
}

pub fn log_unlock(lock_id: &str, account: &AccountId, validator: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"unlock\",\"lock_id\":\"{}\",\"account\":\"{}\",\"validator\":\"{}\"}}",
        lock_id,
        account,
        validator
    );
}

pub fn log_claim_unlocked(account: &AccountId, validator_pool: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"claim_unlocked\",\"account\":\"{}\",\"validator\":\"{}\"}}",
        account,
        validator_pool
    );
}

pub fn log_withdraw(account: &AccountId, amount_yocto: u128) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"withdraw\",\"account\":\"{}\",\"amount_yocto\":\"{}\"}}",
        account,
        amount_yocto
    );
}

pub fn log_epoch_operation(event: &str, validator_pool: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"{}\",\"validator\":\"{}\"}}",
        event,
        validator_pool
    );
}

pub fn log_pool_withdraw_in(amount_yocto: u128, validator_pool: &AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"pool_withdraw_in\",\"validator\":\"{}\",\"amount_yocto\":\"{}\"}}",
        validator_pool,
        amount_yocto
    );
}
