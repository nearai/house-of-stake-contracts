use crate::proposal::{ProposalAction, ProposalId, ProposalStatus};
use crate::*;
use common::events;
use near_sdk::{Gas, Promise, PromiseResult, ext_contract};

const GAS_FOR_ON_EXECUTE_CALLBACK: Gas = Gas::from_tgas(10);

#[near]
impl Contract {
    /// Executes the on-chain actions for a proposal that has passed timelock.
    /// Can be called by anyone. The proposal must be in `Executable` status.
    pub fn execute_proposal(&mut self, proposal_id: ProposalId) -> Promise {
        self.assert_not_paused();
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        require!(
            proposal.status == ProposalStatus::Executable,
            "Proposal is not in Executable status"
        );

        let actions = proposal.actions.as_ref().unwrap();
        let promise = actions
            .iter()
            .map(|action| match action {
                ProposalAction::FunctionCall {
                    receiver_id,
                    method_name,
                    args,
                    deposit,
                    gas,
                } => Promise::new(receiver_id.clone()).function_call(
                    method_name.clone(),
                    args.0.clone(),
                    *deposit,
                    *gas,
                ),
                ProposalAction::Transfer {
                    receiver_id,
                    amount,
                } => Promise::new(receiver_id.clone()).transfer(*amount),
            })
            .reduce(|acc, p| acc.then(p))
            .unwrap();

        events::emit::execute_proposal_action(&env::predecessor_account_id(), proposal_id);

        proposal.status = ProposalStatus::InProgress;
        self.internal_set_proposal(proposal);

        promise.then(
            ext_execute_self::ext(env::current_account_id())
                .with_static_gas(GAS_FOR_ON_EXECUTE_CALLBACK)
                .on_execute_proposal(proposal_id),
        )
    }

    #[private]
    pub fn on_execute_proposal(&mut self, proposal_id: ProposalId) {
        let mut proposal = self.internal_expect_proposal_updated(proposal_id);

        let all_succeeded = (0..env::promise_results_count())
            .all(|i| !matches!(env::promise_result(i), PromiseResult::Failed));

        proposal.status = if all_succeeded {
            ProposalStatus::Succeeded
        } else {
            ProposalStatus::Failed
        };

        events::emit::execute_proposal_result(proposal_id, all_succeeded);

        self.internal_set_proposal(proposal);
    }
}

#[allow(dead_code)]
#[ext_contract(ext_execute_self)]
trait ExtExecuteSelf {
    fn on_execute_proposal(&mut self, proposal_id: ProposalId);
}
