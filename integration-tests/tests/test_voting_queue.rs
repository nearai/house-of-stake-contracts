mod setup;

use crate::setup::voting_helpers::*;
use crate::setup::{NS_IN_SECOND, VenearTestWorkspaceBuilder};
use near_sdk::{Gas, NearToken};
use serde_json::json;
use voting_contract::proposal::{MajorityType, ProposalStatus, VoteOption};

/// Approvals past the active-slot cap should land in `Queued` regardless of flow. Fill the
/// cap=3 with V2 Sandboxes, then verify that both a 4th (Classic) and 5th (V2) approval end up
/// in the pending queue in FIFO order.
#[tokio::test]
async fn test_queue_proposal_queued() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    // Create all proposals (3 V2, 1 Classic, 1 V2).
    let p1 = create_proposal_v2(&v, &user, None).await?;
    let p2 = create_proposal_v2(&v, &user, None).await?;
    let p3 = create_proposal_v2(&v, &user, None).await?;
    let p4 = create_proposal(&v, &user, None).await?;
    let p5 = create_proposal_v2(&v, &user, None).await?;

    // Approve in order — first three fill the active slots, remaining two queue.
    approve_proposal_v2(&v, reviewer, p1, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, p2, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, p3, MajorityType::Simple).await?;
    approve_proposal(&v, reviewer, p4).await?;
    approve_proposal_v2(&v, reviewer, p5, MajorityType::Simple).await?;

    assert_eq!(
        get_status(&v.get_proposal(p1).await?)?,
        ProposalStatus::Sandbox
    );
    assert_eq!(
        get_status(&v.get_proposal(p2).await?)?,
        ProposalStatus::Sandbox
    );
    assert_eq!(
        get_status(&v.get_proposal(p3).await?)?,
        ProposalStatus::Sandbox
    );
    assert_eq!(
        get_status(&v.get_proposal(p4).await?)?,
        ProposalStatus::Queued
    );
    assert_eq!(
        get_status(&v.get_proposal(p5).await?)?,
        ProposalStatus::Queued
    );

    let state = get_queue_state(&v).await?;
    assert_eq!(state.active_proposals.len(), 3);
    assert_eq!(state.pending_queue, vec![p4, p5]);

    Ok(())
}

/// Vetoing a Scheduled/Timelock proposal frees a slot; the next queued proposal is promoted
/// by `reject_proposal`'s auto-tick. Exercises both flows: a V2 queued proposal becomes Sandbox,
/// a Classic queued proposal becomes Voting.
#[tokio::test]
async fn test_queued_promotes_on_veto() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;
    let council = &v.voting.as_ref().unwrap().council;

    let mut ids = Vec::new();
    for _ in 0..4 {
        let id = create_proposal_v2(&v, &user, None).await?;
        approve_proposal_v2(&v, reviewer, id, MajorityType::Simple).await?;
        ids.push(id);
    }
    let classic_p = create_proposal(&v, &user, None).await?;
    approve_proposal(&v, reviewer, classic_p).await?;

    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Queued
    );
    assert_eq!(
        get_status(&v.get_proposal(classic_p).await?)?,
        ProposalStatus::Queued
    );
    assert_eq!(
        get_queue_state(&v).await?.pending_queue,
        vec![ids[3], classic_p]
    );

    vote_for_option(&v, &user, ids[0], VoteOption::For).await?;
    assert_eq!(
        get_status(&v.get_proposal(ids[0]).await?)?,
        ProposalStatus::Scheduled
    );
    let outcome = reject_proposal(&v, council, ids[0]).await?;
    assert!(outcome.is_success(), "Veto should succeed: {:#?}", outcome);

    assert_eq!(
        get_status(&v.get_proposal(ids[0]).await?)?,
        ProposalStatus::Rejected
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Sandbox,
        "V2 queued proposal should be promoted into Sandbox after a slot freed via veto"
    );
    assert_eq!(
        get_status(&v.get_proposal(classic_p).await?)?,
        ProposalStatus::Queued,
        "Classic still waiting — only one slot freed"
    );

    vote_for_option(&v, &user, ids[1], VoteOption::For).await?;
    assert_eq!(
        get_status(&v.get_proposal(ids[1]).await?)?,
        ProposalStatus::Scheduled
    );
    let outcome = reject_proposal(&v, council, ids[1]).await?;
    assert!(
        outcome.is_success(),
        "Second veto should succeed: {:#?}",
        outcome
    );

    assert_eq!(
        get_status(&v.get_proposal(ids[1]).await?)?,
        ProposalStatus::Rejected
    );
    assert_eq!(
        get_status(&v.get_proposal(classic_p).await?)?,
        ProposalStatus::Voting,
        "Classic queued proposal should be promoted into Voting after a slot freed via veto"
    );
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);
    assert_eq!(get_queue_state(&v).await?.pending_queue.len() as u32, 0);

    Ok(())
}

