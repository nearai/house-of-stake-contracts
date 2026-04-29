use crate::config::Config;
use crate::proposal::{is_active_status, Proposal, ProposalFlow, ProposalStatus, VProposal};
use crate::*;
use near_sdk::borsh::{self, BorshDeserialize};
use near_sdk::json_types::U64;
use near_sdk::store::{IterableSet, LookupMap, Vector};
use near_sdk::Gas;

const MIGRATE_STATE_GAS: Gas = Gas::from_tgas(50);
const GET_CONFIG_GAS: Gas = Gas::from_tgas(5);

// Defaults applied when migrating a contract that predates the merged flow.
const DEFAULT_BOND_AMOUNT_NEAR: u128 = 10;
const DEFAULT_SIMPLE_MAJORITY_BPS: u16 = 5_000;
const DEFAULT_STRONG_MAJORITY_BPS: u16 = 6_667;
const DEFAULT_SANDBOX_DURATION_NS: u64 = 14 * 24 * 60 * 60 * 1_000_000_000; // 14 days
const DEFAULT_SANDBOX_THRESHOLD_BPS: u16 = 3_000;
const DEFAULT_MAX_ACTIVE_PROPOSALS: u32 = 3;

/// Config from v1.0.3 (pre-merge). No v2 fields.
#[derive(Clone, BorshDeserialize, near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "borsh")]
struct OldConfig {
    venear_account_id: AccountId,
    reviewer_ids: Vec<AccountId>,
    council_ids: Vec<AccountId>,
    owner_account_id: AccountId,
    voting_duration_ns: U64,
    timelock_duration_ns: U64,
    base_proposal_fee: NearToken,
    vote_storage_fee: NearToken,
    guardians: Vec<AccountId>,
    proposal_expiration_ns: U64,
    proposed_new_owner_account_id: Option<AccountId>,
    quorum_threshold_bps: u16,
    quorum_floor: NearToken,
    approval_threshold_bps: u16,
}

/// Legacy classic-flow proposal shape stored under `VProposal::Current` at v1.0.3.
#[derive(Clone, BorshDeserialize, near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "borsh")]
struct LegacyProposalClassic {
    id: ProposalId,
    creation_time_ns: U64,
    proposer_id: AccountId,
    reviewer_id: Option<AccountId>,
    rejecter_id: Option<AccountId>,
    voting_start_time_ns: Option<U64>,
    voting_duration_ns: U64,
    timelock_duration_ns: U64,
    expiration_ns: U64,
    snapshot_and_state: Option<crate::proposal::SnapshotAndState>,
    votes: Vec<crate::proposal::VoteStats>,
    total_votes: crate::proposal::VoteStats,
    status: ProposalStatus,
    quorum_threshold_bps: u16,
    quorum_floor: NearToken,
    approval_threshold_bps: u16,
    actions: Option<Vec<crate::proposal::ProposalAction>>,
}

/// Legacy pre-v1.0.3 proposal shape, if any are still tagged as `V1` in storage.
#[derive(Clone, BorshDeserialize, near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "borsh")]
struct LegacyProposalV1 {
    id: ProposalId,
    creation_time_ns: U64,
    proposer_id: AccountId,
    reviewer_id: Option<AccountId>,
    voting_start_time_ns: Option<U64>,
    voting_duration_ns: U64,
    rejected: bool,
    snapshot_and_state: Option<crate::proposal::SnapshotAndState>,
    votes: Vec<crate::proposal::VoteStats>,
    total_votes: crate::proposal::VoteStats,
    status: ProposalStatus,
}

/// Legacy `VProposal` envelope as it was encoded at v1.0.3.
#[derive(Clone, BorshDeserialize, near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "borsh")]
enum LegacyVProposal {
    V1(LegacyProposalV1),
    Current(LegacyProposalClassic),
}

impl From<LegacyProposalClassic> for Proposal {
    fn from(c: LegacyProposalClassic) -> Self {
        Self {
            id: c.id,
            creation_time_ns: c.creation_time_ns,
            proposer_id: c.proposer_id,
            reviewer_id: c.reviewer_id,
            rejecter_id: c.rejecter_id,
            approval_time_ns: None,
            voting_start_time_ns: c.voting_start_time_ns,
            voting_duration_ns: c.voting_duration_ns,
            timelock_duration_ns: c.timelock_duration_ns,
            expiration_ns: c.expiration_ns,
            snapshot_and_state: c.snapshot_and_state,
            votes: c.votes,
            total_votes: c.total_votes,
            status: c.status,
            quorum_threshold_bps: c.quorum_threshold_bps,
            quorum_floor: c.quorum_floor,
            approval_threshold_bps: c.approval_threshold_bps,
            actions: c.actions,
            bond_amount: NearToken::from_yoctonear(0),
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: 0,
            flow: ProposalFlow::Classic,
        }
    }
}

impl From<LegacyProposalV1> for Proposal {
    fn from(v1: LegacyProposalV1) -> Self {
        Self {
            id: v1.id,
            creation_time_ns: v1.creation_time_ns,
            proposer_id: v1.proposer_id,
            reviewer_id: v1.reviewer_id,
            rejecter_id: None,
            approval_time_ns: None,
            voting_start_time_ns: v1.voting_start_time_ns,
            voting_duration_ns: v1.voting_duration_ns,
            timelock_duration_ns: U64(0),
            expiration_ns: U64(0),
            snapshot_and_state: v1.snapshot_and_state,
            votes: v1.votes,
            total_votes: v1.total_votes,
            status: v1.status,
            quorum_threshold_bps: 0,
            quorum_floor: NearToken::from_yoctonear(0),
            approval_threshold_bps: 0,
            actions: None,
            bond_amount: NearToken::from_yoctonear(0),
            sandbox_duration_ns: U64(0),
            sandbox_threshold_bps: 0,
            flow: ProposalFlow::Classic,
        }
    }
}

