//! Subscription billing helpers (Stripe-style linear months) and subscription **lifecycle** RPCs
//! (`cancel_subscription`, `upgrade_subscription`, …). Subscription **locking** (`lock_for_subscription`)
//! stays in [`crate::lock`] because it shares the pool refresh / mint pipeline with product locks.

use crate::utils::{
    AVG_MONTH_NS, block_timestamp, check_near_price_lock, min_locked_yocto_for_duration,
    near_from_shares,
};
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

#[near]
impl Contract {
    /// Stop renewing after the current billing period. The active lock remains until `lock.end_ns`; use
    /// [`crate::unlock::Contract::unlock`] afterwards. Attach 1 yocto.
    #[payable]
    pub fn cancel_subscription(&mut self, product_id: ProductId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();
        let (sid, sub) = self.require_subscription_owned_by(&buyer, &product_id);
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
        let (sid, mut sub) = self.require_subscription_owned_by(&buyer, &product_id);
        Self::assert_subscription_active(&sub);
        require!(
            sub.cancel_at_period_end,
            "Subscription is not scheduled to cancel at period end"
        );
        require!(
            block_timestamp() < sub.end_ns.0,
            "Current billing period has ended; subscribe again with lock_for_subscription instead"
        );
        sub.cancel_at_period_end = false;
        self.internal_set_subscription(sid.clone(), sub.clone());
        crate::events::log_subscription_resume(&buyer, &product_id);
    }

    /// Upgrade to a higher-priced recurring tier on the same product immediately. Attach extra NEAR so that
    /// `existing_locked + deposit` satisfies [`check_near_price_lock`] for the new tier over the remainder of
    /// the current period (`lock.end_ns - now`). Runs the shared per-epoch validator pipeline before minting
    /// additional shares (same as [`crate::lock::Contract::lock_for_subscription`] on WASM).
    #[payable]
    pub fn upgrade_subscription(&mut self, new_price_id: PriceId) -> PromiseOrValue<LockId> {
        self.require_enough_gas_for_epoch_settlement();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();

        let deposit = env::attached_deposit();
        require!(
            deposit.as_yoctonear() >= self.internal_get_config().min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let (sid, _sub, _new_price, product, _lock) =
            self.checked_subscription_upgrade_inputs(&buyer, deposit, &new_price_id, None, None);
        let sid = sid.expect("Subscription id is available when looked up by product");

        let validator_id = product.validator_id.clone();
        self.assert_validator_active_for_lock(&validator_id);

        let _validator = self.require_validator_idle(&validator_id);

        #[cfg(not(target_arch = "wasm32"))]
        {
            return PromiseOrValue::Value(self.commit_subscription_upgrade(
                buyer,
                deposit,
                new_price_id,
                sid,
                validator_id,
            ));
        }
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .promise_validator_per_epoch_settlement_then(
                    validator_id.clone(),
                    UserAction::SubscriptionUpgrade {
                        validator_id,
                        buyer,
                        deposit,
                        new_price_id,
                        subscription_id: sid,
                    },
                )
                .into();
        }
    }

    /// Schedule a lower tier for the **next** billing period (Phase A: applied at renewal; no automatic refund).
    #[payable]
    pub fn schedule_downgrade_subscription(&mut self, target_price_id: PriceId) {
        assert_one_yocto();
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();

        let target = self.require_active_recurring_monthly_price(&target_price_id);

        let (sid, mut sub) = self.require_subscription_owned_by(&buyer, &target.product_id);
        require!(
            target.product_id == sub.product_id,
            "Price must belong to this subscription product"
        );

        let current = self.require_price(&sub.price_id);
        require!(
            target.amount.0 < current.amount.0,
            "Target tier must have a lower catalog amount than current tier"
        );

        sub.pending_downgrade_price_id = Some(target_price_id.clone());
        self.internal_set_subscription(sid, sub.clone());

        crate::events::log_subscription_downgrade_scheduled(&buyer, &target_price_id);
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
        let sid = self
            .subscription_by_account_product
            .get(&(account_id, product_id.clone()))?
            .clone();
        self.internal_get_subscription(&sid)
            .map(|sub| self.project_subscription_view_now(sub))
    }

