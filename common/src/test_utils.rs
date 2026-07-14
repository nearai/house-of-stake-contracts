//! Shared fixtures for fast unit tests across the contract crates.
//!
//! Gated behind the `test-utils` feature so this code never reaches a production
//! wasm build. Contract crates enable it via a dev-dependency on `common`.
//! Contract-specific builders (each crate's `fresh_contract`, `Config`, proposal
//! lifecycle helpers) stay in that crate; only contract-agnostic primitives and
//! shared named-account fixtures live here.

use crate::account::{Account, VAccount};
use crate::global_state::{GlobalState, VGlobalState};
use crate::venear::{VenearGrowthConfig, VenearGrowthConfigFixedRate};
use crate::{Fraction, PooledVenearBalance, TimestampNs, VenearBalance};
use merkle_tree::{MerkleProof, MerkleTree, MerkleTreeSnapshot};
use near_sdk::json_types::U128;
pub use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{AccountId, BorshStorageKey, NearToken, near, testing_env};
use std::collections::HashMap;

pub fn acc(id: &str) -> AccountId {
    id.parse().unwrap()
}

// ---------------------------------------------------------------------------
// Named-account fixtures shared across contract crates
// ---------------------------------------------------------------------------

pub fn current_account() -> AccountId {
    acc("vote.near")
}

pub fn voter() -> AccountId {
    acc("voter.near")
}

pub fn reviewer() -> AccountId {
    acc("reviewer.near")
}

pub fn proposer() -> AccountId {
    acc("proposer.near")
}

pub fn owner() -> AccountId {
    acc("owner.near")
}

pub fn council() -> AccountId {
    acc("council.near")
}

pub fn guardian() -> AccountId {
    acc("guardian.near")
}

pub fn for_voter() -> AccountId {
    acc("for-voter.near")
}

pub fn against_voter() -> AccountId {
    acc("against-voter.near")
}

pub fn abstain_voter() -> AccountId {
    acc("abstain-voter.near")
}

pub fn set_ctx(predecessor: AccountId, attached_deposit_yocto: u128, block_ts_ns: u64) {
    let ctx = VMContextBuilder::new()
        .predecessor_account_id(predecessor.clone())
        .signer_account_id(predecessor)
        .attached_deposit(NearToken::from_yoctonear(attached_deposit_yocto))
        .block_timestamp(block_ts_ns)
        .build();
    testing_env!(ctx);
}

/// Production fixed growth config: 6% annual, expressed per nanosecond.
/// `6 / (100 * 365 * 24 * 60 * 60 * 10^9)` rounded to the nearest integer,
/// with denominator `10^30` (matches deployment and satisfies `Contract::new`).
pub fn fixed_rate_growth_config() -> VenearGrowthConfigFixedRate {
    VenearGrowthConfigFixedRate {
        annual_growth_rate_ns: Fraction {
            numerator: U128(1_902_587_519_026),
            denominator: U128(10u128.pow(30)),
        },
    }
}

/// Production-rate growth config used by snapshot fixtures.
pub fn growth_config() -> VenearGrowthConfig {
    VenearGrowthConfig::FixedRate(Box::new(fixed_rate_growth_config()))
}

/// How long before a fixture's snapshot/evaluation time its accounts were last
/// updated. A whole-second window keeps the growth division exact (no rounding
/// panic) while making the production rate move balances by a real, non-zero
/// amount — so tests exercise growth instead of a zero-delta shortcut.
pub const LOCK_AGE_NS: u64 = 86_400 * 1_000_000_000;

#[derive(BorshStorageKey)]
#[near(serializers=[borsh])]
enum FixtureStorageKeys {
    Tree,
}

/// One voter's input to `snapshot_with_voters`.
#[derive(Clone)]
pub struct VoterSpec {
    pub account_id: AccountId,
    pub near_balance: NearToken,
    pub extra: NearToken,
}

impl VoterSpec {
    pub fn new(account_id: AccountId, near_balance: NearToken) -> Self {
        Self {
            account_id,
            near_balance,
            extra: NearToken::from_yoctonear(0),
        }
    }
}

/// Snapshot + per-voter proofs, threaded into voting's `vote()` after `deliver_snapshot`.
pub struct SnapshotFixture {
    pub snapshot: MerkleTreeSnapshot,
    pub vgs: VGlobalState,
    pub proofs: HashMap<AccountId, (MerkleProof, VAccount)>,
}

impl SnapshotFixture {
    pub fn proof_for(&self, account_id: &AccountId) -> (MerkleProof, VAccount) {
        self.proofs
            .get(account_id)
            .cloned()
            .expect("voter missing from fixture")
    }
}

/// Build a `VAccount::V1` whose merkle leaf has the given balance, with
/// `update_timestamp` truncated to whole seconds so `total_balance` succeeds.
pub fn make_v_account(
    account_id: AccountId,
    near_balance: NearToken,
    extra: NearToken,
    at_timestamp_ns: TimestampNs,
) -> VAccount {
    VAccount::V1(Account {
        account_id,
        update_timestamp: at_timestamp_ns,
        balance: VenearBalance {
            near_balance,
            extra_venear_balance: extra,
        },
        delegated_balance: Default::default(),
        delegations: vec![],
    })
}

/// Materialises a merkle tree with the given voters and emits a fixture that
/// includes a `MerkleProof` per voter. `total_venear` is baked into the
/// `VGlobalState` so a later `on_get_snapshot` reproduces it. `ctx_account` is
/// installed as predecessor while the tree is built and the block is advanced.
pub fn snapshot_with_voters(
    voters: &[VoterSpec],
    total_venear: NearToken,
    now_ns: u64,
    ctx_account: AccountId,
) -> SnapshotFixture {
    // Accounts and global state were last updated `LOCK_AGE_NS` before the
    // snapshot is taken, so evaluating balances at `now_ns` applies real growth.
    let eval_ns = now_ns / 1_000_000_000 * 1_000_000_000;
    let lock_ts = TimestampNs::from(eval_ns - LOCK_AGE_NS);
    let total_balance =
        PooledVenearBalance::default().pooled_add(&VenearBalance::from_near(total_venear));
    let global_state = GlobalState {
        update_timestamp: lock_ts,
        total_venear_balance: total_balance,
        venear_growth_config: growth_config(),
    };
    let vgs: VGlobalState = global_state.into();

    set_ctx(ctx_account, 0, now_ns);
    let mut tree: MerkleTree<VAccount, VGlobalState> =
        MerkleTree::new(FixtureStorageKeys::Tree, vgs.clone());

    let mut v_accounts: HashMap<AccountId, VAccount> = HashMap::new();
    for spec in voters {
        let v = make_v_account(
            spec.account_id.clone(),
            spec.near_balance,
            spec.extra,
            lock_ts,
        );
        tree.set(spec.account_id.clone(), v.clone());
        v_accounts.insert(spec.account_id.clone(), v);
    }

    let mut proofs = HashMap::new();
    for spec in voters {
        let (proof, _) = tree.get_proof(&spec.account_id).expect("proof");
        proofs.insert(
            spec.account_id.clone(),
            (proof, v_accounts.remove(&spec.account_id).unwrap()),
        );
    }

    // `get_snapshot` only returns state when the current height differs from the
    // height at which the tree was written, so bump the block before reading.
    testing_env!(VMContextBuilder::new().block_height(1).build());
    let (snapshot, _vgs_again) = tree.get_snapshot().expect("snapshot");

    SnapshotFixture {
        snapshot,
        vgs,
        proofs,
    }
}
