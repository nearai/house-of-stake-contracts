# staking-contract — public API

Reference for **on-chain methods** exposed by `staking-contract` (Rust type names below match JSON **camelCase** field names from [`near-sdk`] serialization unless noted).

**Conventions**

- **`#[payable]` + “attach 1 yocto”**: method calls [`near_sdk::assert_one_yocto()`](https://docs.rs/near-sdk/latest/near_sdk/fn.assert_one_yocto.html); attach exactly **1 yoctoNEAR**.
- **Other `#[payable]`**: attach the stated NEAR (e.g. storage, lock stake).
- **Operators**: `epoch_*` methods require `predecessor` ∈ `config.operators` **unless** `operators` is empty, in which case **any** account may call them.
- **Catalog auth**: mutating catalog methods assert the caller equals the staking pool’s **`get_owner_id()`** for that validator’s pool (cross-contract view), not a cached field on-chain.
- **Pause**: when `paused == true`, most mutating user/operator paths revert (see individual methods).

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
| `get_account` | `account_id: AccountId` | `Account \| null` | NEP-style storage + `withdrawable_balance`. |
| `get_validator` | `validator_id: AccountId` | `Validator \| null` | Validator row for one staking pool contract account. |
| `get_validators` | `from_index: u64`, `limit: u64` | `Validator[]` | Paginated allowlist (stable ordering); each row’s **`validator_id`** is that pool’s account id. |
| `get_product` | `product_id: string` | `Product \| null` | Catalog product (`prod_*`). |
| `get_price` | `price_id: string` | `Price \| null` | Catalog price (`price_*`). |
| `get_products` | `from_index: u64`, `limit: u64` | `Product[]` | Paginated catalog (stable creation order in contract index). |
| `get_product_default_price` | `product_id: string` | `string \| null` | Same as **`Product.default_price_id`** from **`get_product`** / **`get_products`** — default catalog **`price_id`** (see **`set_product_default_price`**); **`null`** if unset. |
| `get_lock` | `lock_id: string` | `Lock \| null` | Lock record (`lock_*`). |
| `get_subscription` | `subscription_id: string` | `Subscription \| null` | Subscription (`sub_*`). |
| `get_subscription_for_product` | `account_id`, `product_id` | `Subscription \| null` | Lookup by `(account, product)`. |
| `get_subscription_for_price` | `account_id`, `price_id` | `Subscription \| null` | Resolves product from price, then same as above. |

---

## Storage & balances (`accounts.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `storage_deposit` | Any | **Attach NEAR** | Register/update prepaid storage: must satisfy `min_storage_deposit` + `per_lock_storage_stake × user_lock_count`. |
| `storage_withdraw` | Account owner | **1 yocto** + logical `amount: NearToken` | Withdraw prepaid storage down to the required minimum for current lock count. Returns transfer promise. |

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

## Locks & subscriptions (`lock.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `lock_for_product` | Buyer | **Attach NEAR** | One-off purchase: JSON **`price_id`**, **`lock_duration_ns`** (`U64`), **`product_id`**. Provide **exactly one** of **`price_id`** or **`product_id`** (the other **`null`**). If **`product_id`** is set, uses **`Product.default_price_id`** (must be a **one-off** price). Returns **`lock_id`**. |
| `lock_for_subscription` | Subscriber | **Attach NEAR** | Recurring (monthly): **`price_id`**, **`product_id`** — same XOR rule as **`lock_for_product`**; default price must be **recurring** monthly. |
| `cancel_subscription` | Subscriber | **1 yocto** | `product_id` — stop renewing after current period (`cancel_at_period_end`). After **`end_ns`**, the next **`lock_for_subscription`** replaces the row so the user may subscribe again (index is not left stale). |
| `upgrade_subscription` | Subscriber | **Attach NEAR** (≥ `min_lock_amount`; tier differential) | `new_price_id` — upgrade recurring tier mid-period; returns **`LockId`**. |
| `schedule_downgrade_subscription` | Subscriber | **1 yocto** | `target_price_id` — schedule lower tier for next billing period. |

---

## Unlock (`unlock.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `unlock` | Lock owner | **1 yocto** | After **`block_timestamp >= lock.end_ns`**, queues unstake for this lock’s shares (`UnlockRequested`). |

---

## Epoch operations — operators (`epoch.rs`)

**`epoch_stake`**, **`epoch_unstake`**, **`epoch_withdraw`**, **`refresh_validator_balance`:** require **`assert_not_paused`** and **`assert_operator`** (if **`config.operators`** is non-empty, calls must come from a listed operator; if empty, any account may call — same rules for all four). Each **`epoch_*`** and **`refresh_validator_balance`** returns a **`Promise`** to the staking pool / callbacks.

| Method | Description |
|--------|-------------|
| `epoch_stake` | `validator_id` — stake **`pending_to_stake`** via pool **`deposit_and_stake`**. Serialized per pool (`tx_status`, one stake batch per epoch). |
| `epoch_unstake` | `validator_id` — unstake **`pending_to_unstake`**. Gated by **`epoch_unstake_settle_epochs`** vs last unstake epoch. |
| `epoch_withdraw` | `validator_id` — after settle epochs, pull unstaked NEAR from pool into **`pending_to_withdraw`**. |
| `refresh_validator_balance` | `validator_id` — **`get_account_total_balance`** callback updates **`Validator.total_staked_balance`**. Shares **`tx_status`** serialization with **`epoch_*`**. |

---

## Pool promise callbacks (`pool_callbacks.rs`)

**Not intended for users.** Marked **`#[private]`** — only the contract account may call (promise continuation).

| Method | Role |
|--------|------|
| `on_deposit_and_stake` | Completes `epoch_stake`; clears pending / updates stake epoch / balance. |
| `on_unstake` | Completes `epoch_unstake`. |
| `on_get_unstaked_for_epoch_withdraw` | Continues `epoch_withdraw`; may chain **`withdraw`** on pool. |
| `on_epoch_withdraw_transfer_done` | Credits **`pending_to_withdraw`** after pool transfer. |
| `on_refresh_total_balance` | Completes **`refresh_validator_balance`**. |

---

## Withdrawals & claims (`withdraw.rs`)

| Method | Access | Deposit | Description |
|--------|--------|---------|-------------|
| `claim_unlocked_near` | User | **1 yocto** | `validator_id` — pro-rata claim from **`pending_to_withdraw`** into **`withdrawable_balance`** (requires prior **`epoch_withdraw`** and liability bookkeeping). |
| `withdraw` | User | **1 yocto** | JSON args are **`{ "amount": <NearToken> }`**; use **`"amount": null`** to withdraw the full **`withdrawable_balance`**. A bare JSON **`null`** body does not deserialize (near-sdk wraps args in a struct keyed by parameter names). |
| `sweep_stranded_withdraw_bucket` | **Owner** | **1 yocto** | `validator_id` — if user liability total is zero but **`pending_to_withdraw`** remains, send remainder to **`owner_account_id`**. |

---

## Governance (`governance.rs`)

**Caller:** **`owner_account_id`** unless noted. All attach **1 yocto**.

| Method | Description |
|--------|-------------|
| `propose_new_owner_account_id` | `new_owner_account_id: AccountId \| null`. |
| `accept_ownership` | **Proposed** account accepts (must match `proposed_new_owner_account_id`). |
| `set_guardians` | Replace **`guardians`** list. |
| `set_operators` | Replace **`operators`** list (empty ⇒ anyone may run **`epoch_*`**). |
| `set_per_lock_storage_stake` | Per-lock storage surcharge config. |
| `set_lock_bounds` | `min_lock_duration_ns`, `max_lock_duration_ns` (`U64`). |
| `set_min_lock_amount` | Minimum attach for locks. |
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

- **`Config`** — [`../src/config.rs`](../src/config.rs): `owner_account_id`, `guardians`, `operators`, lock/storage economics, `epoch_unstake_settle_epochs`, …
- **`Validator`** — [`../src/validators.rs`](../src/validators.rs): **`validator_id`** (pool contract account), accounting fields, pending buckets, **`tx_status`** (`Idle` \| `Busy`).
- **`Product`**, **`Price`**, **`Subscription`**, **`Lock`**, **`Account`** — [`../src/types.rs`](../src/types.rs), [`../src/accounts.rs`](../src/accounts.rs).

For EVENT_JSON shapes and naming, see [`../src/events.rs`](../src/events.rs).

---

## Related

| Doc | Content |
|-----|---------|
| [DESIGN.md](DESIGN.md) | Architecture overview |
| [PLAN.md](PLAN.md) | Detailed design notes |