#[tokio::test]
async fn test_queued_promotes_on_sandbox_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let mut ids = Vec::new();
    for _ in 0..4 {
        let id = create_proposal_v2(&v, &user, None).await?;
        approve_proposal_v2(&v, reviewer, id, MajorityType::Simple).await?;
        ids.push(id);
    }
    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Queued
    );

    // Fast-forward past the sandbox deadline. The three already-Sandbox proposals all time out.
    v.fast_forward_to_proposal_status_v2(ids[0], ProposalStatus::Defeated)
        .await?;

    assert_eq!(
        get_status(&v.get_proposal(ids[0]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[1]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[2]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Sandbox,
        "View should already show ids[3] as virtually promoted before advance_queue is called"
    );
    let pre = get_queue_state(&v).await?;
    assert_eq!(pre.active_proposals, vec![ids[3]]);
    assert_eq!(pre.pending_queue.len(), 0);

    // Now actually persist the virtual state via an explicit advance_queue. Anyone can call it.
    let outcome = advance_queue(&v, &user).await?;
    assert!(outcome.is_success(), "advance_queue failed: {:#?}", outcome);

    assert_eq!(
        get_status(&v.get_proposal(ids[0]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[1]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[2]).await?)?,
        ProposalStatus::Defeated
    );
    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Sandbox,
        "Queued proposal should be promoted — its sandbox clock starts at admission, not earlier"
    );
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 1);
    assert_eq!(get_queue_state(&v).await?.pending_queue.len() as u32, 0);

    Ok(())
}

/// Classic proposals in Timelock continue to hold an active slot. Filling all slots with
/// Timelocks must force a newly approved Classic proposal into the queue.
#[tokio::test]
async fn test_classic_timelock_holds_slot() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let mut ids = Vec::new();
    for _ in 0..3 {
        let id = create_proposal(&v, &user, None).await?;
        approve_proposal(&v, reviewer, id).await?;
        ids.push(id);
    }
    for &id in &ids {
        assert_eq!(
            get_status(&v.get_proposal(id).await?)?,
            ProposalStatus::Voting
        );
    }
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);

    // Vote For so each proposal transitions to Timelock (rather than Defeated) at voting end.
    for &id in &ids {
        vote_for_option(&v, &user, id, VoteOption::For).await?;
    }

    // Fast-forward past voting_end. All three transition to Timelock and keep their slots.
    v.fast_forward_to_proposal_status(ids[0], ProposalStatus::Timelock)
        .await?;
    for &id in &ids {
        assert_eq!(
            get_status(&v.get_proposal(id).await?)?,
            ProposalStatus::Timelock
        );
    }
    assert_eq!(
        get_queue_state(&v).await?.active_proposals.len() as u32,
        3,
        "Timelock proposals must still occupy active slots"
    );

    // Approve a fourth Classic proposal. Cap is full of Timelocks — it must queue.
    let id4 = create_proposal(&v, &user, None).await?;
    approve_proposal(&v, reviewer, id4).await?;

    assert_eq!(
        get_status(&v.get_proposal(id4).await?)?,
        ProposalStatus::Queued,
        "4th Classic must queue because Timelock holds a slot"
    );
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);
    assert_eq!(get_queue_state(&v).await?.pending_queue, vec![id4]);

    Ok(())
}

