use crate::proposal::{Proposal, is_active_status};
use crate::*;
use std::collections::{HashMap, VecDeque};

struct QueueAdvanceOutcome {
    active_updates: Vec<Proposal>,
    queue_promotions: Vec<Proposal>,
}

/// Snapshot of the proposal scheduler: currently-active proposals and the FIFO pending queue.
#[near(serializers=[json])]
#[derive(Debug, PartialEq, Eq)]
pub struct QueueState {
    pub active_proposals: Vec<ProposalId>,
    pub pending_queue: Vec<ProposalId>,
}

#[near]
impl Contract {
    pub fn advance_queue(&mut self) {
        self.assert_not_paused();
        self.internal_advance_queue();
    }

    /// Returns the active proposal ids and the pending-queue ids (front first).
    pub fn get_queue_state(&self) -> QueueState {
        let outcome = self.compute_queue_advance();

        let exiting: Vec<ProposalId> = outcome
            .active_updates
            .iter()
            .filter(|p| !is_active_status(p.status))
            .map(|p| p.id)
            .collect();

        let mut active_proposals: Vec<ProposalId> = self
            .active_proposals
            .iter()
            .copied()
            .filter(|id| !exiting.contains(id))
            .collect();
        active_proposals.extend(
            outcome
                .queue_promotions
                .iter()
                .filter(|p| is_active_status(p.status))
                .map(|p| p.id),
        );

        let pending_queue: Vec<ProposalId> = self
            .pending_queue
            .iter()
            .copied()
            .skip(outcome.queue_promotions.len())
            .collect();

        QueueState {
            active_proposals,
            pending_queue,
        }
    }
}

impl Contract {
    fn compute_queue_advance(&self) -> QueueAdvanceOutcome {
        let now = env::block_timestamp();
        let mut active_updates = Vec::new();
        let mut queue_promotions = Vec::new();
        let mut virtual_active_count = 0u32;
        let mut freed_slot_times: VecDeque<u64> = VecDeque::new();

        for &id in self.active_proposals.iter() {
            let (proposal, updated) = self
                .internal_get_proposal(id)
                .expect("active proposal missing");
            if is_active_status(proposal.status) {
                virtual_active_count += 1;
            }
            if updated {
                if !is_active_status(proposal.status) {
                    let end_time = proposal.active_end_time_ns();
                    let pos = freed_slot_times
                        .iter()
                        .position(|&t| t >= end_time)
                        .unwrap_or(freed_slot_times.len());
                    freed_slot_times.insert(pos, end_time);
                }
                active_updates.push(proposal);
            }
        }

        for &id in self.pending_queue.iter() {
            if virtual_active_count >= self.config.max_active_proposals {
                break;
            }

            let mut proposal = self.internal_expect_proposal_updated(id);

            // Pre-existing empty slots start at `now`.
            let start_time = freed_slot_times.pop_front().unwrap_or(now).into();

            proposal.activate(start_time);
            proposal.update(now.into());

            if is_active_status(proposal.status) {
                virtual_active_count += 1;
            } else {
                let end = proposal.active_end_time_ns();
                let pos = freed_slot_times
                    .iter()
                    .position(|&t| t >= end)
                    .unwrap_or(freed_slot_times.len());
                freed_slot_times.insert(pos, end);
            }

            queue_promotions.push(proposal);
        }

        QueueAdvanceOutcome {
            active_updates,
            queue_promotions,
        }
    }

    pub(crate) fn internal_advance_queue(&mut self) {
        let outcome = self.compute_queue_advance();

        self.pending_queue
            .drain(0..outcome.queue_promotions.len() as u32);

        for proposal in outcome.queue_promotions {
            self.internal_set_proposal(proposal);
        }
        for proposal in outcome.active_updates {
            self.internal_set_proposal(proposal);
        }
    }

    pub(crate) fn get_proposals_virtual_updates(&self) -> HashMap<ProposalId, Proposal> {
        self.compute_queue_advance()
            .queue_promotions
            .into_iter()
            .map(|p| (p.id, p))
            .collect()
    }
}