impl From<LegacyVProposal> for Proposal {
    fn from(v: LegacyVProposal) -> Self {
        match v {
            LegacyVProposal::V1(v1) => v1.into(),
            LegacyVProposal::Current(c) => c.into(),
        }
    }
}

/// Contract state from v1.0.3 (pre-merge). Proposals are tagged with
/// `LegacyVProposal` (`V1` | `Current(LegacyProposalClassic)`).
#[derive(BorshDeserialize)]
#[borsh(crate = "borsh")]
struct OldContract {
    config: OldConfig,
    proposals: Vector<LegacyVProposal>,
    proposal_metadata: Vector<VProposalMetadata>,
    votes: LookupMap<(AccountId, ProposalId), u8>,
    approved_proposals: Vector<ProposalId>,
    paused: bool,
}

#[near]
impl Contract {
    /// Private method to migrate the contract state from v1.0.3 (pre-merge) to the merged flow.
    /// Rewrites every legacy proposal into the unified `Proposal` shape with `flow = Classic`.
    #[private]
    #[init(ignore_state)]
    pub fn migrate_state() -> Self {
        let mut old: OldContract = env::state_read().unwrap();

        let mut proposals = Vector::new(StorageKeys::Proposal);
        let mut active_proposals = IterableSet::new(StorageKeys::ActiveProposals);
        for (idx, legacy) in old.proposals.iter().enumerate() {
            let proposal: Proposal = legacy.clone().into();
            if is_active_status(proposal.status) {
                active_proposals.insert(idx as ProposalId);
            }
            proposals.push(VProposal::Current(proposal));
        }

        // The merged flow no longer tracks `approved_proposals` separately; release its storage.
        old.approved_proposals.clear();

        Self {
            config: Config {
                venear_account_id: old.config.venear_account_id,
                reviewer_ids: old.config.reviewer_ids,
                council_ids: old.config.council_ids,
                owner_account_id: old.config.owner_account_id,
                voting_duration_ns: old.config.voting_duration_ns,
                timelock_duration_ns: old.config.timelock_duration_ns,
                base_proposal_fee: old.config.base_proposal_fee,
                bond_amount: NearToken::from_near(DEFAULT_BOND_AMOUNT_NEAR),
                vote_storage_fee: old.config.vote_storage_fee,
                guardians: old.config.guardians,
                proposal_expiration_ns: old.config.proposal_expiration_ns,
                proposed_new_owner_account_id: old.config.proposed_new_owner_account_id,
                quorum_threshold_bps: old.config.quorum_threshold_bps,
                quorum_floor: old.config.quorum_floor,
                approval_threshold_bps: old.config.approval_threshold_bps,
                simple_majority_threshold_bps: DEFAULT_SIMPLE_MAJORITY_BPS,
                strong_majority_threshold_bps: DEFAULT_STRONG_MAJORITY_BPS,
                sandbox_duration_ns: U64(DEFAULT_SANDBOX_DURATION_NS),
                sandbox_threshold_bps: DEFAULT_SANDBOX_THRESHOLD_BPS,
                max_active_proposals: DEFAULT_MAX_ACTIVE_PROPOSALS,
            },
            proposals,
            proposal_metadata: old.proposal_metadata,
            votes: old.votes,
            paused: old.paused,
            pending_queue: Vector::new(StorageKeys::PendingQueue),
            active_proposals,
        }
    }

    /// Returns the version of the contract from the Cargo.toml.
    pub fn get_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

/// Upgrades the contract to the new version.
/// Requires the method to be called by the owner.
/// The input is the new contract code.
/// The contract will call `migrate_state` method on the new contract and then return the config,
/// to verify that the migration was successful.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn upgrade() {
    env::setup_panic_hook();
    let contract: Contract = env::state_read().unwrap();
    contract.assert_owner();
    let current_account_id = env::current_account_id();
    let current_account_id = current_account_id.as_str();
    let migrate_method_name = b"migrate_state".to_vec();
    let get_config_method_name = b"get_config".to_vec();
    let empty_args = b"{}".to_vec();
    unsafe {
        sys::input(0);
        let promise_id = sys::promise_batch_create(
            current_account_id.len() as _,
            current_account_id.as_ptr() as _,
        );
        sys::promise_batch_action_deploy_contract(promise_id, u64::MAX as _, 0);

        // Scheduling state migration.
        sys::promise_batch_action_function_call_weight(
            promise_id,
            migrate_method_name.len() as _,
            migrate_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            MIGRATE_STATE_GAS.as_gas(),
            1,
        );
        // Scheduling to return a config after the migration is completed.
        // It's an extra safety guard for the remote contract upgrades.
        sys::promise_batch_action_function_call(
            promise_id,
            get_config_method_name.len() as _,
            get_config_method_name.as_ptr() as _,
            empty_args.len() as _,
            empty_args.as_ptr() as _,
            0 as _,
            GET_CONFIG_GAS.as_gas(),
        );
        sys::promise_return(promise_id);
    }
}
