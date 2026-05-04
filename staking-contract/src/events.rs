//! EVENT_JSON logs for indexers (expand per DESIGN).

use near_sdk::log;

pub fn log_validator_added(pool: &near_sdk::AccountId) {
    log!("EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"validator_add\",\"pool\":\"{}\"}}", pool);
}

pub fn log_lock(lock_id: &str, account: &near_sdk::AccountId) {
    log!(
        "EVENT_JSON:{{\"standard\":\"stakedao\",\"event\":\"lock_create\",\"lock_id\":\"{}\",\"account\":\"{}\"}}",
        lock_id,
        account
    );
}
