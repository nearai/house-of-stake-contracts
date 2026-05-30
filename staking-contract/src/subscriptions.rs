//! Subscription billing helpers (Stripe-style linear months) and subscription **lifecycle** RPCs
//! (`cancel_subscription`, `update_subscription`, …). Subscription **locking** (`lock`)
//! stays in [`crate::lock`] because it shares the pool refresh / mint pipeline with product locks.

use crate::utils::{AVG_MONTH_NS, block_timestamp, check_near_price_lock, near_from_shares};
use crate::*;
use common::U256;
use near_sdk::json_types::{U64, U128};
use near_sdk::{AccountId, NearToken, PromiseOrValue, assert_one_yocto, env, near, require};

/// Extend `from_ns` by `months` × average Gregorian months (linear approximation).
/// `anchor_day` is validated but not yet applied; see `docs/ACTION_ITEMS.md`.
pub fn add_months_stripe_style(anchor_day: u8, months: u32, from_ns: u64) -> u64 {
    let _anchor_day = anchor_day.clamp(1, 31);
    let add_ns = (months as u128).saturating_mul(AVG_MONTH_NS);
    let add_u64 = u64::try_from(add_ns).unwrap_or(u64::MAX);
    from_ns.saturating_add(add_u64)
}

struct SubscriptionUpdateInputs {
    sub: Subscription,
    target_price: Price,
    target_product: Product,
    lock: Lock,
}

#[near]
impl Contract {
    /// Stop renewing after the current billing period. The active lock remains until `lock.end_ns`; use
    /// [`crate::unlock::Contract::unlock`] afterwards. Attach 1 yocto.
    #[payable]
    pub fn cancel_subscription(&mut self, product_id: ProductId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let (sid, _) = self.require_subscription_owned_by(&buyer, &product_id);
        self.apply_due_subscription_downgrade(&sid);
        let sub = self.require_subscription_owned_by_id(&buyer, &sid);
        // Normalize stale active windows before marking cancel-at-end so stored `end_ns`
        // represents the current virtual billing period boundary.
        let mut sub = self.project_subscription_view_now(sub);
        Self::assert_subscription_active(&sub);
        sub.cancel_at_period_end = true;
        self.internal_set_subscription(sid.clone(), sub.clone());
        crate::events::log_subscription_cancel(&buyer, &product_id);
    }

    /// Undo [`Contract::cancel_subscription`] before the current billing period ends: clear `cancel_at_period_end`
    /// so renewals resume normally after `end_ns`. Attach 1 yocto.
    #[payable]
    pub fn resume_subscription(&mut self, product_id: ProductId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let (sid, _) = self.require_subscription_owned_by(&buyer, &product_id);
        self.apply_due_subscription_downgrade(&sid);
        let mut sub = self.require_subscription_owned_by_id(&buyer, &sid);
        Self::assert_subscription_active(&sub);
        require!(
            sub.cancel_at_period_end,
            "Subscription is not scheduled to cancel at period end"
        );
        require!(
            block_timestamp() < sub.end_ns.0,
            "Current billing period has ended; subscribe again with lock instead"
        );
        sub.cancel_at_period_end = false;
        self.internal_set_subscription(sid.clone(), sub.clone());
        crate::events::log_subscription_resume(&buyer, &product_id);
    }