#[tokio::test]
async fn test_classic_mixed_lifecycle_then_more_approvals() -> Result<(), Box<dyn std::error::Error>>
{
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let mut ids = Vec::new();
    for _ in 0..6 {
        let id = create_proposal(&v, &user, None).await?;
        approve_proposal(&v, reviewer, id).await?;
        ids.push(id);
    }
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);
    assert_eq!(get_queue_state(&v).await?.pending_queue.len() as u32, 3);

    // Vote For on ids[0] (will pass quorum + threshold). ids[1] and ids[2] get no votes.
    vote_for_option(&v, &user, ids[0], VoteOption::For).await?;
    v.fast_forward_to_proposal_status(ids[0], ProposalStatus::Succeeded)
        .await?;
    v.fast_forward_to_proposal_status(ids[5], ProposalStatus::Defeated)
        .await?;

    assert_eq!(
        get_status(&v.get_proposal(ids[0]).await?)?,
        ProposalStatus::Succeeded
    );
    for i in 1..6 {
        assert_eq!(
            get_status(&v.get_proposal(ids[i]).await?)?,
            ProposalStatus::Defeated,
            "ids[{i}] should be Defeated"
        );
    }

    let mut new_ids = Vec::new();
    for _ in 0..4 {
        let id = create_proposal(&v, &user, None).await?;
        approve_proposal(&v, reviewer, id).await?;
        new_ids.push(id);
    }

    for i in 0..3 {
        assert_eq!(
            get_status(&v.get_proposal(new_ids[i]).await?)?,
            ProposalStatus::Voting,
            "new_ids[{i}] should be Voting"
        );
    }
    assert_eq!(
        get_status(&v.get_proposal(new_ids[3]).await?)?,
        ProposalStatus::Queued
    );

    let state = get_queue_state(&v).await?;
    assert_eq!(state.active_proposals.len(), 3);
    assert_eq!(state.pending_queue, vec![new_ids[3]]);

    Ok(())
}

