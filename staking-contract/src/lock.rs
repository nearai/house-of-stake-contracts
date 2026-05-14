use crate::internal::{
    NS_PER_DAY_TIMESTAMP, check_near_price_lock, effective_stake_for_share_exit, mint_shares,
};
use crate::*;
use near_sdk::json_types::{U64, U128};
use near_sdk::{NearToken, PromiseOrValue, env, near, require};

/// Stripe-style **billing anchor day** (1–31). Not the real UTC calendar day-of-month; it is a stable
/// fingerprint from block time until civil-calendar billing is implemented (see `subscriptions` / `docs/ACTION_ITEMS.md`).
fn anchor_day_from_timestamp(ts: u64) -> u8 {
    let d = (ts / NS_PER_DAY_TIMESTAMP) % 31;
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
    ) -> PromiseOrValue<LockId> {
        let resolved = self.resolve_price_id_for_catalog_lock(price_id, product_id);
        self.lock_for_product_with_price_id(resolved, lock_duration_ns)
    }

    fn lock_for_product_with_price_id(
        &mut self,
        price_id: PriceId,
        lock_duration_ns: U64,
    ) -> PromiseOrValue<LockId> {
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

        let (price, product) = self.load_active_catalog_price_product(&price_id);
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

        let order = OrderRef::ProductPurchase {
            product_id: product.product_id.clone(),
            price_id: price.price_id.clone(),
        };
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        // Production: [`Contract::promise_validator_per_epoch_settlement_then`] then mint (see `epoch.rs`).
        // Host `tests/*.rs` use `near_sdk::testing_env!`, which does not run returned promise chains like
        // the real runtime; the `#[cfg(test)]` path runs the same mint/persist logic synchronously without
        // a pool round-trip (see `finalize_lock_common` → `apply_lock_mint_after_pool_balance_refresh`).
        #[cfg(test)]
        {
            return PromiseOrValue::Value(
                self.finalize_lock_common(buyer, price, product, locked, dur_u128, order),
            );
        }
        #[cfg(not(test))]
        {
            return self
                .promise_lock_refresh_then_finalize(
                    buyer,
                    locked,
                    dur_u128,
                    order,
                    validator_id,
                    None,
                )
                .into();
        }
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
    ) -> PromiseOrValue<LockId> {
        let resolved = self.resolve_price_id_for_catalog_lock(price_id, product_id);
        self.lock_for_subscription_with_price_id(resolved)
    }

    fn lock_for_subscription_with_price_id(&mut self, price_id: PriceId) -> PromiseOrValue<LockId> {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        self.ensure_min_storage_for_new_lock(&buyer);

        let locked = env::attached_deposit();
        require!(
            locked.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let (price, product) = self.load_active_catalog_price_product(&price_id);
        require!(
            price.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );
        require!(
            price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let product_id = product.product_id.clone();
        let sub_key = (buyer.clone(), product_id.clone());
        let now = env::block_timestamp();

        let (subscription, sub_id, is_new_index) = if let Some(sid_ref) =
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

        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Idle,
            "Validator pool is busy; wait for the in-flight pool call to finish"
        );
        // Same `#[cfg(test)]` rationale as `lock_for_product_with_price_id`: sync finish for host tests
        // without executing the pool refresh promise chain.
        #[cfg(test)]
        {
            let mut subscription = subscription;
            let lock_id = self.finalize_lock_common(
                buyer.clone(),
                price,
                product,
                locked,
                duration_ns,
                order.clone(),
            );
            subscription.last_lock_id = lock_id.clone();
            self.subscriptions.insert(sub_id.clone(), subscription);
            if is_new_index {
                self.subscription_by_account_product.insert(sub_key, sub_id);
            }
            return PromiseOrValue::Value(lock_id);
        }
        #[cfg(not(test))]
        {
            return self
                .promise_lock_refresh_then_finalize(
                    buyer,
                    locked,
                    duration_ns,
                    order,
                    validator_id,
                    Some((subscription, sub_id, is_new_index)),
                )
                .into();
        }
    }

    pub fn get_lock(&self, lock_id: LockId) -> Option<Lock> {
        self.locks.get(&lock_id).cloned()
    }
}

