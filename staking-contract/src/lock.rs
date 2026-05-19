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
        let resolved = self.resolve_price_id_for_lock(price_id, product_id);
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

        let (price, product) = self.get_active_price_and_product(&price_id);
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
        self.assert_validator_idle_for_user_action(&validator);
        // WASM production: [`Contract::promise_validator_per_epoch_settlement_then`] then mint (`epoch.rs`).
        // Host targets (`tests/*.rs`, `cargo check` on the host triple): `near_sdk::testing_env!` does not run
        // returned promise chains—use synchronous commit (`finalize_lock` → `commit_catalog_lock`).
        // The library is built **without** `cfg(test)` for integration tests, so this split uses `target_arch`
        // (not `cfg(test)`): WASM builds always use the real promise path.
        #[cfg(not(target_arch = "wasm32"))]
        {
            return PromiseOrValue::Value(
                self.finalize_lock(buyer, price, product, locked, dur_u128, order),
            );
        }
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .promise_validator_per_epoch_settlement_then(
                    validator_id.clone(),
                    PerEpochContinue::CatalogLockMint {
                        validator_id,
                        buyer,
                        locked,
                        duration_ns: dur_u128,
                        order,
                        subscription_followup: None,
                    },
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
        let resolved = self.resolve_price_id_for_lock(price_id, product_id);
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

        let (price, product) = self.get_active_price_and_product(&price_id);
        self.require_recurring_monthly_price(&price);

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let product_id = product.product_id.clone();
        let sub_key = (buyer.clone(), product_id.clone());
        let now = env::block_timestamp();

        let (subscription, sub_id, is_new_index) = if let Some(sid_ref) =
            self.subscription_by_account_product.get(&sub_key)
        {
            let sid = sid_ref.clone();
            let mut sub = self.require_subscription_row(&sid);
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
        self.assert_validator_idle_for_user_action(&validator);
        // Same host synchronous path as `lock_for_product_with_price_id` (see comment there).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut subscription = subscription;
            let lock_id = self.finalize_lock(
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
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .promise_validator_per_epoch_settlement_then(
                    validator_id.clone(),
                    PerEpochContinue::CatalogLockMint {
                        validator_id,
                        buyer,
                        locked,
                        duration_ns,
                        order,
                        subscription_followup: Some((subscription, sub_id, is_new_index)),
                    },
                )
                .into();
        }
    }

    pub fn get_lock(&self, lock_id: LockId) -> Option<Lock> {
        self.locks.get(&lock_id).cloned()
    }
}

