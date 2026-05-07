use crate::internal::{check_near_price_lock, effective_stake_yocto, mint_shares};
use crate::*;
use near_sdk::json_types::{U64, U128};
use near_sdk::{NearToken, env, near, require};

const NS_PER_DAY: u64 = 86_400_000_000_000;

/// Stripe-style **billing anchor day** (1–31). Not the real UTC calendar day-of-month; it is a stable
/// fingerprint from block time until civil-calendar billing is implemented (see `subscriptions` / `docs/ACTION_ITEMS.md`).
fn anchor_day_from_timestamp(ts: u64) -> u8 {
    let d = (ts / NS_PER_DAY) % 31;
    (d as u8 + 1).min(31)
}

#[near]
impl Contract {
    /// Lock NEAR for a one-off product purchase. Attach the NEAR to lock.
    #[payable]
    pub fn lock_for_product(&mut self, price_id: PriceId, lock_duration_ns: U64) -> LockId {
        self.assert_not_paused();

        let buyer = env::predecessor_account_id();
        self.ensure_min_storage_for_new_lock(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached deposit below min_lock_amount"
        );

        let dur = lock_duration_ns.0;
        require!(
            dur >= self.config.min_lock_duration_ns.0 && dur <= self.config.max_lock_duration_ns.0,
            "lock_duration_ns out of bounds"
        );

        let price_opt = self.prices.get(&price_id).cloned();
        require!(price_opt.is_some(), "Unknown price");
        let price = price_opt.unwrap();
        let product_opt = self.products.get(&price.product_id).cloned();
        require!(product_opt.is_some(), "Unknown product");
        let product = product_opt.unwrap();
        require!(price.status == CatalogStatus::Active, "Price not active");
        require!(
            product.status == CatalogStatus::Active,
            "Product not active"
        );
        require!(
            price.price_type == PriceType::OneOff,
            "Recurring prices: use lock_for_subscription"
        );
        require!(
            price.billing_period.is_none(),
            "One-off price must not set billing_period"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let dur_u128 = u128::from(dur);
        check_near_price_lock(&price, locked.as_yoctonear(), dur_u128).expect("price check");

        self.finalize_product_lock(buyer, price, product, locked, dur_u128)
    }

    /// Lock NEAR for a **monthly recurring** catalog price (NEAR-denominated). One subscription row per
    /// `(account, price_id)`; renews the billing window when the previous period has ended.
    #[payable]
    pub fn lock_for_subscription(&mut self, price_id: PriceId) -> LockId {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        self.ensure_min_storage_for_new_lock(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached deposit below min_lock_amount"
        );

        let price_opt = self.prices.get(&price_id).cloned();
        require!(price_opt.is_some(), "Unknown price");
        let price = price_opt.unwrap();
        require!(price.status == CatalogStatus::Active, "Price not active");
        require!(
            price.price_type == PriceType::Recurring,
            "Not a subscription price"
        );
        require!(
            price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );

        let product_opt = self.products.get(&price.product_id).cloned();
        require!(product_opt.is_some(), "Unknown product");
        let product = product_opt.unwrap();
        require!(
            product.status == CatalogStatus::Active,
            "Product not active"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let sub_key = (buyer.clone(), price_id.clone());
        let now = env::block_timestamp();

        let (mut subscription, sub_id, is_new_index) = if let Some(sid_ref) =
            self.subscription_by_account_price.get(&sub_key)
        {
            let sid = sid_ref.clone();
            let mut sub = self
                .subscriptions
                .get(sid_ref.as_str())
                .cloned()
                .unwrap_or_else(|| env::panic_str("Unknown subscription"));
            require!(sub.account_id == buyer, "Subscription account mismatch");
            if now < sub.end_ns.0 {
                if let Some(prev) = self.locks.get(&sub.last_lock_id) {
                    require!(
                        prev.status != LockStatus::Active,
                        "This subscription period already has an active lock"
                    );
                }
            } else {
                // Late renewal: start the new period at `now` if the previous `end_ns` is already in the past.
                let start = sub.end_ns.0.max(now);
                let end = crate::subscriptions::add_months_stripe_style(sub.anchor_day, 1, start);
                sub.start_ns = U64(start);
                sub.end_ns = U64(end);
                sub.status = SubscriptionStatus::Active;
            }
            (sub, sid, false)
        } else {
            let anchor = anchor_day_from_timestamp(now);
            let end = crate::subscriptions::add_months_stripe_style(anchor, 1, now);
            let sid = crate::ids::next_subscription_id(&mut self.id_nonce);
            let sub = Subscription {
                subscription_id: sid.clone(),
                account_id: buyer.clone(),
                product_id: product.product_id.clone(),
                price_id: price_id.clone(),
                start_ns: U64(now),
                end_ns: U64(end),
                anchor_day: anchor,
                last_lock_id: String::new(),
                status: SubscriptionStatus::Active,
            };
            (sub, sid, true)
        };

        require!(subscription.end_ns.0 > now, "Invalid subscription period");
        let duration_ns = u128::from(subscription.end_ns.0.saturating_sub(now));
        require!(duration_ns > 0, "Lock duration must be positive");

        check_near_price_lock(&price, locked.as_yoctonear(), duration_ns)
            .unwrap_or_else(|e| env::panic_str(e));

        let order = OrderRef::Subscription {
            subscription_id: sub_id.clone(),
            price_id: price_id.clone(),
            period_start_ns: subscription.start_ns,
            period_end_ns: subscription.end_ns,
        };

        let lock_id = self.finalize_lock_common(buyer, price, product, locked, duration_ns, order);

        subscription.last_lock_id = lock_id.clone();
        self.subscriptions.insert(sub_id.clone(), subscription);
        if is_new_index {
            self.subscription_by_account_price.insert(sub_key, sub_id);
        }

        lock_id
    }

    pub fn get_lock(&self, lock_id: LockId) -> Option<Lock> {
        self.locks.get(&lock_id).cloned()
    }

    pub fn get_subscription(&self, subscription_id: SubscriptionId) -> Option<Subscription> {
        self.subscriptions.get(subscription_id.as_str()).cloned()
    }

    pub fn get_subscription_for_price(
        &self,
        account_id: AccountId,
        price_id: PriceId,
    ) -> Option<Subscription> {
        let sid = self
            .subscription_by_account_price
            .get(&(account_id, price_id))?;
        self.subscriptions.get(sid.as_str()).cloned()
    }
}

impl Contract {
    pub(crate) fn finalize_product_lock(
        &mut self,
        buyer: AccountId,
        price: Price,
        product: Product,
        locked: NearToken,
        duration_ns: u128,
    ) -> LockId {
        let order = OrderRef::ProductPurchase {
            product_id: product.product_id.clone(),
            price_id: price.price_id.clone(),
        };
        self.finalize_lock_common(buyer, price, product, locked, duration_ns, order)
    }

    pub(crate) fn finalize_lock_common(
        &mut self,
        buyer: AccountId,
        mut price: Price,
        mut product: Product,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
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
        let prev = self.user_validator_shares.get(&key).copied().unwrap_or(0);
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
            end_ns: U64(env::block_timestamp()
                .saturating_add(u64::try_from(duration_ns).unwrap_or(u64::MAX))),
            order,
            status: LockStatus::Active,
        };
        self.locks.insert(lock_id.clone(), lock);

        price.usage_count = price.usage_count.saturating_add(1);
        product.usage_count = product.usage_count.saturating_add(1);
        self.prices.insert(price.price_id.clone(), price);
        self.products.insert(product.product_id.clone(), product);
        self.validators.insert(validator_id, v);

        let cnt = self.user_lock_count.get(&buyer).copied().unwrap_or(0);
        self.user_lock_count
            .insert(buyer.clone(), cnt.saturating_add(1));

        crate::events::log_lock(lock_id.as_str(), &buyer);

        lock_id
    }
}
