#![cfg(test)]
//! Fixtures for fast unit tests against `Contract` without a sandbox.
//!
//! The cross-contract surface is small: `ext_venear::get_snapshot()` returns
//! `(MerkleTreeSnapshot, VGlobalState)`, and `on_get_snapshot` writes that into
//! `proposal.snapshot_and_state`. Once that field is set, the rest of the
//! voting logic is purely local — so most tests can seed a `Proposal` with a
//! ready-made `SnapshotAndState` and invoke `vote()` / siblings directly.
//!
//! Public surface kept intentionally small:
//! - `fresh_contract()` — the single contract constructor.
//! - `snapshot_with_voters()` — the single fixture builder.
//! - `create_proposal()` / `approve_proposal()` — composable proposal-lifecycle
//!   steps. `approve_proposal` takes an optional fixture: pass `Some(&f)` to
//!   also deliver the venear snapshot callback, or `None` to stop after
//!   approval (e.g. for queueing or pre-snapshot tests).
//! - `cast_vote()` / `cast_vote_at()` — vote with the standard deposit/context.

use crate::Contract;
use crate::config::Config;
use crate::metadata::ProposalMetadata;
use crate::proposal::{
    MajorityType, ProposalFlow, ProposalId, ProposalStatus, is_active_status,
};
use chrono::{FixedOffset, NaiveDate};
use common::Fraction;
use common::account::{AccountV1, VAccount};
use common::global_state::{GlobalState, VGlobalState};
use common::venear::{VenearGrowthConfig, VenearGrowthConfigFixedRate};
use common::voting::VoteOption;
use common::{Bps, PooledVenearBalance, TimestampNs, VenearBalance};
use merkle_tree::{MerkleProof, MerkleTree, MerkleTreeSnapshot};
use near_sdk::json_types::{U64, U128};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{AccountId, BorshStorageKey, NearToken, near, testing_env};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Constants and named-account helpers
// ---------------------------------------------------------------------------

/// Test default: 2026-06-01 00:00:00 UTC in nanoseconds, truncated to seconds.
pub const TEST_NOW_NS: u64 = 1_780_272_000_000_000_000;

pub fn acc(id: &str) -> AccountId {
    id.parse().unwrap()
}

pub fn current_account() -> AccountId {
    acc("vote.test.near")
}

pub fn voter() -> AccountId {
    acc("voter.test.near")
}

pub fn reviewer() -> AccountId {
    acc("reviewer.test.near")
}

pub fn proposer() -> AccountId {
    acc("proposer.test.near")
}

pub fn owner() -> AccountId {
    acc("owner.test.near")
}

pub fn council() -> AccountId {
    acc("council.test.near")
}

pub fn guardian() -> AccountId {
    acc("guardian.test.near")
}

pub fn for_voter() -> AccountId {
    acc("for-voter.test.near")
}

pub fn against_voter() -> AccountId {
    acc("against-voter.test.near")
}

