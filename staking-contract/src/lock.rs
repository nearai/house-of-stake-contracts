use crate::internal::{
    check_near_price_lock, effective_stake_for_share_exit, mint_shares, near_from_shares,
};
use crate::*;
use common::U256;
use near_sdk::json_types::{U64, U128};
use near_sdk::{NearToken, assert_one_yocto, env, near, require};

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
    ///
    /// Provide **exactly one** of **`price_id`** or **`product_id`**:
    /// - **`price_id: Some`**, **`product_id: null`** — lock using that catalog price (same as always).
    /// - **`price_id: null`**, **`product_id: Some`** — lock using [`Product::default_price_id`](crate::types::Product::default_price_id) for that product (`set_product_default_price`).
    #[payable]
    pub fn lock_for_product(
        &mut self,
        price_id: Option<PriceId>,
        lock_duration_ns: U64,
        product_id: Option<ProductId>,
    ) -> LockId {
        let resolved = self.resolve_price_id_for_catalog_lock(price_id, product_id);
        self.lock_for_product_with_price_id(resolved, lock_duration_ns)
    }

    fn lock_for_product_with_price_id(
        &mut self,
        price_id: PriceId,
        lock_duration_ns: U64,
    ) -> LockId {
        self.assert_not_paused();

        let buyer = env::predecessor_account_id();
        self.ensure_min_storage_for_new_lock(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let dur = lock_duration_ns.0;
        require!(
            dur >= self.config.min_lock_duration_ns.0 && dur <= self.config.max_lock_duration_ns.0,
            "Lock duration is outside the allowed range for this contract"
        );

        let price_opt = self.prices.get(&price_id).cloned();
        require!(price_opt.is_some(), "Price not found in the catalog");
        let price = price_opt.unwrap();
        let product_opt = self.products.get(&price.product_id).cloned();
        require!(product_opt.is_some(), "Product not found in the catalog");
        let product = product_opt.unwrap();
        require!(
            price.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        require!(
            product.status == CatalogStatus::Active,
            "This product is not active; pick an active product"
        );
        require!(
            price.price_type == PriceType::OneOff,
            "Recurring prices: use lock_for_subscription with price_id or product_id"
        );
        require!(
            price.billing_period.is_none(),
            "One-off price must not set billing_period"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let dur_u128 = u128::from(dur);
        check_near_price_lock(&price, locked.as_yoctonear(), dur_u128)
            .unwrap_or_else(|e| env::panic_str(e));

        self.finalize_product_lock(buyer, price, product, locked, dur_u128)
    }

    /// Lock NEAR for a **monthly recurring** catalog price (NEAR-denominated). One subscription row per
    /// `(account, product_id)`; [`Subscription::price_id`] is the active tier.
    ///
    /// Provide **exactly one** of **`price_id`** or **`product_id`** (same rules as [`Contract::lock_for_product`]).
    #[payable]
    pub fn lock_for_subscription(
        &mut self,
        price_id: Option<PriceId>,
        product_id: Option<ProductId>,
    ) -> LockId {
        let resolved = self.resolve_price_id_for_catalog_lock(price_id, product_id);
        self.lock_for_subscription_with_price_id(resolved)
    }

    fn lock_for_subscription_with_price_id(&mut self, price_id: PriceId) -> LockId {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        self.ensure_min_storage_for_new_lock(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let price_opt = self.prices.get(&price_id).cloned();
        require!(price_opt.is_some(), "Price not found in the catalog");
        let price = price_opt.unwrap();
        require!(
            price.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        require!(
            price.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );
        require!(
            price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );

        let product_opt = self.products.get(&price.product_id).cloned();
        require!(product_opt.is_some(), "Product not found in the catalog");
        let product = product_opt.unwrap();
        require!(
            product.status == CatalogStatus::Active,
            "This product is not active; pick an active product"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let product_id = product.product_id.clone();
        let sub_key = (buyer.clone(), product_id.clone());
        let now = env::block_timestamp();

        let (mut subscription, sub_id, is_new_index) = if let Some(sid_ref) =
            self.subscription_by_account_product.get(&sub_key)
        {
            let sid = sid_ref.clone();
            let mut sub = self
                .subscriptions
                .get(sid_ref.as_str())
                .cloned()
                .unwrap_or_else(|| env::panic_str("Subscription not found"));
            require!(
                sub.account_id == buyer,
                "Only the subscription owner can perform this action"
            );
            if now < sub.end_ns.0 {
                if let Some(prev) = self.locks.get(&sub.last_lock_id) {
                    require!(
                        prev.status != LockStatus::Active,
                        "This subscription period already has an active lock"
                    );
                }
                (sub, sid, false)
            } else if sub.cancel_at_period_end {
                // Period has ended with cancel-at-end: remove stale index + row so this call creates a
                // fresh subscription row (same path as first-time subscribe).
                self.subscription_by_account_product.remove(&sub_key);
                self.subscriptions.remove(sid.as_str());
                let anchor = anchor_day_from_timestamp(now);
                let end = crate::subscriptions::add_months_stripe_style(anchor, 1, now);
                let sid_new = crate::ids::next_subscription_id(&mut self.id_nonce);
                let sub_new = Subscription {
                    subscription_id: sid_new.clone(),
                    account_id: buyer.clone(),
                    product_id: product.product_id.clone(),
                    price_id: price_id.clone(),
                    start_ns: U64(now),
                    end_ns: U64(end),
                    anchor_day: anchor,
                    last_lock_id: String::new(),
                    status: SubscriptionStatus::Active,
                    cancel_at_period_end: false,
                    pending_downgrade_price_id: None,
                };
                (sub_new, sid_new, true)
            } else {
                // Renewal window: scheduled downgrade / extend billing period.
                if let Some(low_id) = sub.pending_downgrade_price_id.take() {
                    let high_price = self.prices.get(&sub.price_id).cloned().unwrap_or_else(|| {
                        env::panic_str("High tier price not found in the catalog")
                    });
                    let low_price = self.prices.get(&low_id).cloned().unwrap_or_else(|| {
                        env::panic_str("Low tier price not found in the catalog")
                    });
                    let completed_ns = u128::from(sub.end_ns.0.saturating_sub(sub.start_ns.0));
                    self.apply_downgrade_prorate_at_renewal(
                        &buyer,
                        &sub,
                        &high_price,
                        &low_price,
                        completed_ns,
                    );
                    sub.price_id = low_id;
                }
                let start = sub.end_ns.0.max(now);
                let end = crate::subscriptions::add_months_stripe_style(sub.anchor_day, 1, start);
                sub.start_ns = U64(start);
                sub.end_ns = U64(end);
                sub.status = SubscriptionStatus::Active;
                (sub, sid, false)
            }
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
                cancel_at_period_end: false,
                pending_downgrade_price_id: None,
            };
            (sub, sid, true)
        };

        if !is_new_index {
            require!(
                price_id == subscription.price_id,
                "price_id must match current subscription tier"
            );
        }

        require!(
            subscription.end_ns.0 > now,
            "Subscription billing period has already ended"
        );
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
            self.subscription_by_account_product.insert(sub_key, sub_id);
        }

        lock_id
    }

    /// Stop renewing after the current billing period. The active lock remains until `lock.end_ns`; use
    /// [`Contract::unlock`] afterwards. Attach 1 yocto.
    #[payable]
    pub fn cancel_subscription(&mut self, product_id: ProductId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let sid = self
            .subscription_by_account_product
            .get(&(buyer.clone(), product_id.clone()))
            .cloned()
            .unwrap_or_else(|| env::panic_str("No subscription for this product; subscribe first"));
        let mut sub = self
            .subscriptions
            .get(sid.as_str())
            .cloned()
            .unwrap_or_else(|| env::panic_str("Subscription not found"));
        require!(
            sub.account_id == buyer,
            "Only the subscription owner can perform this action"
        );
        require!(
            sub.status == SubscriptionStatus::Active,
            "This subscription is not active (cancelled, expired, or not yet started)"
        );
        sub.cancel_at_period_end = true;
        self.subscriptions.insert(sid.clone(), sub.clone());
        crate::events::log_subscription_cancel(&buyer, &product_id);
    }

    /// Undo [`Contract::cancel_subscription`] before the current billing period ends: clear `cancel_at_period_end`
    /// so renewals resume normally after `end_ns`. Attach 1 yocto.
    #[payable]
    pub fn resume_subscription(&mut self, product_id: ProductId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let sid = self
            .subscription_by_account_product
            .get(&(buyer.clone(), product_id.clone()))
            .cloned()
            .unwrap_or_else(|| env::panic_str("No subscription for this product; subscribe first"));
        let mut sub = self
            .subscriptions
            .get(sid.as_str())
            .cloned()
            .unwrap_or_else(|| env::panic_str("Subscription not found"));
        require!(
            sub.account_id == buyer,
            "Only the subscription owner can perform this action"
        );
        require!(
            sub.status == SubscriptionStatus::Active,
            "This subscription is not active (cancelled, expired, or not yet started)"
        );
        require!(
            sub.cancel_at_period_end,
            "Subscription is not scheduled to cancel at period end"
        );
        sub.cancel_at_period_end = false;
        self.subscriptions.insert(sid.clone(), sub.clone());
        crate::events::log_subscription_resume(&buyer, &product_id);
    }

    /// Upgrade to a higher-priced recurring tier on the same product immediately. Attach extra NEAR so that
    /// `existing_locked + deposit` satisfies [`check_near_price_lock`] for the new tier over the remainder of
    /// the current period (`lock.end_ns - now`).
    #[payable]
    pub fn upgrade_subscription(&mut self, new_price_id: PriceId) -> LockId {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();

        let deposit = env::attached_deposit();
        require!(
            deposit.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let new_price = self
            .prices
            .get(&new_price_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Price not found in the catalog"));
        require!(
            new_price.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        require!(
            new_price.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );
        require!(
            new_price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );

        let product = self
            .products
            .get(&new_price.product_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Product not found in the catalog"));
        require!(
            product.status == CatalogStatus::Active,
            "This product is not active; pick an active product"
        );

        let sid = self
            .subscription_by_account_product
            .get(&(buyer.clone(), new_price.product_id.clone()))
            .cloned()
            .unwrap_or_else(|| env::panic_str("No subscription for this product; subscribe first"));
        let mut sub = self
            .subscriptions
            .get(sid.as_str())
            .cloned()
            .unwrap_or_else(|| env::panic_str("Subscription not found"));
        require!(
            sub.account_id == buyer,
            "Only the subscription owner can perform this action"
        );

        let old_price = self.prices.get(&sub.price_id).cloned().unwrap_or_else(|| {
            env::panic_str("Current subscription price not found in the catalog")
        });
        require!(
            new_price.product_id == sub.product_id,
            "Price must belong to this subscription product"
        );
        require!(
            new_price.amount.0 > old_price.amount.0,
            "New tier must have a higher catalog amount than current tier"
        );

        let mut lock = self
            .locks
            .get(&sub.last_lock_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("No lock is linked to this subscription"));
        require!(
            lock.account_id == buyer,
            "Only the lock owner can change this subscription lock"
        );
        require!(lock.status == LockStatus::Active, "Lock is not active");

        let now = env::block_timestamp();
        require!(
            now < lock.end_ns.0,
            "Current period already ended; renew instead"
        );

        let rem_ns = u128::from(lock.end_ns.0.saturating_sub(now));
        let total_locked = lock
            .amount_near
            .as_yoctonear()
            .saturating_add(deposit.as_yoctonear());
        check_near_price_lock(&new_price, total_locked, rem_ns)
            .unwrap_or_else(|e| env::panic_str(e));

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let mut v = self
            .validators
            .get(&validator_id)
            .cloned()
            .expect("Validator not found on the allowlist");

        let eff = effective_stake_for_share_exit(
            v.total_staked_balance,
            v.pending_to_stake,
            v.pending_user_unstake_total,
        );
        let ts = v.total_shares.0;
        if ts > 0 {
            require!(
                eff > 0,
                "No effective stake for share minting; wait for balance refresh or settlement"
            );
        }
        let add_shares = mint_shares(ts, eff, deposit.as_yoctonear());

        v.total_shares = U128(ts.saturating_add(add_shares));
        v.pending_to_stake = v
            .pending_to_stake
            .checked_add(deposit)
            .expect("pending_to_stake overflow when recording this lock");

        let ukey = (buyer.clone(), validator_id.clone());
        let prev_u = self.user_validator_shares.get(&ukey).copied().unwrap_or(0);
        self.user_validator_shares
            .insert(ukey, prev_u.saturating_add(add_shares));

        lock.amount_near = lock
            .amount_near
            .checked_add(deposit)
            .expect("lock amount_near overflow");
        lock.shares = U128(lock.shares.0.saturating_add(add_shares));
        lock.order = OrderRef::Subscription {
            subscription_id: sub.subscription_id.clone(),
            price_id: new_price_id.clone(),
            period_start_ns: sub.start_ns,
            period_end_ns: sub.end_ns,
        };

        sub.price_id = new_price_id.clone();

        let lock_id_out = lock.lock_id.clone();
        self.validators.insert(validator_id.clone(), v);
        self.locks.insert(lock_id_out.clone(), lock);
        self.subscriptions.insert(sid, sub);

        crate::events::log_subscription_upgrade(&buyer, &new_price_id);
        crate::events::log_lock(lock_id_out.as_str(), &buyer);

        lock_id_out
    }

    /// Schedule a lower tier for the **next** billing period (Phase A: applied at renewal; no automatic refund).
    #[payable]
    pub fn schedule_downgrade_subscription(&mut self, target_price_id: PriceId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();

        let target = self
            .prices
            .get(&target_price_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Price not found in the catalog"));
        require!(
            target.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        require!(
            target.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );

        let sid = self
            .subscription_by_account_product
            .get(&(buyer.clone(), target.product_id.clone()))
            .cloned()
            .unwrap_or_else(|| env::panic_str("No subscription for this product; subscribe first"));
        let mut sub = self
            .subscriptions
            .get(sid.as_str())
            .cloned()
            .unwrap_or_else(|| env::panic_str("Subscription not found"));
        require!(
            sub.account_id == buyer,
            "Only the subscription owner can perform this action"
        );
        require!(
            target.product_id == sub.product_id,
            "Price must belong to this subscription product"
        );

        let current = self.prices.get(&sub.price_id).cloned().unwrap_or_else(|| {
            env::panic_str("Current subscription price not found in the catalog")
        });
        require!(
            target.amount.0 < current.amount.0,
            "Target tier must have a lower catalog amount than current tier"
        );

        sub.pending_downgrade_price_id = Some(target_price_id.clone());
        self.subscriptions.insert(sid, sub.clone());

        crate::events::log_subscription_downgrade_scheduled(&buyer, &target_price_id);
    }

    pub fn get_lock(&self, lock_id: LockId) -> Option<Lock> {
        self.locks.get(&lock_id).cloned()
    }

    pub fn get_subscription(&self, subscription_id: SubscriptionId) -> Option<Subscription> {
        self.subscriptions.get(subscription_id.as_str()).cloned()
    }

    /// Lookup subscription by account and catalog product (one row per product).
    pub fn get_subscription_for_product(
        &self,
        account_id: AccountId,
        product_id: ProductId,
    ) -> Option<Subscription> {
        let sid = self
            .subscription_by_account_product
            .get(&(account_id, product_id.clone()))?
            .clone();
        self.subscriptions.get(sid.as_str()).cloned()
    }

    pub fn get_subscription_for_price(
        &self,
        account_id: AccountId,
        price_id: PriceId,
    ) -> Option<Subscription> {
        let price = self.prices.get(&price_id)?;
        self.get_subscription_for_product(account_id, price.product_id.clone())
    }
}

impl Contract {
    fn resolve_price_id_for_catalog_lock(
        &self,
        price_id: Option<PriceId>,
        product_id: Option<ProductId>,
    ) -> PriceId {
        match (price_id, product_id) {
            (Some(pid), None) => pid,
            (None, Some(prod_id)) => {
                let pr_id = self
                    .products
                    .get(&prod_id)
                    .and_then(|p| p.default_price_id.clone())
                    .unwrap_or_else(|| env::panic_str("No default price for this product"));
                let pr = self
                    .prices
                    .get(&pr_id)
                    .cloned()
                    .unwrap_or_else(|| env::panic_str("Price not found in the catalog"));
                require!(
                    pr.product_id == prod_id,
                    "Default price does not belong to this product"
                );
                pr_id
            }
            (Some(_), Some(_)) => env::panic_str("Provide only one of price_id or product_id"),
            (None, None) => env::panic_str("Provide price_id or product_id"),
        }
    }

    /// Phase B: at scheduled downgrade renewal, release catalog **tier-gap** stake (min high − min low for
    /// the completed period) as shares → same unstake queue as [`crate::unlock::Contract::unlock`].
    fn apply_downgrade_prorate_at_renewal(
        &mut self,
        buyer: &AccountId,
        sub: &Subscription,
        high_price: &Price,
        low_price: &Price,
        completed_period_ns: u128,
    ) {
        if completed_period_ns == 0 {
            return;
        }
        let min_h = crate::internal::min_locked_yocto_for_duration(high_price, completed_period_ns);
        let min_l = crate::internal::min_locked_yocto_for_duration(low_price, completed_period_ns);
        let surplus_target = min_h.saturating_sub(min_l);
        if surplus_target == 0 {
            return;
        }

        let mut lock = match self.locks.get(&sub.last_lock_id).cloned() {
            Some(l) => l,
            None => return,
        };
        if &lock.account_id != buyer || lock.status != LockStatus::Active {
            return;
        }

        let validator_id = lock.validator_id.clone();
        let v = self
            .validators
            .get(&validator_id)
            .cloned()
            .expect("Validator not found on the allowlist");
        let eff = effective_stake_for_share_exit(
            v.total_staked_balance,
            v.pending_to_stake,
            v.pending_user_unstake_total,
        );
        let ts = v.total_shares.0;
        let lock_near_val = near_from_shares(lock.shares.0, eff, ts);
        if lock_near_val == 0 {
            return;
        }

        let surplus_near = surplus_target.min(lock_near_val);
        let shares_remove = (U256::from(lock.shares.0) * U256::from(surplus_near)
            / U256::from(lock_near_val))
        .as_u128();
        let shares_remove = shares_remove.min(lock.shares.0);
        if shares_remove == 0 {
            return;
        }

        let near_amt = self.queue_shares_unstake(buyer.clone(), validator_id, shares_remove);
        lock.shares = U128(lock.shares.0.saturating_sub(shares_remove));
        let new_amt = lock.amount_near.as_yoctonear().saturating_sub(near_amt);
        lock.amount_near = NearToken::from_yoctonear(new_amt);
        if lock.shares.0 == 0 {
            lock.status = LockStatus::UnlockRequested;
        }
        self.locks.insert(lock.lock_id.clone(), lock);

        crate::events::log_subscription_downgrade_prorate(buyer, &sub.product_id, near_amt);
    }

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
            .expect("Validator not found on the allowlist");

        let eff = effective_stake_for_share_exit(
            v.total_staked_balance,
            v.pending_to_stake,
            v.pending_user_unstake_total,
        );
        let ts = v.total_shares.0;
        if ts > 0 {
            require!(
                eff > 0,
                "No effective stake for share minting; wait for balance refresh or settlement"
            );
        }
        let new_shares = mint_shares(ts, eff, locked.as_yoctonear());

        v.total_shares = U128(ts.saturating_add(new_shares));
        v.pending_to_stake = v
            .pending_to_stake
            .checked_add(locked)
            .expect("pending_to_stake overflow when recording this lock");

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
