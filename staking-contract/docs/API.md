# staking-contract — public API

Reference for **on-chain methods** exposed by `staking-contract` (Rust type names below match JSON **camelCase** field names from [`near-sdk`] serialization unless noted).

**Conventions**

- **`#[payable]` + “attach 1 yocto”**: method calls [`near_sdk::assert_one_yocto()`](https://docs.rs/near-sdk/latest/near_sdk/fn.assert_one_yocto.html); attach exactly **1 yoctoNEAR**.
- **Other `#[payable]`**: attach the stated NEAR (e.g. storage, lock stake).
- **Catalog auth**: mutating catalog methods assert the caller equals the staking pool’s **`get_owner_id()`** for that validator’s pool (cross-contract view), not a cached field on-chain.
- **Pause**: when `paused == true`, most mutating user paths revert (see individual methods).

---

## Initialization

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `new` | Deploy init | — | — | **`#[init]`** — constructs contract from `config` (see [`Config`](../src/config.rs)). |

---

## Views (read-only)

| Method | Parameters | Returns | Description |
|--------|------------|---------|-------------|
| `get_config` | — | `Config` | Full governance & economics config. |
| `get_version` | — | `string` | Crate package version string. |
| `is_paused` | — | `bool` | Global pause flag. |
| `get_account` | `account_id: AccountId` | `Account \| null` | NEP-style prepaid **`storage_deposit`** only. |
| `get_validator` | `validator_id: AccountId` | `Validator \| null` | Validator row for one staking pool contract account. |
| `get_validators` | `from_index: u64`, `limit: u64` | `Validator[]` | Paginated allowlist (stable ordering); each row’s **`validator_id`** is that pool’s account id. |
| `get_product` | `product_id: string` | `Product \| null` | Catalog product (`prod_*`). |
| `get_price` | `price_id: string` | `Price \| null` | Catalog price (`price_*`). |
| `get_products` | `from_index: u64`, `limit: u64` | `Product[]` | Paginated catalog (stable creation order in contract index). |
| `get_product_default_price` | `product_id: string` | `string \| null` | Same as **`Product.default_price_id`** from **`get_product`** / **`get_products`** — default catalog **`price_id`** (see **`set_product_default_price`**); **`null`** if unset. |
| `get_purchase` | `purchase_id: string` | `Purchase \| null` | Direct-payment purchase record (`pay_*`). |
| `get_purchases` | `from_index: u64`, `limit: u64` | `Purchase[]` | Paginated direct-payment purchases. |
| `get_purchases_for_account` | `account_id`, `from_index`, `limit` | `Purchase[]` | Paginated direct-payment purchases by buyer. |
| `get_purchases_for_product` | `product_id`, `from_index`, `limit` | `Purchase[]` | Paginated direct-payment purchases by product. |
| `get_revenue_balance_for_validator` | `validator_id` | `NearToken` | Withdrawable direct-payment revenue for a validator. |
| `get_revenue_balance_for_product` | `product_id` | `NearToken` | Gross direct-payment revenue recorded for a product. |
| `get_lock` | `lock_id: string` | `Lock \| null` | Lock record (`lock_*`). |
| `get_subscription` | `subscription_id: string` | `Subscription \| null` | Subscription (`sub_*`). |
| `get_subscription_for_product` | `account_id`, `product_id` | `Subscription \| null` | Lookup by `(account, product)`. |
| `get_subscription_for_price` | `account_id`, `price_id` | `Subscription \| null` | Resolves product from price, then same as above. |

---

## Storage & balances (`accounts.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `storage_deposit` | Any | **Attach NEAR** | Register/update prepaid storage: must satisfy `min_storage_deposit` + `per_lock_storage_stake × user_lock_count` + `per_purchase_storage_stake × user_purchase_count`. |
| `storage_withdraw` | Account owner | **1 yocto** + logical `amount: NearToken` | Withdraw prepaid storage down to the required minimum for current lock and purchase counts. Returns transfer promise. |

---

## Validator allowlist (`validators.rs`)

**Caller:** contract **`owner_account_id`** (from `config`), unless noted.

| Method | Deposit | Description |
|--------|---------|-------------|
| `add_validator` | **1 yocto** | Allowlist a **`validator_id`** (staking pool contract account). Fails if that validator row already exists. |
| `pause_validator` | **1 yocto** | Set validator **`ValidatorStatus::Paused`** (blocks **new** locks for that pool). |
| `remove_validator` | **1 yocto** | Mark **`Removed`** when no shares / pending stake / unstake / withdraw buckets (see contract checks). |

---

## Catalog — products & prices (`products.rs`)

All mutation entrypoints attach **1 yocto**, require contract **not paused**, validator **allowlisted**, then **`Promise`** chain: pool **`get_owner_id()`** → **`#[private]`** callback verifies `pool_owner == predecessor` of the **original** call.

**Returns:** most return **`Promise`** (async completion of catalog write).

| Method | Description |
|--------|-------------|
| `create_product` | `validator_id`, `name`, `description` → creates `prod_*`. |
| `edit_product` | `product_id`, `name`, `description`. |
| `archive_product` | `product_id`. |
| `unarchive_product` | `product_id` — restore **`CatalogStatus::Active`** (must currently be archived). |
| `delete_product` | `product_id` (invariants: no attached prices in use — see contract). |
| `create_price` | `product_id`, `name`, `description`, `amount` (`U128` yocto), `price_type`, `billing_period`, `lock_factor_near_months`. |
| `edit_price` | `price_id`, `name`, `description`. |
| `archive_price` | `price_id`. |
| `unarchive_price` | `price_id` — restore **`CatalogStatus::Active`** (must currently be archived). |
| `delete_price` | `price_id`. |
| `set_product_default_price` | `product_id`, **`price_id`: optional** — set or clear **`Product.default_price_id`**. **`price_id`** must refer to an **active** (unarchived) catalog price on that product; archived prices are rejected (**unarchive** first). Cleared when **`archive_product`**, **`archive_price`** (if that price was the default), or **`delete_price`**. |

---

## Locks (`lock.rs`)

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `lock` | Buyer / subscriber | **Attach NEAR** | **`PromiseOrValue<LockId>`** | **`price_id`**, **`product_id`**, **`duration_ns`** — provide exactly one of **`price_id`** or **`product_id`**. One-off prices require **`duration_ns`** (`U64`). Recurring monthly subscription prices require **`duration_ns: null`** and derive the duration from the billing period. Default price from **`Product.default_price_id`** when only **`product_id`** is set. **WASM:** shared per-epoch pipeline (**0–3**) then mint (**5a**); see [LAZY_EPOCH_PIPELINE.md](LAZY_EPOCH_PIPELINE.md). **Host tests:** synchronous mint (no promise chain). |

## Direct payments (`payments.rs`)

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `pay` | Buyer | **Attach exact NEAR price × quantity** | `PurchaseId` | Direct one-off payment for `price_id` or a product default price. Requires an active one-off price with no billing period, creates a `pay_*` purchase record, increments product/price usage, and accrues validator revenue. Does not create a lock or touch pool staking. |
| `withdraw_revenue` | Validator owner | **1 yocto** | `Promise` | `validator_id`, optional `amount`. Verifies ownership through pool `get_owner_id()`, then transfers direct-payment revenue to the validator owner. Omitted `amount` withdraws the full validator balance. |

---

## Subscriptions (`subscriptions.rs`)

Lifecycle RPCs (locking / renewal stays in **`lock`** above).

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `cancel_subscription` | Subscriber | **1 yocto** | — | **`product_id`** — set **`cancel_at_period_end`**; lock remains until **`lock.end_ns`**, then **`unlock`**. After **`end_ns`**, next **`lock`** starts a new period. |
| `resume_subscription` | Subscriber | **1 yocto** | — | **`product_id`** — clear **`cancel_at_period_end`** while **`Active`**, only before stored **`end_ns`** (current billing period). Fails after period end; use **`lock`** for a new period. Requires **`cancel_at_period_end == true`**. |
| `update_subscription` | Subscriber | **Attach delta NEAR for increases; 1 yocto otherwise** | **`PromiseOrValue<SubscriptionPlanChangeOutcome>`** | **`subscription_id`, `target_price_id`, `target_amount`** — unified plan update. Stake increases apply immediately after the same pre-user pipeline as **`lock`**; stake decreases are scheduled for the billing boundary, projected in views after `apply_ns`, and lazily committed by later mutations after validator settlement; price-only changes with unchanged stake apply immediately. |

---

## Unlock (`unlock.rs`)

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `unlock` | Lock owner | **1 yocto** | **`Promise`** | After **`block_timestamp >= lock.end_ns`**: pre-user settlement (**0–3**), then **`commit_share_exit`** (**5b**) — burns shares, queues **`pending_to_unstake`** and user tranches. Pool **`unstake`** for this exit is **not** called in the same transaction; it runs on the next **`lock`**, **`unlock`**, **`withdraw`**, or **`epoch_settle`** when the epoch slot is available. |

---

## Pool pipeline (`epoch.rs`)

Public **`epoch_stake` / `epoch_unstake` / `epoch_withdraw` / `refresh_validator_balance`** are **not** exposed. Pool work is driven by user flows below and optional **`epoch_settle`**.

**Per allowlisted pool (`validator_id` = pool contract account):**

- **`tx_status`**: **`Idle`** / **`Busy`** — one orchestrated pipeline at a time; **`Busy`** from entry until **`on_epoch_pipeline_terminal_release`** (**6**).
- **Per NEAR `epoch_height`**: at most **one** successful pool **`deposit_and_stake`** **or** **`unstake`** (or inline net-zero clear); updates **`last_settlement_epoch`**.
- **Fast path**: when **`last_settlement_epoch >= epoch_height`**, skip pool **`get_account`**, withdraw-if-ready, and net settle; jump to user tail (**4**) using cached **`total_staked_balance`**.
- **Full path**: pool **`get_account`** → optional withdraw (**2a–2c**) → **`try_epoch_stake_or_unstake`** on **existing** pending → user tail (**4**).
- **Unstake spacing**: another pool **`unstake`** requires **`validator_unstake_waiting_finished`** (`last_unstake_epoch` + **`epoch_unstake_settle_epochs`**).
- **Withdraw from pool** does **not** consume the stake/unstake epoch slot.

| Entry | `UserAction` tail | User tail |
|--------|-------------------|-----------|
| `lock` | `CommitLock` | Mint lock (**5a**); optional post-settle |
| `update_subscription` | `SubscriptionUpdate` | Update subscription lock or schedule decrease (**5d**); optional post-settle |
| `unlock` | `UnlockQueueUnstake` | Share exit only (**5b**) |
| `withdraw` (WASM) | `WithdrawUserTransfer` | Payout (**5c**) |
| `epoch_settle` | `SettleOnly` | No-op then **6** |

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `epoch_settle` | Any | **None** | **`Promise`** | **`validator_id`** — manual retry / advance pending stake or unstake; same rules as automatic flows. |

Pipeline steps and callbacks: [LAZY_EPOCH_PIPELINE.md](LAZY_EPOCH_PIPELINE.md).

---

## Private pool callbacks (`epoch.rs`, `lock.rs`, `unlock.rs`, `subscriptions.rs`)

**Not for users.** **`#[private]`** — only this contract account (promise continuations).

| Callback | Pipeline | Role |
|----------|----------|------|
| `on_epoch_settlement_after_pool_account` | **1** | After pool **`get_account`**: refresh **`total_staked_balance`**, optional **2a–2c**, then **3** with `Some(cont)`. |
| `on_epoch_withdraw_transfer_done` | **2b** | Credit **`pending_to_withdraw`** after pool **`withdraw`**. |
| `on_after_pool_withdraw_maybe_settle` | **2c** | After **2b**; **`Some(cont)`** → **3** + **4**, **`None`** → tail **3** only. |
| `on_deposit_and_stake` | **3b** | Stake callback; pending queue + **`last_settlement_epoch`**. |
| `on_unstake` | **3c** | Unstake callback; **`last_unstake_epoch`**, **`last_settlement_epoch`**. |
| `on_epoch_settlement_after_try_epoch_stake_or_unstake` | **3′** | After async **3** → **4**. |
| `on_epoch_settlement_dispatch_continue` | **4** | Fan-out to **5a** / **5b** / **5c** / **5d**, then **6**. |
| `on_lock_finally_mint_and_maybe_post_settle` | **5a** | Catalog mint (`lock.rs`). |
| `on_unlock_tail_after_pre_user_settle` | **5b** | Share exit (`unlock.rs`). |
| `on_subscription_upgrade_after_settle` | **5d** | Subscription upgrade (`subscriptions.rs`). |
| `on_epoch_pipeline_terminal_release` | **6** | Set **`tx_status`** → **`Idle`**. |

---

## Withdrawals & claims (`withdraw.rs`)

| Method | Access | Deposit | Returns | Description |
|--------|--------|---------|---------|-------------|
| `withdraw` | User | **1 yocto** | **`Promise`** | JSON **`{ "validator_id": <AccountId> }`** — claim from **`pending_to_withdraw`** for your epoch-eligible pending-unstake tranches on that pool (up to the bucket balance), then **transfer** the NEAR to you in the same flow. May chain an internal pool withdraw when the bucket is empty but settlement allows (see `docs/LAZY_EPOCH_PIPELINE.md`). |

> **Note:** An owner-only **`sweep_stranded_withdraw_bucket`**-style cleanup (when **`pending_user_unstake_total == 0`** but **`pending_to_withdraw > 0`**) is described in [DESIGN.md](DESIGN.md) but **not** exposed in the current ABI.

---

## Governance (`governance.rs`)

**Caller:** **`owner_account_id`** unless noted. All attach **1 yocto**.

| Method | Description |
|--------|-------------|
| `propose_new_owner_account_id` | `new_owner_account_id: AccountId \| null`. |
| `accept_ownership` | **Proposed** account accepts (must match `proposed_new_owner_account_id`). |
| `set_guardians` | Replace **`guardians`** list. |
| `set_per_lock_storage_stake` | Per-lock storage surcharge config. |
| `set_per_purchase_storage_stake` | Per-direct-purchase storage surcharge config. |
| `set_lock_bounds` | `min_lock_duration_ns`, `max_lock_duration_ns` (`U64`). |
| `set_min_lock_amount` | Minimum attach for locks; must be **≥ 1 NEAR** (`PROTOCOL_MIN_LOCK_AMOUNT_YOCTO` in `config.rs`). |
| `set_min_storage_deposit` | Minimum prepaid storage. |
| `set_epoch_unstake_settle_epochs` | Epochs to wait between unstake rounds / withdraw gates. |

---

## Pause (`pause.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `pause` | **Guardian** (or owner via `assert_guardian`) | **1 yocto** | Sets **`paused = true`**. |
| `unpause` | **Owner** | **1 yocto** | Sets **`paused = false`**. |

---

## Upgrade (`upgrade.rs`)

| Method | Access | Description |
|--------|--------|-------------|
| `migrate_state` | **`#[private]`** — contract account only | **`#[init(ignore_state)]`** — returns deserialized state after code upgrade (used by deploy script). |
| `get_version` | Any | Version string (see Views). |

---

## Catalog internal callbacks (`products.rs`)

**`#[private]`** — invoked only as promise callbacks after **`get_owner_id`** on the pool.

`create_product_after_get_owner`, `edit_product_after_get_owner`, `archive_product_after_get_owner`, `delete_product_after_get_owner`, `create_price_after_get_owner`, `edit_price_after_get_owner`, `archive_price_after_get_owner`, `delete_price_after_get_owner`, `unarchive_product_after_get_owner`, `unarchive_price_after_get_owner`, `set_product_default_price_after_get_owner`.

---

## Main types (contract JSON views)

- **`Config`** — [`../src/config.rs`](../src/config.rs): `owner_account_id`, `guardians`, lock/storage economics, `epoch_unstake_settle_epochs`, … **`min_lock_amount`** is the minimum attach for locks (including first delegation to an empty pool); governance may raise it but not below **`PROTOCOL_MIN_LOCK_AMOUNT_YOCTO`** (1 NEAR), enforced in `new` and `set_min_lock_amount`.
- **`Validator`** — [`../src/validators.rs`](../src/validators.rs): **`validator_id`** (pool contract account), accounting fields, pending buckets, **`tx_status`** (`Idle` \| `Busy`).
- **`Product`**, **`Price`**, **`Purchase`**, **`Subscription`**, **`Lock`**, **`Account`** — [`../src/types.rs`](../src/types.rs), [`../src/accounts.rs`](../src/accounts.rs). **`Account`** is prepaid **`storage_deposit`** only (unlocked stake exits transfer directly to the user via **`withdraw`**).

For EVENT_JSON shapes and naming, see [`../src/events.rs`](../src/events.rs).

---

## Related

| Doc | Content |
|-----|---------|
| [LAZY_EPOCH_PIPELINE.md](LAZY_EPOCH_PIPELINE.md) | Per-epoch limits, fast path, promise pipeline **0–6**, callbacks |
| [DESIGN.md](DESIGN.md) | Architecture overview |
| [PLAN.md](PLAN.md) | Detailed design notes |
