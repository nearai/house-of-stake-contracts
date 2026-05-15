mod bond;
mod config;
mod execute;
mod governance;
mod metadata;
mod pause;
pub mod proposal;
pub mod queue;
mod reviewer;
mod upgrade;
mod votes;

use merkle_tree::{MerkleProof, MerkleTreeSnapshot};

use crate::config::Config;
use crate::metadata::VProposalMetadata;
use crate::proposal::{ProposalId, VProposal, VoteOption};
use common::account::*;
use common::venear::VenearGrowthConfig;
use near_sdk::store::{IterableSet, LookupMap, Vector};
use near_sdk::{
    AccountId, BorshStorageKey, NearToken, PanicOnDefault, env, ext_contract, near, require,
};

#[allow(dead_code)]
#[ext_contract(ext_venear)]
pub(crate) trait ExtVenear {
    fn get_snapshot(&self);
}

#[allow(dead_code)]
#[ext_contract(ext_self)]
pub(crate) trait ExtSelf {
    fn on_get_snapshot(&mut self, proposal_id: ProposalId);
    fn vote(
        &mut self,
        proposal_id: ProposalId,
        vote: VoteOption,
        merkle_proof: MerkleProof,
        v_account: VAccount,
    );
}

#[derive(BorshStorageKey)]
#[near(serializers=[borsh(use_discriminant = true)])]
enum StorageKeys {
    Proposal = 0,
    ProposalMetadata = 1,
    Votes = 2,
    // ApprovedProposals = 3,
    PendingQueue = 4,
    ActiveProposals = 5,
}

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct Contract {
    config: Config,
    proposals: Vector<VProposal>,
    proposal_metadata: Vector<VProposalMetadata>,
    /// A map from the account ID and the proposal ID to the vote option index.
    votes: LookupMap<(AccountId, ProposalId), u8>,
    /// A flag indicating whether the contract is paused.
    /// The paused contract will not accept new proposals, new votes or updated votes, proposals
    /// cannot be approved or rejected.
    paused: bool,
    /// Approved proposals that are waiting for a slot.
    pending_queue: Vector<ProposalId>,
    /// Set of proposals currently occupying an active slot.
    active_proposals: IterableSet<ProposalId>,
}

#[near]
impl Contract {
    /// Initializes the contract with the given configuration.
    #[init]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            proposals: Vector::new(StorageKeys::Proposal),
            proposal_metadata: Vector::new(StorageKeys::ProposalMetadata),
            votes: LookupMap::new(StorageKeys::Votes),
            paused: false,
            pending_queue: Vector::new(StorageKeys::PendingQueue),
            active_proposals: IterableSet::new(StorageKeys::ActiveProposals),
        }
    }
}