    pub fn get_subscription_for_price(
        &self,
        account_id: AccountId,
        price_id: PriceId,
    ) -> Option<Subscription> {
        let price = self.internal_get_price(&price_id)?;
        self.get_subscription_for_product(account_id, price.product_id.clone())
    }
}

// Epoch pipeline: subscription upgrade tail callback.

#[near]
impl Contract {
    /// **[Pipeline 5d]** Subscription upgrade after pre-user settlement (**4**).
    #[private]
    pub fn on_subscription_upgrade_after_settle(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        new_price_id: PriceId,
        subscription_id: SubscriptionId,
        validator_id: ValidatorId,
    ) -> PromiseOrValue<LockId> {
        let lock_id = self.commit_subscription_upgrade(
            buyer,
            deposit,
            new_price_id,
            subscription_id,
            validator_id.clone(),
        );
        let _validator = self.require_validator_busy(
            &validator_id,
            "Validator pool must be busy after per-epoch settlement",
        );
        // Pre-user settlement (**0–3**) already ran; new `pending_to_stake` from the upgrade
        // is queued for the next user action or `epoch_settle` (same as **5a** catalog mint).
        PromiseOrValue::Value(lock_id)
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

    /// Resolve `(account, product)` index, load subscription, verify caller ownership. Panics with stable user-facing messages.
    pub(crate) fn require_subscription_owned_by(
        &self,
        buyer: &AccountId,
        product_id: &ProductId,
    ) -> (SubscriptionId, Subscription) {
        let sid = self
            .subscription_by_account_product
            .get(&(buyer.clone(), product_id.clone()))
            .cloned()
            .unwrap_or_else(|| env::panic_str("No subscription for this product; subscribe first"));
        let sub = self.require_subscription_by_id(&sid);
        require!(
            sub.account_id == *buyer,
            "Only the subscription owner can perform this action"
        );
        (sid, sub)
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

    fn checked_subscription_upgrade_inputs(
        &self,
        buyer: &AccountId,
        deposit: NearToken,
        new_price_id: &PriceId,
        subscription_id: Option<&SubscriptionId>,
        expected_validator_id: Option<&ValidatorId>,
    ) -> (Option<SubscriptionId>, Subscription, Price, Product, Lock) {
        let (new_price, product) = self.get_active_price_and_product(new_price_id);
        self.require_recurring_monthly_price(&new_price);

        let (sid, sub) = match subscription_id {
            Some(sid) => (None, self.require_subscription_owned_by_id(buyer, sid)),
            None => {
                let (sid, sub) = self.require_subscription_owned_by(buyer, &new_price.product_id);
                (Some(sid), sub)
            }
        };
        Self::assert_subscription_active(&sub);
        // Same virtual billing window as [`Contract::get_subscription`] / `get_subscription_for_product`.
        let sub = self.project_subscription_view_now(sub);

        require!(
            new_price.product_id == sub.product_id,
            "Price must belong to this subscription product"
        );
        let old_price = self.require_price(&sub.price_id);
        require!(
            new_price.amount.0 > old_price.amount.0,
            "New tier must have a higher catalog amount than current tier"
        );

        let lock = self.require_subscription_lock_owned_by(&sub, buyer);
        if let Some(expected) = expected_validator_id {
            require!(
                product.validator_id == *expected,
                "Catalog validator for this price does not match the pool used for this subscription upgrade"
            );
            require!(
                lock.validator_id == *expected,
                "Subscription lock validator does not match the upgrade validator"
            );
        }

        let now = block_timestamp();
        require!(
            now < sub.end_ns.0,
            "Current period already ended; renew instead"
        );
        let rem_ns = u128::from(sub.end_ns.0.saturating_sub(now));
        let total_locked = lock
            .amount_near
            .as_yoctonear()
            .saturating_add(deposit.as_yoctonear());
        check_near_price_lock(&new_price, total_locked, rem_ns)
            .unwrap_or_else(|e| env::panic_str(e));

        (sid, sub, new_price, product, lock)
    }

    pub(crate) fn commit_subscription_upgrade(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        new_price_id: PriceId,
        subscription_id: SubscriptionId,
        expected_validator_id: ValidatorId,
    ) -> LockId {
        let (_sid, mut sub, mut new_price, mut product, mut lock) = self
            .checked_subscription_upgrade_inputs(
                &buyer,
                deposit,
                &new_price_id,
                Some(&subscription_id),
                Some(&expected_validator_id),
            );

        let add_shares = self.internal_stake(&buyer, &expected_validator_id, deposit);

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
        new_price.usage_count = new_price.usage_count.saturating_add(1);
        product.usage_count = product.usage_count.saturating_add(1);

        let lock_id_out = lock.lock_id.clone();
        self.internal_set_lock(lock_id_out.clone(), lock);
        self.internal_set_subscription(subscription_id, sub);
        self.internal_set_price(new_price.price_id.clone(), new_price);
        self.internal_set_product(product.product_id.clone(), product);

        crate::events::log_subscription_upgrade(&buyer, &new_price_id);
        crate::events::log_lock(lock_id_out.as_str(), &buyer);

        lock_id_out
    }

    /// At subscription renewal commit: apply scheduled downgrade proration from **stored** subscription
    /// (completed period + `last_lock_id`), then clear `pending_downgrade_price_id` on `subscription`.
    ///
    /// Idempotent: if storage already has no pending downgrade, returns without mutating locks.
    pub(crate) fn apply_pending_downgrade_before_renewal_lock(
        &mut self,
        buyer: &AccountId,
        subscription_id: &SubscriptionId,
        subscription: &mut Subscription,
    ) {
        // First subscribe persists the subscription only at the end of `commit_catalog_lock`.
        let Some(stored) = self.internal_get_subscription(subscription_id) else {
            return;
        };
        let Some(low_id) = stored.pending_downgrade_price_id.clone() else {
            return;
        };
        let completed_period_ns = u128::from(stored.end_ns.0.saturating_sub(stored.start_ns.0));
        let high_price = self.require_price(&stored.price_id);
        let low_price = self.require_price(&low_id);

        if completed_period_ns > 0 {
            self.apply_downgrade_prorate_at_renewal(
                buyer,
                &stored,
                &high_price,
                &low_price,
                completed_period_ns,
            );
        }

        subscription.price_id = low_id.clone();
        subscription.pending_downgrade_price_id = None;

        // Clear pending downgrade in storage before the renewal lock is minted so a failed async
        // pipeline cannot apply the same proration again on retry.
        let mut stored_update = stored;
        stored_update.price_id = low_id;
        stored_update.pending_downgrade_price_id = None;
        self.internal_set_subscription(subscription_id.clone(), stored_update);
    }

    /// Phase B: at scheduled downgrade renewal, release catalog **tier-gap** stake (min high − min low for
    /// the completed period) as shares → same unstake queue as [`crate::unlock::Contract::unlock`].
    pub(crate) fn apply_downgrade_prorate_at_renewal(
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
        let min_h = min_locked_yocto_for_duration(high_price, completed_period_ns);
        let min_l = min_locked_yocto_for_duration(low_price, completed_period_ns);
        let surplus_target = min_h.saturating_sub(min_l);
        if surplus_target == 0 {
            return;
        }

        let mut lock = match self.internal_get_lock(&sub.last_lock_id) {
            Some(l) => l,
            None => return,
        };
        if &lock.account_id != buyer || lock.status != LockStatus::Active {
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

    fn project_subscription_view_now(&self, mut sub: Subscription) -> Subscription {
        if sub.status != SubscriptionStatus::Active || sub.cancel_at_period_end {
            return sub;
        }
        let now = block_timestamp();
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