impl Contract {
    fn load_active_catalog_price_product(&self, price_id: &PriceId) -> (Price, Product) {
        let price = self
            .prices
            .get(price_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Price not found in the catalog"));
        require!(
            price.status == CatalogStatus::Active,
            "This price is not active; pick an active price"
        );
        let product = self
            .products
            .get(&price.product_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("Product not found in the catalog"));
        require!(
            product.status == CatalogStatus::Active,
            "This product is not active; pick an active product"
        );
        (price, product)
    }

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

    /// Mint shares, persist lock and catalog usage, and optional subscription index — after a successful
    /// pool `get_account_total_balance` refresh (production) or in unit tests without cross-contract calls.
    pub(crate) fn apply_lock_mint_after_pool_balance_refresh(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> LockId {
        let price_id = match &order {
            OrderRef::ProductPurchase { price_id, .. }
            | OrderRef::Subscription { price_id, .. } => price_id.clone(),
        };
        let (mut price, mut product) = self.load_active_catalog_price_product(&price_id);
        require!(
            product.validator_id == validator_id,
            "Catalog validator for this price does not match the pool used for this lock"
        );

        let mut validator = self.require_validator(&validator_id);

        if validator.total_shares.0 == 0 {
            require!(
                locked.as_yoctonear() >= crate::config::MIN_FIRST_VALIDATOR_DEPOSIT_NEAR_YOCTO,
                "The first stake on this validator must be at least 1 NEAR (protocol minimum)"
            );
        }

        let effective_stake_yocto = effective_stake_for_share_exit(
            validator.total_staked_balance,
            validator.pending_to_stake,
            validator.pending_user_unstake_total,
        );
        let validator_total_shares = validator.total_shares.0;
        if validator_total_shares > 0 {
            require!(
                effective_stake_yocto > 0,
                "No effective stake for share minting; wait for balance refresh or settlement"
            );
        }
        let new_shares = mint_shares(
            validator_total_shares,
            effective_stake_yocto,
            locked.as_yoctonear(),
        );

        validator.total_shares = U128(validator_total_shares.saturating_add(new_shares));
        validator.pending_to_stake = validator
            .pending_to_stake
            .checked_add(locked)
            .expect("pending_to_stake overflow when recording this lock");

        let user_validator_shares_key = (buyer.clone(), validator_id.clone());
        let user_shares_before_lock = self
            .user_validator_shares
            .get(&user_validator_shares_key)
            .copied()
            .unwrap_or(0);
        self.user_validator_shares.insert(
            user_validator_shares_key,
            user_shares_before_lock.saturating_add(new_shares),
        );

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
            order: order.clone(),
            status: LockStatus::Active,
        };
        self.locks.insert(lock_id.clone(), lock);

        price.usage_count = price.usage_count.saturating_add(1);
        product.usage_count = product.usage_count.saturating_add(1);
        self.prices.insert(price.price_id.clone(), price);
        self.products.insert(product.product_id.clone(), product);
        self.validators.insert(validator_id, validator);

        let user_lock_count_before = self.user_lock_count.get(&buyer).copied().unwrap_or(0);
        self.user_lock_count
            .insert(buyer.clone(), user_lock_count_before.saturating_add(1));

        if let Some((mut subscription, sub_id, is_new_index)) = subscription_followup {
            subscription.last_lock_id = lock_id.clone();
            let sub_key = (
                subscription.account_id.clone(),
                subscription.product_id.clone(),
            );
            self.subscriptions.insert(sub_id.clone(), subscription);
            if is_new_index {
                self.subscription_by_account_product.insert(sub_key, sub_id);
            }
        }

        crate::events::log_lock(lock_id.as_str(), &buyer);

        lock_id
    }

    #[cfg(test)]
    /// Used only by the `#[cfg(test)]` lock paths above: skips pool balance refresh and stake promises.
    pub(crate) fn finalize_lock_common(
        &mut self,
        buyer: AccountId,
        _price: Price,
        product: Product,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
    ) -> LockId {
        self.apply_lock_mint_after_pool_balance_refresh(
            buyer,
            locked,
            duration_ns,
            order,
            product.validator_id.clone(),
            None,
        )
    }
}
