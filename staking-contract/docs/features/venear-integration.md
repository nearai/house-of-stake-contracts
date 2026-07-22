# veNEAR integration for stake.dao locks and farms

**Status:** Design (v2)  
**Component:** `staking-contract` (`stake.dao`) + `venear-contract`  
**Supersedes:** Deferred veNEAR item in [DESIGN.md](../DESIGN.md) (non-goals §1)
**Tracking issue:** [#36](https://github.com/nearai/house-of-stake-contracts/issues/36)

**Related code:**

| Contract | Path |
|----------|------|
| stake.dao | [staking-contract/src/](../../src/) — `lock.rs`, `unlock.rs`, `subscriptions.rs`, `stake.rs`, `epoch.rs` |
| veNEAR | [venear-contract/src/lockup.rs](../../../venear-contract/src/lockup.rs) — `on_lockup_update`, `internal_lockup_update` |
| lockup (reference) | [lockup-contract/src/venear.rs](../../../lockup-contract/src/venear.rs) — `lock_near`, `begin_unlock_near`, `venear_lockup_update` |
| shared types | [common/src/lockup_update.rs](../../../common/src/lockup_update.rs) — `LockupUpdateV1`, `VLockupUpdate` |

---

## 1. Summary

This design grants **veNEAR voting power** to users who lock or stake NEAR through the `stake.dao` catalog flows, using the **same economics** as NEAR locked via a user lockup account: base veNEAR from eligible locked/staked NEAR principal, time-based extra veNEAR accrual, and forfeiture of accumulated extra veNEAR when reported locked NEAR decreases.

The integration is intentionally **minimal**: reuse `LockupUpdateV1` and mirror the lockup → veNEAR cross-contract call pattern; do not redesign the lazy epoch pipeline, share accounting, or voting contract.

**Clarification:** In `stake.dao`, “staking NEAR” currently means one of the catalog-backed flows:

- `lock` / subscription lock: NEAR attached on lock methods, delegated to a validator pool, represented by `Lock`.
- `stake` for farm prices: NEAR attached to a farm product, delegated to a validator pool, represented by `FarmPosition`.

There is no separate bare-validator stake entrypoint in v1.

---

## 2. Problem and scope

### 2.1 In scope

- veNEAR updates when a user creates or changes an eligible **catalog lock** or **farm position** that opts in to veNEAR registration.
- Lifecycle hooks: lock mint (`commit_catalog_lock` / `resolve_lock`), unlock (`resolve_unlock`), subscription **upgrade** (additional deposit on lock), subscription **downgrade prorate** (reduction of `Lock.amount_near`), farm stake (`resolve_farm_stake`), and farm unstake (`resolve_farm_unstake`).
- veNEAR contract changes to accept a second allowlisted reporter (`stake.dao`) without breaking lockup updates.
- Migration and rollout for both contracts.

### 2.2 Out of scope

- veNEAR for NEAR that is only in `user_pending_unstake` or withdrawn.
- veNEAR for farm reward units (`pending_reward_units`, `accumulated_reward_units`, or `total_earned_reward_units`). Farm rewards are not NEAR principal.
- Generic liquid stake without catalog (future API would need its own design).
- Fungible receipt / share tokens on `stake.dao`.
- Changes to [voting-contract](../../../voting-contract/) (still consumes veNEAR snapshots).
- Indexer-only or event-driven minting as the source of truth for balances.
- veNEAR delegation UX on `stake.dao` (users continue to use `venear-contract` directly).

---

## 3. Reference behavior: lockup → veNEAR

Today, veNEAR power for lockup users works as follows (see [house-of-stake-contracts/README.md](../../../README.md)):

1. User **registers** on `venear-contract` (`storage_deposit`).
2. User deploys a **lockup** subaccount via veNEAR and calls `lock_near` on the lockup.
3. Lockup tracks `venear_locked_balance` and calls `venear.on_lockup_update(version, owner, update)`.
4. veNEAR updates the owner’s Merkle-tree account: `VenearBalance.near_balance` (plus storage `deposit`), accrues **extra veNEAR** over time via `VenearGrowthConfig`, and **zeros `extra_venear_balance`** when the reported locked NEAR amount **decreases**.

### 3.1 Update payload

[`LockupUpdateV1`](../../../common/src/lockup_update.rs):

| Field | Role |
|-------|------|
| `locked_near_balance` | Total NEAR counted for veNEAR base (lockup: `venear_locked_balance`) |
| `timestamp` | Update time (nanoseconds) |
| `lockup_update_nonce` | Monotonic per lockup; stale updates rejected |

Wrapped as `VLockupUpdate::V1(...)`.

### 3.2 veNEAR handler (lockup path)

[`on_lockup_update`](../../../venear-contract/src/lockup.rs) requires `predecessor == get_lockup_account_id(owner)` and delegates to [`internal_lockup_update`](../../../venear-contract/src/lockup.rs):

- Nonce must increase.
- If new locked NEAR &lt; previous `account.balance.near_balance` → `extra_venear_balance = 0`.
- `account.balance.near_balance = near_add(lockup_update.locked_near_balance, account_internal.deposit)`.
- Global pooled totals and delegation mirrors updated.

### 3.3 Lockup owner actions

[`lockup-contract/src/venear.rs`](../../../lockup-contract/src/venear.rs):

| Action | Effect on veNEAR-reported locked NEAR |
|--------|--------------------------------------|
| `lock_near` | Increase `venear_locked_balance` → `venear_lockup_update()` |
| `begin_unlock_near` | Decrease locked, move to pending → update (forfeit extra if total down) |
| `end_unlock_near` / `lock_pending_near` | Adjust pending vs locked → update |

Gas for the veNEAR call: `GAS_FOR_VENEAR_LOCKUP_UPDATE` (~20 TGas) in [lockup-contract/src/venear_ext.rs](../../../lockup-contract/src/venear_ext.rs).

**stake.dao should mirror this reporter pattern**, not reimplement veNEAR math locally.

---

## 4. Architecture

```mermaid
sequenceDiagram
    participant User
    participant StakeDao as stake.dao
    participant Pool as validatorPool
    participant VeNEAR as venear.dao

    User->>StakeDao: lock/stake + NEAR
    StakeDao->>Pool: epoch pipeline deposit_and_stake
    StakeDao->>StakeDao: commit lock or farm position
    StakeDao->>VeNEAR: on_stake_dao_update
    Note over VeNEAR: near_balance = lockup_locked + stake_dao_locked + deposit

    User->>StakeDao: unlock/unstake
    StakeDao->>StakeDao: internal_unstake + position update
    StakeDao->>VeNEAR: on_stake_dao_update lower total
    Note over VeNEAR: extra_venear forfeited if total decreased
```

### 4.1 Design principles

| Principle | Choice |
|-----------|--------|
| Source of truth | **Direct** cross-contract calls from `stake.dao` to veNEAR (same as lockup) |
| Payload type | Reuse `VLockupUpdate` / `LockupUpdateV1` — **no new common type** |
| Reporter trust | veNEAR allowlists `stake.dao` account id in config |
| Billing vs governance | Catalog `end_ns` gates **unlock**; veNEAR drop aligns with **`unlock()`** (like `begin_unlock_near`), not `withdraw()` |
| Eligible amount | Principal-like eligible NEAR, not validator-share mark-to-market |
| Integration default | **Opt-in per lock/position** (`register_with_venear`, default `false`) for migration safety |

Existing `lock_create` events remain for indexers; they are **not** sufficient for balance updates.

---

## 5. Eligible NEAR accounting (principal model)

### 5.1 Eligible sources

This design separates two different concepts:

- **Realtime staked value:** current mark-to-market NEAR represented by validator shares. This is useful for views and withdrawals.
- **veNEAR-eligible principal:** NEAR amount reported to veNEAR. This should mirror lockup semantics and only change on user-level lock/stake/unlock/unstake transitions.

| Source | Count toward veNEAR? | Notes |
|--------|----------------------|-------|
| One-off catalog `Lock` | Yes, if opted in and `Active` | Use `Lock.amount_near`. |
| Recurring subscription `Lock` | Yes, if opted in and `Active` | Cancellation at period end does not reduce veNEAR until unlock/stake reduction. |
| Subscription upgrade | Yes | Increase by the additional principal deposited. |
| Subscription downgrade/prorate | Yes | Decrease by the principal removed from the lock. |
| Farm `FarmPosition` | Open decision; recommended yes if user opts in | Requires explicit principal tracking because the stored position is share-based. |
| Farm reward units | No | Reward units are not NEAR principal. |
| `user_pending_unstake` / `pending_to_withdraw` / `pending_to_claim` | No | Same class as lockup pending unlock: no longer counted as locked veNEAR principal. |

### 5.2 Definition

For a user `account_id`:

```text
stake_dao_locked(account_id) =
  locked_principal(account_id)
  + farm_principal(account_id)

locked_principal(account_id) =
  Σ lock.amount_near for active opt-in locks

farm_principal(account_id) =
  Σ tracked opt-in farm principal for active farm positions
```

`stake.dao` reports `stake_dao_locked` to veNEAR on each material change.

### 5.3 Principal vs mark-to-market

| Model | Pros | Cons |
|-------|------|------|
| **Principal** (`Lock.amount_near`, plus tracked farm principal) | Matches lockup’s explicit `venear_locked_balance`; O(1) aggregate; no extra pool views | Ignores slash-induced share value loss; requires explicit farm principal tracking |
| Mark-to-market (`near_from_shares(shares, validator.net_stake_yocto(), validator.total_shares)`) | Economically closer to current staked value | Cached until validator balance refresh; async pipeline coupling; diverges from lockup semantics; cannot be maintained in a per-user map without unbounded updates |

**Decision:** use **principal** for veNEAR base NEAR.

Slashing: locks and farm positions keep share count; `near_from_shares(...)` can fall while principal remains unchanged. This is the same class of risk as lockup NEAR staked in pools without reducing `venear_locked_balance`.

### 5.4 Realtime user staked balance view

If product or analytics code needs a live-ish user stake value, it can be derived from validator shares:

```text
user_staked_near_balance(account_id) =
  Σ near_from_shares(
      user_validator_shares[(account_id, validator_id)],
      validator.net_stake_yocto(),
      validator.total_shares
    )
  for each validator where the user has shares
```

This is not suitable as a cached `LookupMap<AccountId, NearToken>` because validator share price changes globally when rewards accrue, slashing happens, or validator settlement refreshes `total_staked_balance`. Updating every affected user on each share-price change would be unbounded.

If this view is needed, add a bounded index:

```text
user_validator_ids: LookupMap<AccountId, Vec<ValidatorId>>
```

and expose a view such as `get_user_staked_near_balance(account_id)`. Treat the result as based on the staking contract's latest cached validator state, not a direct realtime staking-pool query.

### 5.5 On-chain aggregate (stake.dao)

Maintain incrementally (avoid scanning all locks):

```text
user_venear_eligible_principal: LookupMap<AccountId, u128> // yoctoNEAR
stake_dao_venear_nonce: LookupMap<AccountId, u64>          // per-user nonce for veNEAR updates
```

| Event | Delta to `user_venear_eligible_principal` |
|-------|-------------------------------|
| `commit_catalog_lock` with `register_with_venear` | `+ lock.amount_near` |
| `resolve_unlock` for opt-in lock | `- lock.amount_near` |
| Subscription upgrade (deposit on lock) | `+` top-up amount |
| Downgrade prorate (reduces `lock.amount_near`) | `-` prorated reduction |
| `resolve_farm_stake` with opt-in farm position | `+` added farm principal |
| `resolve_farm_unstake` for opt-in farm position | `-` principal removed from the farm position |

Locks in `UnlockRequested` or `Withdrawn` do not count (removed at unlock).

Farm positions need one additional decision before implementation: either store per-position eligible principal directly, or maintain only the per-account aggregate and rely on carefully paired stake/unstake deltas. Storing per-position principal is easier to audit and repair.

---

## 6. stake.dao changes

### 6.1 Configuration

[`config.rs`](../../src/config.rs):

```rust
pub venear_account_id: Option<AccountId>,
```

- Set by contract owner after veNEAR deploy (`set_venear_account_id`).
- If `None`, veNEAR integration is disabled (v1 behavior).

### 6.2 Lock and farm position types

[`types.rs`](../../src/types.rs) — extend `Lock` and `FarmPosition`:

```rust
pub register_with_venear: bool,  // default false on migrate
```

Expose optional argument on:

- `lock(..., register_with_venear: Option<bool>)`
- `stake(product_id, price_id, register_with_venear: Option<bool>)`

Default: `false` (recommended for rollout).

For farms, also add explicit principal if farm positions count toward veNEAR:

```rust
pub venear_principal: NearToken, // or farm_principal_near
```

Current `FarmPosition` stores validator shares and the view derives `staked_near_amount` from share price. That is useful for current-value views, but veNEAR should not depend on a cached share-price conversion.

### 6.3 New module `venear.rs`

Patterned on [lockup-contract/src/venear.rs](../../../lockup-contract/src/venear.rs):

- `ext_venear` contract trait: `on_stake_dao_update(owner_account_id, update: VLockupUpdate)`
- `GAS_FOR_VENEAR_STAKE_DAO_UPDATE` ≈ 20 TGas (same as lockup)
- `fn venear_stake_dao_update(&mut self, owner: AccountId) -> Promise`
  - Increments per-user nonce in `stake_dao_venear_nonce`
  - Sends `VLockupUpdate::V1(LockupUpdateV1 { locked_near_balance, timestamp, lockup_update_nonce })`
  - `locked_near_balance` = `user_venear_eligible_principal[owner]`

Register module in [`lib.rs`](../../src/lib.rs).

### 6.4 Hook points

| Location | When to call veNEAR |
|----------|---------------------|
| [`lock.rs`](../../src/lock.rs) — `resolve_lock` | After successful `commit_catalog_lock`, if lock opted in and `venear_account_id` set |
| [`unlock.rs`](../../src/unlock.rs) — `resolve_unlock` | After `internal_unstake` + status `UnlockRequested`, if lock had opt-in |
| [`subscriptions.rs`](../../src/subscriptions.rs) | Upgrade tail: increase aggregate by deposit delta; downgrade prorate: decrease by NEAR removed from lock |
| [`stake.rs`](../../src/stake.rs) — `resolve_farm_stake` | After successful `commit_farm_stake`, if farm position opted in and `venear_account_id` set |
| [`stake.rs`](../../src/stake.rs) — `resolve_farm_unstake` | After `commit_farm_unstake`, decrease aggregate by the principal removed from the farm position |

Chain the veNEAR promise **after** the epoch pipeline tail succeeds (fire-and-forget, same as lockup’s `venear_lockup_update()` at end of `lock_near`). Extend [`gas.rs`](../../src/gas.rs) callback budgets (`ON_LOCK_FINALLY_MINT`, `ON_UNLOCK_TAIL_AFTER_PRE_USER`) if the promise is attached in the release callback chain.

### 6.5 Exit timing vs veNEAR

| stake.dao step | veNEAR effect |
|----------------|---------------|
| `unlock()` at `now >= end_ns` | Report **lower** `stake_dao_locked` (like `begin_unlock_near`) |
| Farm `unstake()` | Report **lower** `stake_dao_locked` when the farm position principal decreases |
| Pool unstake / `withdraw()` | **No** additional veNEAR change (pending unstake not counted, like lockup pending) |

### 6.6 Failure policy

- **Lock/unlock must not revert** if the veNEAR promise fails (lockup does not roll back `lock_near` on veNEAR failure either).
- Emit `venear_sync_failed` event (account, nonce, intended balance) for monitoring.
- Optional repair: `sync_venear(account_id)` — owner or user recomputes aggregate from locks and pushes update (see §11).

### 6.7 User prerequisites

1. Register on veNEAR: `storage_deposit` on `venear-contract`.
2. Opt in on lock or farm stake: `register_with_venear: true`.

When (2) without (1): **require** at lock entry (cross-contract view or explicit check) — recommended to fail fast with a clear error rather than silent skip.

---

## 7. venear-contract changes

### 7.1 Configuration

[`venear-contract/src/config.rs`](../../../venear-contract/src/config.rs):

```rust
pub stake_dao_account_id: Option<AccountId>,
```

Owner sets after `stake.dao` deployment. `on_stake_dao_update` requires `env::predecessor_account_id() == stake_dao_account_id`.

### 7.2 AccountInternal split

Today `internal_lockup_update` sets:

```text
near_balance = lockup_reported_locked + deposit
```

which **overwrites** the full base NEAR on every lockup update. Adding stake.dao requires **separate stored components**:

```rust
pub struct AccountInternal {
    pub lockup_version: Option<Version>,
    pub deposit: NearToken,
    pub lockup_update_nonce: U64,
    pub lockup_locked_near: NearToken,       // NEW — migrated from tree
    pub stake_dao_locked_near: NearToken,    // NEW
    pub stake_dao_update_nonce: U64,         // NEW
}
```

Recompute on any update:

```text
account.balance.near_balance =
  near_add(near_add(lockup_locked_near, stake_dao_locked_near), deposit)
```

Refactor `internal_lockup_update` into a shared helper, e.g. `internal_apply_locked_near_update(source, owner, update)`:

- **Lockup:** update `lockup_locked_near` + `lockup_update_nonce`; preserve `stake_dao_locked_near`.
- **Stake.dao:** update `stake_dao_locked_near` + `stake_dao_update_nonce`; preserve `lockup_locked_near`.

Retain existing rules: nonce monotonicity, extra veNEAR forfeit when **total** base NEAR (after recompute) decreases vs previous `account.balance` before update, delegation/global state propagation unchanged.

### 7.3 New public method

```rust
pub fn on_stake_dao_update(
    &mut self,
    owner_account_id: AccountId,
    update: VLockupUpdate,
)
```

- Require owner registered (`internal_get_account_internal`).
- Require `stake_dao_account_id` configured.
- Match `VLockupUpdate::V1` and call shared internal helper with source = StakeDao.

### 7.4 Double-counting policy

For users with **both** lockup and stake.dao:

```text
effective_locked_for_venear = lockup_locked_near + stake_dao_locked_near + deposit
```

- Lockup reports only `venear_locked_balance`.
- stake.dao reports only eligible opt-in locks and farm positions.
- **Operational policy:** the same NEAR must not be intentionally locked in both systems for voting power; governance/docs should state this clearly.

---

## 8. Parity matrix: lockup vs stake.dao

| Behavior | Lockup | stake.dao (this design) |
|----------|--------|-------------------------|
| Base veNEAR from | `venear_locked_balance` | Active opt-in lock principal + active opt-in farm principal |
| Extra veNEAR growth | Global `VenearGrowthConfig` | Same |
| Forfeit extra on decrease | `begin_unlock_near` (and any locked decrease) | `unlock()` or farm `unstake()` that lowers eligible principal |
| Pending / unstaking NEAR | Not in veNEAR locked total | `user_pending_unstake` not counted |
| Time gate before exit | `unlock_duration_ns` (veNEAR pending bucket) | Catalog `end_ns` (billing only) |
| Reporter contract | User lockup subaccount | `stake.dao` allowlisted |
| Pool staking | Optional inside lockup | Required for catalog locks and farm positions |
| Per-position opt-in | N/A (whole lockup account) | `register_with_venear` per lock/farm position |
| Rewards | Staking rewards do not auto-increase `venear_locked_balance` | Farm reward units do not count toward veNEAR |

---

## 9. Migration and rollout

### 9.1 veNEAR upgrade

1. Add `AccountInternal` fields; migrate existing accounts:
   - `lockup_locked_near` ← reconstruct from current `account.balance.near_balance - deposit` (stake_dao_locked_near = 0).
   - `stake_dao_update_nonce` = 0.
2. Deploy `on_stake_dao_update`; `stake_dao_account_id = None` until linked.

### 9.2 stake.dao upgrade

1. Add `venear_account_id`, `register_with_venear` (default false), maps `user_venear_eligible_principal`, `stake_dao_venear_nonce`, and any farm per-position principal field.
2. veNEAR hooks no-op until `venear_account_id` is set.
3. Existing locks and farm positions default to `register_with_venear = false`; do not backfill voting power unless governance explicitly approves a migration/import flow.

### 9.3 Linking (owner)

1. Set `venear.config.stake_dao_account_id = stake.dao`.
2. Set `stake.dao.config.venear_account_id = venear.dao`.

### 9.4 User-facing rollout

1. Register on veNEAR (`storage_deposit`).
2. Lock or farm-stake on stake.dao with `register_with_venear: true`.
3. Unlock/unstake to reduce veNEAR (same as beginning unlock on lockup).

---

## 10. Testing checklist

| Scenario | Expected |
|----------|----------|
| Lock with opt-in, registered on veNEAR | `user_venear_eligible_principal` increases; veNEAR base NEAR increases; extra accrues over time |
| `unlock()` on opt-in lock | veNEAR base decreases; extra veNEAR forfeited |
| Farm stake with opt-in | Farm principal increases `user_venear_eligible_principal`; farm reward units do not count |
| Farm unstake | Farm principal decreases; pending unstake/withdraw buckets do not count |
| Share price changes after rewards/slashing | `get_user_staked_near_balance` view may change after validator refresh; veNEAR principal does not auto-change |
| User with lockup + stake.dao | Lockup update does not zero stake.dao component; totals additive |
| Stale / replay nonce | veNEAR rejects update |
| Opt-in lock, not registered on veNEAR | Lock fails at entry (if check enabled) |
| veNEAR promise fails after lock commits | Lock exists; `venear_sync_failed` emitted; `sync_venear` repairs |
| Subscription upgrade / downgrade prorate | Aggregate and veNEAR update match `amount_near` delta |
| `register_with_venear: false` | No veNEAR calls; aggregate unchanged |

Suggested locations: extend [integration-tests](../../../integration-tests/) (venear + stake.dao), unit tests on aggregate math, sandbox tests following [tests/sandbox_epoch_settlement.rs](../../tests/sandbox_epoch_settlement.rs) patterns.

---

## 11. Open questions (for review)

1. **Opt-in default**
   - **Recommended:** per-lock/per-position `register_with_venear`, default `false`.
   - Alternatives: account-level `venear_enabled` on stake.dao `Account`; or default-on for all eligible positions (stronger product signal, more registration friction).

2. **Registration check timing**  
   - Fail at lock entry with cross-contract `get_account_info` vs allow lock and fail async on veNEAR (worse UX).

3. **`sync_venear` permissions**  
   - Public (anyone can repair desync) vs owner-only vs lock owner only.

4. **Events**  
   - Whether to extend `lock_create` JSON with `register_with_venear` for indexers (non-authoritative).

5. **Governance disclosure**  
   - README/DESIGN slashing note: veNEAR principal may exceed pool mark-to-market after slash.

6. **Farm eligibility**
   - Recommended: allow farm positions to opt in, but report tracked farm principal, not reward units and not share-derived mark-to-market value.
   - Alternative: exclude farms from the first veNEAR integration and add them after the lock/subscription path is stable.

7. **Farm principal storage**
   - Recommended: store per-position veNEAR principal for auditable repair and migration.
   - Alternative: maintain only `user_venear_eligible_principal` with stake/unstake deltas; simpler storage but harder to repair after bugs.

8. **Realtime user staked balance view**
   - Useful as `get_user_staked_near_balance(account_id)` derived from validator shares.
   - Should remain separate from veNEAR accounting because it depends on cached validator state and share price.

9. **Existing positions**
   - Decide whether existing locks/farm positions can be imported into veNEAR with an explicit user action, or whether only new opt-in positions count.

---

## 12. Implementation checklist (engineering)

- [ ] `common`: no change (reuse `LockupUpdateV1`)
- [ ] `venear-contract`: config, `AccountInternal`, migration, `on_stake_dao_update`, refactor `internal_lockup_update`
- [ ] `staking-contract`: config, `Lock`, `FarmPosition`, maps, optional realtime stake view/index, `venear.rs`, hooks, gas, governance setter, tests
- [ ] `docs`: update [DESIGN.md](../DESIGN.md) architecture diagram when implemented; [API.md](../API.md) new methods/args
- [ ] Deploy scripts: link `stake_dao_account_id` / `venear_account_id` in [scripts/deploy_all.sh](../../../scripts/deploy_all.sh)

---

## Appendix: deferred v1 seam (now specified)

The earlier v1 notes only proposed:

- `lock_create` / `unlock_settled` events for veNEAR consumption
- `register_with_venear: bool` on `Lock`

This design **keeps** `register_with_venear` and events for observability, extends the question to farm positions, and specifies **direct `on_stake_dao_update` calls** as the authoritative balance path, matching production lockup behavior.