    /// Update a subscription to a target recurring tier and explicit target stake amount.
    ///
    /// Stake increases apply immediately after the shared validator settlement pipeline. Stake decreases
    /// are scheduled for the next billing period and applied by `lock` at renewal.
    #[payable]
    pub fn update_subscription(
        &mut self,
        subscription_id: SubscriptionId,
        target_price_id: PriceId,
        target_amount: U128,
    ) -> PromiseOrValue<SubscriptionPlanChangeOutcome> {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let deposit = env::attached_deposit();

        self.apply_due_subscription_downgrade(&subscription_id);
        let inputs = self.checked_subscription_update_inputs(
            &buyer,
            &target_price_id,
            target_amount,
            &subscription_id,
            None,
        );
        let target_amount_near = NearToken::from_yoctonear(target_amount.0);
        let current_amount = inputs.lock.amount_near.as_yoctonear();

        if target_amount.0 == current_amount && target_price_id == inputs.sub.price_id {
            assert_one_yocto();
            let mut sub = inputs.sub;
            sub.pending_downgrade_price_id = None;
            sub.pending_downgrade_target_amount = None;
            sub.pending_downgrade_apply_ns = None;
            self.internal_set_subscription(subscription_id.clone(), sub);
            return PromiseOrValue::Value(SubscriptionPlanChangeOutcome {
                kind: "no_op".to_string(),
                subscription_id,
                target_price_id,
                target_amount,
                lock_id: None,
            });
        }

        if target_amount.0 < current_amount {
            assert_one_yocto();
            self.schedule_subscription_decrease(
                buyer,
                subscription_id.clone(),
                inputs.sub,
                inputs.target_price,
                inputs.target_product,
                target_amount_near,
            );
            return PromiseOrValue::Value(SubscriptionPlanChangeOutcome {
                kind: "scheduled_for_period_end".to_string(),
                subscription_id,
                target_price_id,
                target_amount,
                lock_id: None,
            });
        }

        if target_amount.0 == current_amount {
            assert_one_yocto();
            return PromiseOrValue::Value(
                self.commit_subscription_plan_change_without_stake_delta(
                    buyer,
                    subscription_id,
                    target_price_id,
                    target_amount,
                    inputs.sub,
                    inputs.target_price,
                    inputs.target_product,
                    inputs.lock,
                ),
            );
        }

        let delta = target_amount.0.saturating_sub(current_amount);
        require!(
            deposit.as_yoctonear() == delta,
            "Attached NEAR must equal the target stake increase"
        );

        self.require_enough_gas_for_epoch_settlement();
        let validator_id = inputs.target_product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);
        let _validator = self.require_validator_idle(&validator_id);

        #[cfg(not(target_arch = "wasm32"))]
        {
            return PromiseOrValue::Value(self.commit_subscription_stake_increase(
                buyer,
                deposit,
                target_price_id,
                target_amount,
                subscription_id,
                validator_id,
            ));
        }
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .promise_validator_per_epoch_settlement_then(
                    validator_id.clone(),
                    UserAction::SubscriptionUpdate {
                        validator_id,
                        buyer,
                        deposit,
                        target_price_id,
                        target_amount,
                        subscription_id,
                    },
                )
                .into();
        }
    }

    // -------------------------------------------------------------------------
    // Public subscription view functions
    // -------------------------------------------------------------------------

    pub fn get_subscription(&self, subscription_id: SubscriptionId) -> Option<Subscription> {
        self.internal_get_subscription(&subscription_id)
            .map(|sub| self.project_subscription_view_now(sub))
    }

    /// Lookup subscription by account and catalog product (at most one subscription per product).
    pub fn get_subscription_for_product(
        &self,
        account_id: AccountId,
        product_id: ProductId,
    ) -> Option<Subscription> {
        if let Some(sid) = self
            .subscription_by_account_product
            .get(&(account_id.clone(), product_id.clone()))
            .cloned()
        {
            let projected = self
                .internal_get_subscription(&sid)
                .map(|sub| self.project_subscription_view_now(sub))?;
            return (projected.product_id == product_id).then_some(projected);
        }

        self.subscription_ids_for_account_view(&account_id)
            .into_iter()
            .filter_map(|sid| {
                self.internal_get_subscription(&sid)
                    .map(|sub| self.project_subscription_view_now(sub))
            })
            .find(|sub| sub.product_id == product_id)
    }

    pub fn get_subscription_for_price(
        &self,
        account_id: AccountId,
        price_id: PriceId,
    ) -> Option<Subscription> {
        let price = self.internal_get_price(&price_id)?;
        self.get_subscription_for_product(account_id, price.product_id.clone())
    }

    pub fn get_subscriptions_for_account(
        &self,
        account_id: AccountId,
        from_index: u64,
        limit: u64,
    ) -> Vec<Subscription> {
        let ids = self.subscription_ids_for_account_view(&account_id);
        self.collect_paginated(from_index, limit, ids.len() as u64, |index| {
            ids.get(index as usize)
                .and_then(|id| self.internal_get_subscription(id))
                .map(|sub| self.project_subscription_view_now(sub))
        })
    }
}

