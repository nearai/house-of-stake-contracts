use crate::internal::{check_near_price_lock, effective_stake_yocto, mint_shares};
use crate::*;
use near_sdk::json_types::{U128, U64};
use near_sdk::{env, near, require, NearToken};

#[near]
impl Contract {
    /// Lock NEAR for a one-off product purchase. Attach the NEAR to lock.
    /// **v1:** [`Currency::Usd`] prices require a follow-up `lock_for_product_usd` (oracle XCC) — not implemented yet; calling with USD returns an error.
    #[payable]
    pub fn lock_for_product(&mut self, price_id: PriceId, lock_duration_ns: U64) -> LockId {
        self.assert_not_paused();

        let buyer = env::predecessor_account_id();
        self.ensure_min_storage(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached deposit below min_lock_amount"
        );

        let dur = lock_duration_ns.0;
        require!(
            dur >= self.config.min_lock_duration_ns.0
                && dur <= self.config.max_lock_duration_ns.0,
            "lock_duration_ns out of bounds"
        );

        let price = self.prices.get(&price_id).cloned().expect("Unknown price");
        let product = self
            .products
            .get(&price.product_id)
            .cloned()
            .expect("Unknown product");
        require!(price.status == CatalogStatus::Active, "Price not active");
        require!(
            product.status == CatalogStatus::Active,
            "Product not active"
        );
        require!(
            price.currency == Currency::Near,
            "USD-priced locks: use lock_for_product_usd (TODO oracle callback)"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let dur_u128 = u128::from(dur);
        check_near_price_lock(&price, locked.as_yoctonear(), dur_u128).expect("price check");

        self.finalize_product_lock(buyer, price_id, price, product, locked, dur_u128)
    }

    pub fn get_lock(&self, lock_id: LockId) -> Option<Lock> {
        self.locks.get(&lock_id).cloned()
    }
}

impl Contract {
    pub(crate) fn finalize_product_lock(
        &mut self,
        buyer: AccountId,
        price_id: PriceId,
        mut price: Price,
        mut product: Product,
        locked: NearToken,
        duration_ns: u128,
    ) -> LockId {
        let validator_id = product.validator_id.clone();
        let mut v = self
            .validators
            .get(&validator_id)
            .cloned()
            .expect("validator");

        let eff = effective_stake_yocto(v.total_staked_balance, v.pending_to_stake);
        let ts = v.total_shares.0;
        let new_shares = mint_shares(ts, eff, locked.as_yoctonear());

        v.total_shares = U128(ts.saturating_add(new_shares));
        v.pending_to_stake = v
            .pending_to_stake
            .checked_add(locked)
            .expect("pending stake");

        let key = (buyer.clone(), validator_id.clone());
        let prev = self
            .user_validator_shares
            .get(&key)
            .copied()
            .unwrap_or(0);
        self.user_validator_shares
            .insert(key, prev.saturating_add(new_shares));

        let lock_id = crate::ids::next_lock_id(&mut self.id_nonce);
        let lock = Lock {
            lock_id: lock_id.clone(),
            account_id: buyer.clone(),
            validator_id: validator_id.clone(),
            amount_near: locked,
            shares: U128(new_shares),
            start_ns: U64(env::block_timestamp()),
            end_ns: U64(env::block_timestamp().saturating_add(
                u64::try_from(duration_ns).unwrap_or(u64::MAX),
            )),
            order: OrderRef::ProductPurchase {
                product_id: product.product_id.clone(),
                price_id: price_id.clone(),
            },
            status: LockStatus::Active,
        };
        self.locks.insert(lock_id.clone(), lock);

        price.usage_count = price.usage_count.saturating_add(1);
        product.usage_count = product.usage_count.saturating_add(1);
        self.prices.insert(price.price_id.clone(), price);
        self.products.insert(product.product_id.clone(), product);
        self.validators.insert(validator_id, v);

        lock_id
    }
}