impl Contract {
    pub(crate) fn get_active_price_and_product(&self, price_id: &PriceId) -> (Price, Product) {
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

    pub(crate) fn require_recurring_monthly_price(&self, price: &Price) {
        require!(
            price.price_type == PriceType::Recurring,
            "This price is not a recurring subscription price"
        );
        require!(
            price.billing_period == Some(BillingPeriod::Monthly),
            "Only monthly billing is supported"
        );
    }

    pub(crate) fn require_active_recurring_monthly_price(&self, price_id: &PriceId) -> Price {
        let (price, _) = self.get_active_price_and_product(price_id);
        self.require_recurring_monthly_price(&price);
        price
    }

    /// Picks the catalog price id for a lock from caller input.
    ///
    /// Exactly one of `price_id` or `product_id` must be `Some`. If only `product_id` is given,
    /// the product's default catalog price is used; that price row must reference the same product.
    fn resolve_price_id_for_lock(
        &self,
        price_id: Option<PriceId>,
        product_id: Option<ProductId>,
    ) -> PriceId {
        match (price_id, product_id) {
            // Use the explicitly chosen price; active-catalog checks happen when the lock is applied.
            (Some(pid), None) => pid,
            (None, Some(prod_id)) => {
                // Resolve via the product's default_price_id, then sanity-check the price row.
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
            // Both `price_id` and `product_id` — caller must pick one.
            (Some(_), Some(_)) => env::panic_str("Provide only one of price_id or product_id"),
            // Neither identifier given.
            (None, None) => env::panic_str("Provide price_id or product_id"),
        }
    }

    /// Commits catalog-lock **state**: mints pool shares for `locked` NEAR, stores the lock row,
    /// bumps catalog usage, updates validator pending stake and per-buyer share balance, then
    /// optionally updates subscription storage (row + `(account, product)` index when new).
    ///
    /// **Inputs:** Re-reads active catalog price/product from `order` and requires
    /// `product.validator_id == validator_id` so the mint matches the pool used on the lock path.
    ///
    /// **When to call:** Stake figures passed into [`mint_shares`] must match pool reality. Production
    /// invokes this from [`crate::epoch::Contract::on_lock_finally_mint_and_maybe_post_settle`] after
    /// the shared per-epoch pre-user settlement pipeline (**0–3**) on the lock promise chain.
    pub(crate) fn commit_catalog_lock(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> LockId {
        // Catalog line item for this lock (one-off or subscription).
        let price_id = match &order {
            OrderRef::ProductPurchase { price_id, .. }
            | OrderRef::Subscription { price_id, .. } => price_id.clone(),
        };
        let (mut price, mut product) = self.get_active_price_and_product(&price_id);
        require!(
            product.validator_id == validator_id,
            "Catalog validator for this price does not match the pool used for this lock"
        );

        let mut validator = self.require_validator(&validator_id);

        // Share price uses effective pool stake; skip the strict check when this is the first mint
        // (`total_shares == 0`) so the first tranche can mint off `locked` alone.
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

        // Validator pool aggregates: new shares and NEAR queued for the next `deposit_and_stake`.
        validator.total_shares = U128(validator_total_shares.saturating_add(new_shares));
        validator.pending_to_stake = validator
            .pending_to_stake
            .checked_add(locked)
            .expect("pending_to_stake overflow when recording this lock");

        // This buyer's tranche on this pool (integer share units, same scale as `Validator::total_shares`).
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

        // Persist the lock (duration → `end_ns`); `order` ties billing/catalog to this stake.
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

        // Catalog analytics + write-through updated rows and validator map.
        price.usage_count = price.usage_count.saturating_add(1);
        product.usage_count = product.usage_count.saturating_add(1);

        self.prices.insert(price.price_id.clone(), price);
        self.products.insert(product.product_id.clone(), product);
        self.validators.insert(validator_id, validator);

        // Drives prepaid lock storage (`per_lock_storage_stake` × count) for this account.
        let user_lock_count_before = self.user_lock_count.get(&buyer).copied().unwrap_or(0);
        self.user_lock_count
            .insert(buyer.clone(), user_lock_count_before.saturating_add(1));

        // Subscription path: caller already built/updated `Subscription`; we only persist + optional index.
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

    #[cfg(not(target_arch = "wasm32"))]
    /// Host-only: skips pool balance refresh and stake promises (see module comment on `lock_for_product`).
    pub(crate) fn finalize_lock(
        &mut self,
        buyer: AccountId,
        _price: Price,
        product: Product,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
    ) -> LockId {
        self.commit_catalog_lock(
            buyer,
            locked,
            duration_ns,
            order,
            product.validator_id.clone(),
            None,
        )
    }
}

// =============================================================================
// Epoch pipeline: catalog mint tail (callback from `epoch::on_epoch_settlement_dispatch_continue`)
// =============================================================================

#[near]
impl Contract {
    #[private]
    /// **[Pipeline 5a]** Catalog mint after **4**. Pre-user settlement (**0–3**) already ran before
    /// mint; this lock's `pending_to_stake` is queued for a later `unlock` / `withdraw` / `epoch_settle`.
    pub fn on_lock_finally_mint_and_maybe_post_settle(
        &mut self,
        buyer: AccountId,
        locked: NearToken,
        duration_ns: u128,
        order: OrderRef,
        validator_id: ValidatorId,
        subscription_followup: Option<(Subscription, SubscriptionId, bool)>,
    ) -> PromiseOrValue<()> {
        let _lock_id = self.commit_catalog_lock(
            buyer,
            locked,
            duration_ns,
            order,
            validator_id.clone(),
            subscription_followup,
        );
        let validator = self.require_validator(&validator_id);
        require!(
            validator.tx_status == TransactionStatus::Busy,
            "Validator pool must be busy after per-epoch settlement"
        );
        PromiseOrValue::Value(())
    }
}