// Epoch pipeline: subscription update tail callback.

#[near]
impl Contract {
    /// **[Pipeline 5d]** Subscription update after pre-user settlement (**4**).
    #[private]
    pub fn on_subscription_update_after_settle(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        target_price_id: PriceId,
        target_amount: U128,
        subscription_id: SubscriptionId,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<SubscriptionPlanChangeOutcome> {
        let outcome = self.commit_subscription_stake_increase(
            buyer,
            deposit,
            target_price_id,
            target_amount,
            subscription_id,
            validator_id.clone(),
        );
        let _validator = self.require_validator_busy(
            &validator_id,
            "Validator pool must be busy after per-epoch settlement",
        );
        // Pre-user settlement (**0–3**) already ran; new `pending_to_stake` from the update
        // is queued for the next user action or `epoch_settle` (same as **5a** catalog mint).
        PromiseOrValue::Value(outcome)
    }
}

impl Contract {
    pub(crate) fn internal_get_subscription(&self, id: &SubscriptionId) -> Option<Subscription> {
        self.subscriptions.get(id).cloned().map(Into::into)
    }

    pub(crate) fn internal_set_subscription(
        &mut self,
        id: SubscriptionId,
        subscription: Subscription,
    ) {
        self.subscriptions.insert(id, subscription.into());
    }

    pub(crate) fn add_subscription_to_account_index(
        &mut self,
        account_id: &AccountId,
        subscription_id: &SubscriptionId,
    ) {
        let mut ids = self
            .subscriptions_by_account
            .get(account_id)
            .cloned()
            .unwrap_or_default();
        if !ids.iter().any(|id| id == subscription_id) {
            ids.push(subscription_id.clone());
            self.subscriptions_by_account
                .insert(account_id.clone(), ids);
        }
    }

    pub(crate) fn remove_subscription_from_account_index(
        &mut self,
        account_id: &AccountId,
        subscription_id: &SubscriptionId,
    ) {
        let Some(mut ids) = self.subscriptions_by_account.get(account_id).cloned() else {
            return;
        };
        let before = ids.len();
        ids.retain(|id| id != subscription_id);
        if ids.len() != before {
            self.subscriptions_by_account
                .insert(account_id.clone(), ids);
        }
    }