#[tokio::test]
async fn test_max_active_proposals_setter() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .max_active_proposals(1)
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;

    let owner = &v.voting.as_ref().unwrap().owner;
    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    // With cap=1, two approved V2 proposals: first active, second queued.
    let p1 = create_proposal_v2(&v, &user, None).await?;
    let p2 = create_proposal_v2(&v, &user, None).await?;

    approve_proposal_v2(&v, reviewer, p1, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, p2, MajorityType::Simple).await?;

    assert_eq!(
        get_status(&v.get_proposal(p1).await?)?,
        ProposalStatus::Sandbox
    );
    assert_eq!(
        get_status(&v.get_proposal(p2).await?)?,
        ProposalStatus::Queued
    );

    // Owner raises the cap. The setter persists the queue advance internally, so p2 is
    // promoted by the same call — no explicit advance_queue is needed.
    let outcome = owner
        .call(v.voting_id(), "set_max_active_proposals")
        .args_json(json!({ "max_active_proposals": 2 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(30))
        .transact()
        .await?;
    assert!(outcome.is_success(), "owner setter failed: {:#?}", outcome);

    assert_eq!(
        get_status(&v.get_proposal(p2).await?)?,
        ProposalStatus::Sandbox,
        "p2 should be promoted by the setter's internal advance_queue"
    );
    let post = get_queue_state(&v).await?;
    assert_eq!(post.active_proposals, vec![p1, p2]);
    assert_eq!(post.pending_queue.len(), 0);

    // Lower the cap back to 1 while both p1 and p2 are still active.
    let outcome = owner
        .call(v.voting_id(), "set_max_active_proposals")
        .args_json(json!({ "max_active_proposals": 1 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(30))
        .transact()
        .await?;
    assert!(outcome.is_success(), "owner setter failed: {:#?}", outcome);

    // Actives unchanged by the cap drop.
    assert_eq!(
        get_queue_state(&v).await?.active_proposals,
        vec![p1, p2],
        "Lowering the cap must not demote active proposals"
    );

    // Approve a third proposal while over-cap. It must queue.
    let p3 = create_proposal_v2(&v, &user, None).await?;
    approve_proposal_v2(&v, reviewer, p3, MajorityType::Simple).await?;
    assert_eq!(
        get_status(&v.get_proposal(p3).await?)?,
        ProposalStatus::Queued,
        "p3 should queue because virtual_active_count (2) >= cap (1)"
    );

    let outcome = advance_queue(&v, &user).await?;
    assert!(outcome.is_success(), "advance_queue failed: {:#?}", outcome);
    assert_eq!(
        get_status(&v.get_proposal(p3).await?)?,
        ProposalStatus::Queued,
        "p3 must remain queued while cap is exceeded by existing actives"
    );
    let post = get_queue_state(&v).await?;
    assert_eq!(post.active_proposals, vec![p1, p2]);
    assert_eq!(post.pending_queue, vec![p3]);

    Ok(())
}

/// `get_approved_proposals` is a historical log of all reviewer-approved proposals. Even Queued
/// proposals (which have an approval but no active slot) must show up there.
#[tokio::test]
async fn test_approved_proposals_view_includes_queued() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .max_active_proposals(1)
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let p1 = create_proposal_v2(&v, &user, None).await?;
    let p2 = create_proposal_v2(&v, &user, None).await?;

    approve_proposal_v2(&v, reviewer, p1, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, p2, MajorityType::Simple).await?;

    let count: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_approved_proposals")
        .await?
        .json()?;
    assert_eq!(
        count, 2,
        "Both approvals should be counted even when one is Queued"
    );

    let approved: serde_json::Value = v
        .sandbox
        .view(v.voting_id(), "get_approved_proposals")
        .args_json(json!({ "from_index": 0u32, "limit": 10u32 }))
        .await?
        .json()?;
    let arr = approved.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    let statuses: Vec<ProposalStatus> = arr
        .iter()
        .map(|p| serde_json::from_value(p["status"].clone()).unwrap())
        .collect();
    assert!(statuses.contains(&ProposalStatus::Sandbox));
    assert!(statuses.contains(&ProposalStatus::Queued));

    Ok(())
}

/// Sanity check that the active set and its order change on promotion.
#[tokio::test]
async fn test_active_proposals_view_reflects_promotion() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .max_active_proposals(1)
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let p1 = create_proposal_v2(&v, &user, None).await?;
    let p2 = create_proposal_v2(&v, &user, None).await?;

    approve_proposal_v2(&v, reviewer, p1, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, p2, MajorityType::Simple).await?;

    let active = get_queue_state(&v).await?.active_proposals;
    assert_eq!(active, vec![p1]);

    // Let the first Sandbox time out, then advance and re-check.
    v.fast_forward_to_proposal_status_v2(p1, ProposalStatus::Defeated)
        .await?;
    let outcome = advance_queue(&v, &user).await?;
    assert!(outcome.is_success());

    let active = get_queue_state(&v).await?.active_proposals;
    assert_eq!(active, vec![p2]);

    Ok(())
}

#[tokio::test]
async fn test_view_virtual_advance() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    v.transfer_and_lock(&user, NearToken::from_near(1000))
        .await?;

    let reviewer = &v.voting.as_ref().unwrap().reviewer;

    let mut ids = Vec::new();
    for _ in 0..6 {
        let id = create_proposal_v2(&v, &user, None).await?;
        approve_proposal_v2(&v, reviewer, id, MajorityType::Simple).await?;
        ids.push(id);
    }

    // -----------------------------------------------------------------------
    // Phase 1: no transitions yet — view shows the stored Queued status.
    // -----------------------------------------------------------------------
    assert_eq!(
        get_status(&v.get_proposal(ids[3]).await?)?,
        ProposalStatus::Queued,
        "No slots are free — view must still show Queued"
    );
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);
    assert_eq!(get_queue_state(&v).await?.pending_queue.len() as u32, 3);

    // -----------------------------------------------------------------------
    // Phase 2: first sandbox batch expires. ids[0..=2] virtually Defeated,
    // ids[3..=5] virtually promoted into Sandbox.
    // -----------------------------------------------------------------------
    v.fast_forward_to_proposal_status_v2(ids[0], ProposalStatus::Defeated)
        .await?;
    for i in 0..3 {
        assert_eq!(
            get_status(&v.get_proposal(ids[i]).await?)?,
            ProposalStatus::Defeated,
            "ids[{i}] should be virtually Defeated"
        );
    }
    for i in 3..6 {
        assert_eq!(
            get_status(&v.get_proposal(ids[i]).await?)?,
            ProposalStatus::Sandbox,
            "ids[{i}] should be virtually promoted to Sandbox"
        );
    }
    assert_eq!(get_queue_state(&v).await?.active_proposals.len() as u32, 3);

    // -----------------------------------------------------------------------
    // Phase 3
    // -----------------------------------------------------------------------
    let proposal = v.get_proposal(ids[0]).await?;
    let approval_time: u64 = proposal["approval_time_ns"].as_str().unwrap().parse()?;
    let sandbox_duration: u64 = proposal["sandbox_duration_ns"].as_str().unwrap().parse()?;
    let target = approval_time + sandbox_duration * 5 / 2;
    let num_blocks = (sandbox_duration * 5 / 2) / NS_IN_SECOND;
    v.fast_forward(target, num_blocks, 20).await?;

    for i in 0..6 {
        assert_eq!(
            get_status(&v.get_proposal(ids[i]).await?)?,
            ProposalStatus::Defeated,
            "ids[{i}] should be Defeated"
        );
    }
    let state = get_queue_state(&v).await?;
    assert_eq!(state.active_proposals.len(), 0);
    assert_eq!(state.pending_queue.len(), 0);

    Ok(())
}
