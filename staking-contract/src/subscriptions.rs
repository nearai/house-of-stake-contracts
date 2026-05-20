//! Subscription billing helpers (Stripe-style linear months) and subscription **lifecycle** RPCs
//! (`cancel_subscription`, `upgrade_subscription`, …). Subscription **locking** (`lock_for_subscription`)
//! stays in [`crate::lock`] because it shares the pool refresh / mint pipeline with product locks.

pub use crate::internal::AVG_MONTH_NS;
use crate::internal::{
    check_near_price_lock, min_locked_yocto_for_duration, near_from_shares, net_stake_yocto,
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
        let (sid, mut sub) = self.require_subscription_owned_by(&buyer, &product_id);
        Self::assert_subscription_active(&sub);
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
    /// the current period (`lock.end_ns - now`). Runs the shared per-epoch validator pipeline before minting
    /// additional shares (same as [`crate::lock::Contract::lock_for_subscription`] on WASM).
    #[payable]
    pub fn upgrade_subscription(&mut self, new_price_id: PriceId) -> PromiseOrValue<LockId> {
        self.assert_not_paused();
        let buyer = env::predecessor_account_id();

        let deposit = env::attached_deposit();
        require!(
            deposit.as_yoctonear() >= self.config.min_lock_amount.as_yoctonear(),
            "Attached NEAR is below the contract minimum lock amount (min_lock_amount)"
        );

        let (new_price, product) = self.get_active_price_and_product(&new_price_id);
        self.require_recurring_monthly_price(&new_price);

        let (sid, sub) = self.require_subscription_owned_by(&buyer, &new_price.product_id);

        let old_price = self.require_price(&sub.price_id);
        require!(
            new_price.product_id == sub.product_id,
            "Price must belong to this subscription product"
        );
        require!(
            new_price.amount.0 > old_price.amount.0,
            "New tier must have a higher catalog amount than current tier"
        );

        let lock = self.require_subscription_lock_owned_by(&sub, &buyer);

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

        let _validator = self.require_validator_idle(&validator_id);

        #[cfg(not(target_arch = "wasm32"))]
        {
            return PromiseOrValue::Value(self.commit_subscription_upgrade(
                buyer,
                deposit,
                new_price_id,
                sid,
            ));
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.require_enough_gas_for_epoch_settlement();
            return self
                .promise_validator_per_epoch_settlement_then(
                    validator_id.clone(),
                    PerEpochContinue::SubscriptionUpgrade {
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
        self.subscriptions.insert(sid, sub.clone());

        crate::events::log_subscription_downgrade_scheduled(&buyer, &target_price_id);
    }

    // -------------------------------------------------------------------------
    // Public subscription view functions
    // -------------------------------------------------------------------------

    pub fn get_subscription(&self, subscription_id: SubscriptionId) -> Option<Subscription> {
        self.subscriptions
            .get(subscription_id.as_str())
            .cloned()
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
        self.subscriptions
            .get(sid.as_str())
            .cloned()
            .map(|sub| self.project_subscription_view_now(sub))
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
        let lock_id =
            self.commit_subscription_upgrade(buyer, deposit, new_price_id, subscription_id);
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
        self.subscriptions
            .get(subscription_id.as_str())
            .cloned()
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

    pub(crate) fn commit_subscription_upgrade(
        &mut self,
        buyer: AccountId,
        deposit: NearToken,
        new_price_id: PriceId,
        subscription_id: SubscriptionId,
    ) -> LockId {
        let mut sub = self.require_subscription_owned_by_id(&buyer, &subscription_id);

        let (_, product) = self.require_price_and_product(&new_price_id);

        let mut lock = self.require_subscription_lock_owned_by(&sub, &buyer);

        let validator_id = product.validator_id.clone();
        let add_shares = self.internal_stake(&buyer, &validator_id, deposit);

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
        self.locks.insert(lock_id_out.clone(), lock);
        self.subscriptions.insert(subscription_id, sub);

        crate::events::log_subscription_upgrade(&buyer, &new_price_id);
        crate::events::log_lock(lock_id_out.as_str(), &buyer);

        lock_id_out
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

        let mut lock = match self.locks.get(&sub.last_lock_id).cloned() {
            Some(l) => l,
            None => return,
        };
        if &lock.account_id != buyer || lock.status != LockStatus::Active {
            return;
        }

        let validator_id = lock.validator_id.clone();
        let validator = self.require_validator(&validator_id);
        let net_stake = net_stake_yocto(
            validator.total_staked_balance,
            validator.pending_to_stake,
            validator.pending_user_unstake_total,
        );
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
        self.locks.insert(lock.lock_id.clone(), lock);

        crate::events::log_subscription_downgrade_prorate(buyer, &sub.product_id, near_amt);
    }

    fn project_subscription_view_now(&self, mut sub: Subscription) -> Subscription {
        if sub.status != SubscriptionStatus::Active || sub.cancel_at_period_end {
            return sub;
        }
        let now = env::block_timestamp();
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