pub fn abstain_voter() -> AccountId {
    acc("abstain-voter.test.near")
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Install a unit-test `VMContext` (predecessor, attached deposit, block timestamp).
///
/// `#[callback]`-decorated parameters only matter at the WASM entry point. When the callback
/// is invoked directly from Rust (as in unit tests), the tuple is passed as a regular argument
/// — no `PromiseResult` mocking is required.
pub fn set_ctx(predecessor: AccountId, attached_deposit_yocto: u128, block_ts_ns: u64) {
    set_ctx_at_block(predecessor, attached_deposit_yocto, block_ts_ns, 1);
}

fn set_ctx_at_block(
    predecessor: AccountId,
    attached_deposit_yocto: u128,
    block_ts_ns: u64,
    block_height: u64,
) {
    let ctx = VMContextBuilder::new()
        .current_account_id(current_account())
        .predecessor_account_id(predecessor.clone())
        .signer_account_id(predecessor)
        .attached_deposit(NearToken::from_yoctonear(attached_deposit_yocto))
        .block_timestamp(block_ts_ns)
        .block_height(block_height)
        .build();
    testing_env!(ctx);
}

// ---------------------------------------------------------------------------
// Config and contract construction
// ---------------------------------------------------------------------------

/// Mirrors the production defaults applied by `upgrade.rs`. Tests that need
/// different behavior (e.g. disabled expiration, lower sandbox threshold)
/// must call the matching owner-only setter explicitly.
pub fn default_config() -> Config {
    Config {
        venear_account_id: acc("venear.test.near"),
        reviewer_ids: vec![reviewer()],
        council_ids: vec![council()],
        owner_account_id: owner(),
        classic_voting_duration_ns: U64(14 * 24 * 3600 * 1_000_000_000),
        fast_track_voting_duration_ns: U64(5 * 24 * 3600 * 1_000_000_000),
        timelock_duration_ns: U64(14 * 24 * 3600 * 1_000_000_000),
        base_proposal_fee: NearToken::from_near(1),
        bond_amount: NearToken::from_near(100),
        treasury_account_id: acc("treasury.test.near"),
        vote_storage_fee: NearToken::from_millinear(10),
        guardians: vec![guardian()],
        proposal_expiration_ns: U64(7 * 24 * 3600 * 1_000_000_000),
        fast_track_proposal_expiration_ns: U64(2 * 24 * 3600 * 1_000_000_000),
        proposed_new_owner_account_id: None,
        quorum_threshold_bps: Bps::new(3_500),
        quorum_floor: NearToken::from_yoctonear(0),
        approval_threshold_bps: Bps::new(5_000),
        simple_majority_threshold_bps: Bps::new(5_000),
        strong_majority_threshold_bps: Bps::new(6_667),
        sandbox_duration_ns: U64(7 * 24 * 3600 * 1_000_000_000),
        sandbox_threshold_bps: Bps::new(3_000),
        max_active_proposals: 3,
    }
}

/// Zero growth-rate keeps `Account::total_balance` simple in unit tests: it
/// returns `near_balance + extra_venear_balance` regardless of timestamp delta.
pub fn zero_growth_config() -> VenearGrowthConfig {
    VenearGrowthConfig::FixedRate(Box::new(VenearGrowthConfigFixedRate {
        annual_growth_rate_ns: Fraction {
            numerator: U128(0),
            denominator: U128(1_000_000_000_000_000_000_000_000_000_000),
        },
    }))
}

/// The single contract constructor: installs a proposer context at `TEST_NOW_NS`
/// and builds a `Contract` from `default_config()`. Callers override the context
/// afterwards (e.g. `set_ctx(owner(), 1, TEST_NOW_NS)` before owner-only setters).
pub fn fresh_contract() -> Contract {
    set_ctx(proposer(), 0, TEST_NOW_NS);
    Contract::new(default_config())
}

// ---------------------------------------------------------------------------
// Snapshot fixture (single builder)
// ---------------------------------------------------------------------------

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

/// Snapshot + per-voter proofs, threaded into `vote()` after `deliver_snapshot`.
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
    VAccount::V1(AccountV1 {
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

/// The single fixture builder. Materialises a merkle tree with the given voters
/// and emits a fixture that includes a `MerkleProof` per voter.
///
/// `total_venear` is baked into the `VGlobalState` so that `on_get_snapshot`,
/// which reads `total_venear_balance.total()`, reproduces it when this fixture
/// is later fed through the real `create → approve → on_get_snapshot` path.
pub fn snapshot_with_voters(voters: &[VoterSpec], total_venear: NearToken) -> SnapshotFixture {
    let block_ts_ns = TEST_NOW_NS;
    let timestamp_ns = TimestampNs::from(block_ts_ns / 1_000_000_000 * 1_000_000_000);
    let total_balance =
        PooledVenearBalance::default().pooled_add(&VenearBalance::from_near(total_venear));
    let global_state = GlobalState {
        update_timestamp: timestamp_ns,
        total_venear_balance: total_balance,
        venear_growth_config: zero_growth_config(),
    };
    let vgs: VGlobalState = global_state.into();

    set_ctx_at_block(reviewer(), 0, block_ts_ns, 1);
    let mut tree: MerkleTree<VAccount, VGlobalState> =
        MerkleTree::new(FixtureStorageKeys::Tree, vgs.clone());

    let mut v_accounts: HashMap<AccountId, VAccount> = HashMap::new();
    for spec in voters {
        let v = make_v_account(
            spec.account_id.clone(),
            spec.near_balance,
            spec.extra,
            timestamp_ns,
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

    // Advance the block so `get_snapshot()` exposes the just-built state.
    set_ctx_at_block(reviewer(), 0, block_ts_ns, 2);
    let (snapshot, _vgs_again) = tree.get_snapshot().expect("snapshot");

    SnapshotFixture {
        snapshot,
        vgs,
        proofs,
    }
}

// ---------------------------------------------------------------------------
// Proposal lifecycle (composable primitives — chain as needed)
// ---------------------------------------------------------------------------

const PROPOSER_DEPOSIT_NEAR: u8 = 200;

fn proposer_deposit_yocto() -> u128 {
    NearToken::from_near(PROPOSER_DEPOSIT_NEAR.into()).as_yoctonear()
}

/// Creates a proposal from `proposer()` and returns its id. Leaves it in
/// `Created`. Caller resets the context afterwards as needed.
pub fn create_proposal(contract: &mut Contract, flow: ProposalFlow) -> ProposalId {
    let metadata = proposal_metadata("test proposal");
    set_ctx(proposer(), proposer_deposit_yocto(), TEST_NOW_NS);
    contract.create_proposal(metadata, None, flow)
}

/// Approves a `Created` proposal from `reviewer()`. Auto-detects the flow off
/// the stored proposal to pick the right `MajorityType` (None for Classic,
/// `Simple` for FastTrack).
///
/// If `snapshot` is `Some`, also delivers the venear-callback snapshot tuple
/// (`on_get_snapshot`) so the proposal lands fully active. Pass `None` to stop
/// after approval — used for tests that exercise queueing, the
/// "Snapshot has not been taken yet" guard, or the snapshot callback itself.
pub fn approve_proposal(
    contract: &mut Contract,
    id: ProposalId,
    snapshot: Option<&SnapshotFixture>,
) {
    let flow = contract
        .get_proposal(id)
        .expect("proposal exists")
        .proposal
        .flow;
    let majority = match flow {
        ProposalFlow::Classic => None,
        ProposalFlow::FastTrack => Some(MajorityType::Simple),
    };
    set_ctx(reviewer(), 1, TEST_NOW_NS);
    let _ = contract.approve_proposal(id, majority);
    if let Some(fixture) = snapshot {
        set_ctx(current_account(), 0, TEST_NOW_NS);
        contract.on_get_snapshot((fixture.snapshot.clone(), fixture.vgs.clone()), id);
    }
}

/// Builds a `ProposalMetadata` with the given title and no description/link.
pub fn proposal_metadata(title: &str) -> ProposalMetadata {
    ProposalMetadata {
        title: Some(title.to_string()),
        description: None,
        link: None,
    }
}

/// Yocto deposit that covers the maximum required across classic + fasttrack
/// `create_proposal` paths (base_proposal_fee + bond + headroom).
pub fn over_deposit_yocto() -> u128 {
    NearToken::from_near(200).as_yoctonear()
}

// ---------------------------------------------------------------------------
// Vote helpers
// ---------------------------------------------------------------------------

/// Standard vote-storage deposit (10 millinear). Matches `default_config()`'s
/// `vote_storage_fee`.
pub fn vote_deposit_yocto() -> u128 {
    NearToken::from_millinear(10).as_yoctonear()
}

/// Sets up the caller context and casts a vote at `TEST_NOW_NS`.
pub fn cast_vote(
    contract: &mut Contract,
    fixture: &SnapshotFixture,
    voter_id: AccountId,
    proposal_id: ProposalId,
    option: VoteOption,
) {
    cast_vote_at(
        contract,
        fixture,
        voter_id,
        proposal_id,
        option,
        TEST_NOW_NS,
    );
}

/// Sets up the caller context and casts a vote at an explicit block timestamp.
pub fn cast_vote_at(
    contract: &mut Contract,
    fixture: &SnapshotFixture,
    voter_id: AccountId,
    proposal_id: ProposalId,
    option: VoteOption,
    at_ns: u64,
) {
    let (proof, v_account) = fixture.proof_for(&voter_id);
    set_ctx(voter_id, vote_deposit_yocto(), at_ns);
    contract.vote(proposal_id, option, proof, v_account);
}

// ---------------------------------------------------------------------------
// Status / queue assertions
// ---------------------------------------------------------------------------

/// Reads the proposal status as it would appear at the given block time.
/// `get_proposal()` calls `update()` internally, so this lets tests probe
/// the time-boundary arms of `update_classic` / `update_fast_track`.
pub fn status_at(contract: &Contract, id: ProposalId, at_ns: u64) -> ProposalStatus {
    set_ctx(voter(), 0, at_ns);
    contract.get_proposal(id).expect("proposal").proposal.status
}

/// Reads every proposal's status as it would appear at the given block time,
/// in proposal-id (creation) order. Uses `get_proposals` so virtual queue
/// promotions are reflected.
pub fn all_statuses_at(contract: &Contract, at_ns: u64) -> Vec<ProposalStatus> {
    set_ctx(voter(), 0, at_ns);
    contract
        .get_proposals(0, None)
        .into_iter()
        .map(|p| p.proposal.status)
        .collect()
}

/// Asserts a proposal is both in `active_proposals` AND has the expected
/// lifecycle-active stored status. Catches drift between the active set and
/// the proposal's stored status.
pub fn assert_active_with_status(contract: &Contract, id: ProposalId, expected: ProposalStatus) {
    assert!(
        is_active_status(expected),
        "expected status {:?} is not a lifecycle-active status — use direct status comparison instead",
        expected
    );
    let state = contract.get_queue_state();
    assert!(
        state.active_proposals.contains(&id),
        "proposal {} missing from active_proposals (got {:?})",
        id,
        state.active_proposals
    );
    let actual = contract
        .get_proposal(id)
        .expect("proposal exists")
        .proposal
        .status;
    assert_eq!(
        actual, expected,
        "proposal {} active set OK but status mismatch",
        id
    );
}

/// Asserts a proposal is in the pending queue at the given position with
/// stored status `Queued` (the only valid status for a pending-queue member).
pub fn assert_queued_at(contract: &Contract, id: ProposalId, position: usize) {
    let state = contract.get_queue_state();
    assert_eq!(
        state.pending_queue.get(position).copied(),
        Some(id),
        "expected proposal {} at queue position {}, got {:?}",
        id,
        position,
        state.pending_queue
    );
    let actual = contract
        .get_proposal(id)
        .expect("proposal exists")
        .proposal
        .status;
    assert_eq!(
        actual,
        ProposalStatus::Queued,
        "proposal {} in pending queue but stored status is {:?}",
        id,
        actual
    );
    assert!(
        !state.active_proposals.contains(&id),
        "proposal {} both queued and active",
        id
    );
}

// ---------------------------------------------------------------------------
// Date math
// ---------------------------------------------------------------------------

/// Returns y-m-d 00:00 CET (fixed UTC+1) as UTC nanoseconds. Used by date-math
/// unit tests for `next_voting_start_ns`.
pub fn date_ns(year: i32, month: u32, day: u32) -> u64 {
    let cet = FixedOffset::east_opt(3600).unwrap();
    u64::try_from(
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(cet)
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap(),
    )
    .unwrap()
}
