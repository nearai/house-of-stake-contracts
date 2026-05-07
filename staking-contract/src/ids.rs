//! Stripe-like deterministic IDs: `prod_*`, `price_*`, `sub_*`, `lock_*`.

use near_sdk::env;

/// Prefix lengths per PLAN §11 item 6 (approximate Stripe parity).
pub const PROD_SUFFIX_LEN: usize = 14;
pub const PRICE_SUFFIX_LEN: usize = 24;
pub const SUB_LOCK_SUFFIX_LEN: usize = 17;

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn next_product_id(nonce: &mut u64) -> String {
    next_id("prod", PROD_SUFFIX_LEN, nonce)
}

pub fn next_price_id(nonce: &mut u64) -> String {
    next_id("price", PRICE_SUFFIX_LEN, nonce)
}

pub fn next_subscription_id(nonce: &mut u64) -> String {
    next_id("sub", SUB_LOCK_SUFFIX_LEN, nonce)
}

pub fn next_lock_id(nonce: &mut u64) -> String {
    next_id("lock", SUB_LOCK_SUFFIX_LEN, nonce)
}

/// Collision probability with existing IDs is negligible (SHA-256 → base62). Product/price creation retries
/// if a generated id already exists in storage (see `products` callbacks).
fn next_id(prefix: &str, suffix_len: usize, nonce: &mut u64) -> String {
    let n = *nonce;
    *nonce = nonce.saturating_add(1);

    let mut buf = Vec::new();
    buf.extend_from_slice(prefix.as_bytes());
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(&env::block_height().to_be_bytes());
    buf.extend_from_slice(&env::block_timestamp().to_be_bytes());
    buf.extend_from_slice(env::predecessor_account_id().as_bytes());
    let hash = env::sha256(&buf);
    let encoded = base62_from_hash(&hash, suffix_len);
    format!("{prefix}_{encoded}")
}

fn base62_from_hash(hash: &[u8], out_len: usize) -> String {
    // Re-hash if we need more entropy than 32 bytes for very long suffixes.
    let mut buf = hash.to_vec();
    let mut chars = Vec::with_capacity(out_len);
    let mut idx = 0usize;
    while chars.len() < out_len {
        if idx >= buf.len() {
            buf = env::sha256(&buf).to_vec();
            idx = 0;
        }
        chars.push(BASE62[(buf[idx] % 62) as usize] as char);
        idx += 1;
    }
    chars.into_iter().collect()
}