    fn subscription_ids_for_account_view(&self, account_id: &AccountId) -> Vec<SubscriptionId> {
        self.subscriptions_by_account
            .get(account_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Resolve `(account, product)` index, load subscription, verify caller ownership. Panics with stable user-facing messages.
    pub(crate) fn require_subscription_owned_by(
        &self,
        buyer: &AccountId,
        product_id: &ProductId,
    ) -> (SubscriptionId, Subscription) {
        if let Some(sid) = self
            .subscription_by_account_product
            .get(&(buyer.clone(), product_id.clone()))
            .cloned()
        {
            let sub = self.require_subscription_by_id(&sid);
            require!(
                sub.account_id == *buyer,
                "Only the subscription owner can perform this action"
            );
            let projected = self.project_subscription_view_now(sub.clone());
            require!(
                projected.product_id == *product_id,
                "No subscription for this product; subscribe first"
            );
            return (sid, sub);
        }

        for sid in self.subscription_ids_for_account_view(buyer) {
            let sub = self.require_subscription_by_id(&sid);
            if sub.account_id != *buyer {
                continue;
            }
            let projected = self.project_subscription_view_now(sub.clone());
            if projected.product_id == *product_id {
                return (sid, sub);
            }
        }

        env::panic_str("No subscription for this product; subscribe first")
    }

    pub(crate) fn find_subscription_owned_by_projected_product(
        &self,
        buyer: &AccountId,
        product_id: &ProductId,
    ) -> Option<(SubscriptionId, Subscription)> {
        self.subscription_ids_for_account_view(buyer)
            .into_iter()
            .find_map(|sid| {
                let sub = self.require_subscription_by_id(&sid);
                if sub.account_id != *buyer || sub.status != SubscriptionStatus::Active {
                    return None;
                }
                let projected = self.project_subscription_view_now(sub.clone());
                (projected.product_id == *product_id).then_some((sid, sub))
            })
    }

    pub(crate) fn move_subscription_product_index(
        &mut self,
        buyer: &AccountId,
        subscription_id: &SubscriptionId,
        old_product_id: &ProductId,
        new_product_id: &ProductId,
    ) {
        if old_product_id == new_product_id {
            return;
        }

        let new_key = (buyer.clone(), new_product_id.clone());
        if let Some(existing) = self.subscription_by_account_product.get(&new_key) {
            require!(
                existing == subscription_id,
                "Subscription already exists for target product"
            );
        } else {
            self.subscription_by_account_product
                .insert(new_key, subscription_id.clone());
        }

        let old_key = (buyer.clone(), old_product_id.clone());
        if let Some(existing) = self.subscription_by_account_product.get(&old_key) {
            if existing == subscription_id {
                self.subscription_by_account_product.remove(&old_key);
            }
        }
    }

    pub(crate) fn require_subscription_by_id(
        &self,
        subscription_id: &SubscriptionId,
    ) -> Subscription {
        self.internal_get_subscription(subscription_id)
            .unwrap_or_else(|| env::panic_str("Subscription not found"))
    }

    pub(crate) fn require_subscription_owned_by_id(
        &self,
        buyer: &AccountId,
        subscription_id: &SubscriptionId,
    ) -> Subscription {
        let sub = self.require_subscription_by_id(subscription_id);
        require!(
            sub.account_id == *buyer,
            "Only the subscription owner can perform this action"
        );
        sub
    }

    pub(crate) fn assert_subscription_active(sub: &Subscription) {
        require!(
            sub.status == SubscriptionStatus::Active,
            "This subscription is not active (cancelled, expired, or not yet started)"
        );
    }

    pub(crate) fn validate_subscription_target_amount(&self, price: &Price, target_amount: U128) {
        require!(
            target_amount.0 >= price.amount.0,
            "Target stake amount is below the price minimum"
        );
        if let Some(max_amount) = price.metadata.as_ref().and_then(|m| m.max_amount) {
            require!(
                target_amount.0 <= max_amount.0,
                "Target stake amount is above the price maximum"
            );
        }
    }

    fn clear_pending_downgrade(sub: &mut Subscription) {
        sub.pending_downgrade_price_id = None;
        sub.pending_downgrade_target_amount = None;
        sub.pending_downgrade_apply_ns = None;
    }

    fn projected_subscription_window_from(&self, anchor_day: u8, mut start: u64) -> (U64, U64) {
        let now = block_timestamp();
        let mut end = add_months_stripe_style(anchor_day, 1, start);
        while now >= end {
            start = end;
            end = add_months_stripe_style(anchor_day, 1, start);
        }
        (U64(start), U64(end))
    }

    pub(crate) fn apply_due_subscription_downgrade(
        &mut self,
        subscription_id: &SubscriptionId,
    ) -> bool {
        let Some(stored) = self.internal_get_subscription(subscription_id) else {
            return false;
        };
        if stored.status != SubscriptionStatus::Active || stored.cancel_at_period_end {
            return false;
        }
        let Some(apply_ns) = stored.pending_downgrade_apply_ns else {
            return false;
        };
        if block_timestamp() < apply_ns.0 {
            return false;
        }

        let Some(target_price_id) = stored.pending_downgrade_price_id.clone() else {
            return false;
        };
        let Some(target_amount) = stored.pending_downgrade_target_amount else {
            env::panic_str("Pending downgrade target amount missing");
        };
        let mut target_price = self.require_price(&target_price_id);
        let mut target_product = self.require_product(&target_price.product_id);
        let old_product_id = stored.product_id.clone();
        let new_product_id = target_product.product_id.clone();
        let buyer = stored.account_id.clone();

        self.apply_scheduled_stake_decrease_at_renewal(&buyer, &stored, target_amount);

        let (period_start_ns, period_end_ns) =
            self.projected_subscription_window_from(stored.anchor_day, apply_ns.0);

        if let Some(mut lock) = self.internal_get_lock(&stored.last_lock_id) {
            if lock.account_id == buyer && lock.status == LockStatus::Active {
                lock.start_ns = period_start_ns;
                lock.end_ns = period_end_ns;
                lock.order = OrderRef::Subscription {
                    subscription_id: stored.subscription_id.clone(),
                    price_id: target_price_id.clone(),
                    period_start_ns,
                    period_end_ns,
                };
                self.internal_set_lock(lock.lock_id.clone(), lock);
            }
        }

        let mut updated = stored;
        updated.product_id = new_product_id.clone();
        updated.price_id = target_price_id.clone();
        updated.start_ns = period_start_ns;
        updated.end_ns = period_end_ns;
        Self::clear_pending_downgrade(&mut updated);
        self.move_subscription_product_index(
            &buyer,
            subscription_id,
            &old_product_id,
            &new_product_id,
        );
        self.internal_set_subscription(subscription_id.clone(), updated);
        target_price.usage_count = target_price.usage_count.saturating_add(1);
        target_product.usage_count = target_product.usage_count.saturating_add(1);
        self.internal_set_price(target_price.price_id.clone(), target_price);
        self.internal_set_product(target_product.product_id.clone(), target_product);
        crate::events::log_subscription_update(
            &buyer,
            &target_price_id,
            target_amount.as_yoctonear(),
        );
        true
    }

    fn checked_subscription_update_inputs(
        &self,
        buyer: &AccountId,
        target_price_id: &PriceId,
        target_amount: U128,
        subscription_id: &SubscriptionId,
        expected_validator_id: Option<&ValidatorId>,
    ) -> SubscriptionUpdateInputs {
        let (target_price, target_product) = self.get_active_price_and_product(target_price_id);
        self.require_recurring_monthly_price(&target_price);
        self.validate_subscription_target_amount(&target_price, target_amount);

        let sub = self.require_subscription_owned_by_id(buyer, subscription_id);
        Self::assert_subscription_active(&sub);
        // Same virtual billing window as [`Contract::get_subscription`] / `get_subscription_for_product`.
        let sub = self.project_subscription_view_now(sub);

        let current_price = self.require_price(&sub.price_id);

        let lock = self.require_subscription_lock_owned_by(&sub, buyer);
        require!(
            target_product.validator_id == lock.validator_id,
            "Target product validator must match subscription lock validator"
        );
        if let Some(expected) = expected_validator_id {
            require!(
                target_product.validator_id == *expected,
                "Catalog validator for this price does not match the pool used for this subscription update"
            );
            require!(
                lock.validator_id == *expected,
                "Subscription lock validator does not match the update validator"
            );
        }

        let now = block_timestamp();
        require!(
            now < sub.end_ns.0,
            "Current period already ended; renew instead"
        );
        let current_amount = lock.amount_near.as_yoctonear();
        match target_amount.0.cmp(&current_amount) {
            std::cmp::Ordering::Greater => require!(
                target_price.amount.0 > current_price.amount.0,
                "Target price amount must increase when target stake amount increases"
            ),
            std::cmp::Ordering::Less => require!(
                target_price.amount.0 < current_price.amount.0,
                "Target price amount must decrease when target stake amount decreases"
            ),
            std::cmp::Ordering::Equal => require!(
                target_price.amount.0 == current_price.amount.0,
                "Target price amount must stay equal when target stake amount is unchanged"
            ),
        }

        let rem_ns = u128::from(sub.end_ns.0.saturating_sub(now));
        check_near_price_lock(&target_price, target_amount.0, rem_ns)
            .unwrap_or_else(|e| env::panic_str(e));

        SubscriptionUpdateInputs {
            sub,
            target_price,
            target_product,
            lock,
        }
    }

    fn commit_subscription_plan_change_without_stake_delta(
        &mut self,
        buyer: AccountId,
        subscription_id: SubscriptionId,
        target_price_id: PriceId,
        target_amount: U128,
        mut sub: Subscription,
        mut target_price: Price,
        mut target_product: Product,
        mut lock: Lock,
    ) -> SubscriptionPlanChangeOutcome {
        let old_product_id = sub.product_id.clone();
        let new_product_id = target_price.product_id.clone();
        Self::clear_pending_downgrade(&mut sub);
        lock.order = OrderRef::Subscription {
            subscription_id: sub.subscription_id.clone(),
            price_id: target_price_id.clone(),
            period_start_ns: sub.start_ns,
            period_end_ns: sub.end_ns,
        };
        sub.product_id = new_product_id.clone();
        sub.price_id = target_price_id.clone();
        target_price.usage_count = target_price.usage_count.saturating_add(1);
        target_product.usage_count = target_product.usage_count.saturating_add(1);

        self.internal_set_lock(lock.lock_id.clone(), lock);
        self.move_subscription_product_index(
            &buyer,
            &subscription_id,
            &old_product_id,
            &new_product_id,
        );
        self.internal_set_subscription(subscription_id.clone(), sub);
        self.internal_set_price(target_price.price_id.clone(), target_price);
        self.internal_set_product(target_product.product_id.clone(), target_product);

        crate::events::log_subscription_update(&buyer, &target_price_id, target_amount.0);
        SubscriptionPlanChangeOutcome {
            kind: "changed_immediately".to_string(),
            subscription_id,
            target_price_id,
            target_amount,
            lock_id: None,
        }
    }

    fn schedule_subscription_decrease(
        &mut self,
        buyer: AccountId,
        subscription_id: SubscriptionId,
        mut sub: Subscription,
        target_price: Price,
        target_product: Product,
        target_amount: NearToken,
    ) {
        let lock = self.require_subscription_lock_owned_by(&sub, &buyer);
        require!(
            target_product.validator_id == lock.validator_id,
            "Target product validator must match subscription lock validator"
        );
        if sub.product_id != target_product.product_id {
            let target_key = (buyer.clone(), target_product.product_id.clone());
            if let Some(existing) = self.subscription_by_account_product.get(&target_key) {
                require!(
                    existing == &subscription_id,
                    "Subscription already exists for target product"
                );
            }
        }
        Self::clear_pending_downgrade(&mut sub);
        sub.pending_downgrade_price_id = Some(target_price.price_id.clone());
        sub.pending_downgrade_target_amount = Some(target_amount);
        sub.pending_downgrade_apply_ns = Some(sub.end_ns);
        self.internal_set_subscription(subscription_id, sub);
        crate::events::log_subscription_downgrade_scheduled(&buyer, &target_price.price_id);
    }

    pub(crate) fn commit_subscription_stake_increase(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        target_price_id: PriceId,
        target_amount: U128,
        subscription_id: SubscriptionId,
        expected_validator_id: ValidatorId,
    ) -> SubscriptionPlanChangeOutcome {
        let SubscriptionUpdateInputs {
            mut sub,
            mut target_price,
            mut target_product,
            mut lock,
            ..
        } = self.checked_subscription_update_inputs(
            &buyer,
            &target_price_id,
            target_amount,
            &subscription_id,
            Some(&expected_validator_id),
        );

        let add_shares = self.internal_stake(&buyer, &expected_validator_id, deposit);
        let old_product_id = sub.product_id.clone();
        let new_product_id = target_price.product_id.clone();
        Self::clear_pending_downgrade(&mut sub);

        lock.amount_near = lock
            .amount_near
            .checked_add(deposit)
            .expect("lock amount_near overflow");
        lock.shares = U128(lock.shares.0.saturating_add(add_shares));
        lock.order = OrderRef::Subscription {
            subscription_id: sub.subscription_id.clone(),
            price_id: target_price_id.clone(),
            period_start_ns: sub.start_ns,
            period_end_ns: sub.end_ns,
        };

        sub.product_id = new_product_id.clone();
        sub.price_id = target_price_id.clone();
        target_price.usage_count = target_price.usage_count.saturating_add(1);
        target_product.usage_count = target_product.usage_count.saturating_add(1);

        let lock_id_out = lock.lock_id.clone();
        self.internal_set_lock(lock_id_out.clone(), lock);
        self.move_subscription_product_index(
            &buyer,
            &subscription_id,
            &old_product_id,
            &new_product_id,
        );
        self.internal_set_subscription(subscription_id.clone(), sub);
        self.internal_set_price(target_price.price_id.clone(), target_price);
        self.internal_set_product(target_product.product_id.clone(), target_product);

        crate::events::log_subscription_update(&buyer, &target_price_id, target_amount.0);
        crate::events::log_lock(lock_id_out.as_str(), &buyer);

        SubscriptionPlanChangeOutcome {
            kind: "changed_immediately".to_string(),
            subscription_id,
            target_price_id,
            target_amount,
            lock_id: Some(lock_id_out),
        }
    }

    /// At scheduled decrease renewal, release surplus stake from the completed subscription lock
    /// as shares → same unstake queue as [`crate::unlock::Contract::unlock`].
    pub(crate) fn apply_scheduled_stake_decrease_at_renewal(
        &mut self,
        buyer: &AccountId,
        sub: &Subscription,
        target_amount: NearToken,
    ) {
        let mut lock = match self.internal_get_lock(&sub.last_lock_id) {
            Some(l) => l,
            None => return,
        };
        if &lock.account_id != buyer || lock.status != LockStatus::Active {
            return;
        }
        let surplus_target = lock
            .amount_near
            .as_yoctonear()
            .saturating_sub(target_amount.as_yoctonear());
        if surplus_target == 0 {
            return;
        }

        let validator_id = lock.validator_id.clone();
        let validator = self.require_validator(&validator_id);
        let net_stake = validator.net_stake_yocto();
        let validator_total_shares = validator.total_shares.0;
        let lock_near_val = near_from_shares(lock.shares.0, net_stake, validator_total_shares);
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

        let near_amt = self.internal_unstake(buyer.clone(), validator_id, shares_remove);
        lock.shares = U128(lock.shares.0.saturating_sub(shares_remove));
        let new_amt = lock.amount_near.as_yoctonear().saturating_sub(near_amt);
        lock.amount_near = NearToken::from_yoctonear(new_amt);
        if lock.shares.0 == 0 {
            lock.status = LockStatus::UnlockRequested;
        }
        self.internal_set_lock(lock.lock_id.clone(), lock);

        crate::events::log_subscription_downgrade_prorate(buyer, &sub.product_id, near_amt);
    }

    pub(crate) fn project_subscription_view_now(&self, mut sub: Subscription) -> Subscription {
        if sub.status != SubscriptionStatus::Active || sub.cancel_at_period_end {
            return sub;
        }
        let now = block_timestamp();
        if let Some(apply_ns) = sub.pending_downgrade_apply_ns {
            if now >= apply_ns.0 {
                if let Some(target_price_id) = sub.pending_downgrade_price_id.clone() {
                    let target_price = self.require_price(&target_price_id);
                    sub.product_id = target_price.product_id;
                    sub.price_id = target_price_id;
                }
                let (start_ns, end_ns) =
                    self.projected_subscription_window_from(sub.anchor_day, apply_ns.0);
                sub.start_ns = start_ns;
                sub.end_ns = end_ns;
                Self::clear_pending_downgrade(&mut sub);
                return sub;
            }
        }
        while now >= sub.end_ns.0 {
            let next_start = sub.end_ns.0;
            let next_end = add_months_stripe_style(sub.anchor_day, 1, next_start);
            sub.start_ns = U64(next_start);
            sub.end_ns = U64(next_end);
        }
        sub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_month_stack() {
        let out = add_months_stripe_style(15, 2, 100);
        assert_eq!(out, 100 + (2u128 * AVG_MONTH_NS) as u64);
    }
}
