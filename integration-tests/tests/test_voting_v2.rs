mod setup;

use crate::setup::voting_helpers::*;
use crate::setup::{DEFAULT_BOND_AMOUNT, NS_IN_SECOND, VenearTestWorkspaceBuilder};
use near_sdk::{Gas, NearToken};
use near_workspaces::AccountId;
use serde_json::json;
use voting_contract::proposal::{MajorityType, ProposalStatus, VoteOption};

#[tokio::test]
async fn test_voting_v2() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    // Lock NEAR so users have enough veNEAR to meet quorum floor
    v.transfer_and_lock(&user_a, NearToken::from_near(1000))
        .await?;

    let num_proposals: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_proposals")
        .await?
        .json()?;
    assert_eq!(num_proposals, 0);

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    let num_proposals: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_proposals")
        .await?
        .json()?;
    assert_eq!(num_proposals, 1);
    let num_approved_proposals: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_approved_proposals")
        .await?
        .json()?;
    assert_eq!(num_approved_proposals, 0);

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(get_status(&proposal)?, ProposalStatus::Created);

    // Bond should be set after creation
    assert_ne!(
        proposal["bond_amount"].as_str().unwrap(),
        "0",
        "bond_amount should be non-zero after creation"
    );

    assert!(
        approve_proposal_v2(&v, &user_a, proposal_id, MajorityType::Simple)
            .await
            .is_err(),
        "Regular user should not be able to approve the proposal"
    );

    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(get_status(&proposal)?, ProposalStatus::Sandbox);
    assert_eq!(
        proposal["reviewer_id"].as_str().unwrap(),
        v.voting.as_ref().unwrap().reviewer.id().as_str()
    );
    // Bond stays locked through approval; only refundable from Expired via claim_bond.
    assert_eq!(
        proposal["bond_amount"].as_str().unwrap(),
        DEFAULT_BOND_AMOUNT.as_yoctonear().to_string(),
        "bond_amount should stay locked after approval"
    );
    let num_proposals: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_proposals")
        .await?
        .json()?;
    assert_eq!(num_proposals, 1);
    let num_approved_proposals: u32 = v
        .sandbox
        .view(v.voting_id(), "get_num_approved_proposals")
        .await?
        .json()?;
    assert_eq!(num_approved_proposals, 1);

    let (user_a_merkle_proof, user_a_v_account): (serde_json::Value, serde_json::Value) = v
        .sandbox
        .view(v.venear.id(), "get_proof")
        .args_json(json!({
            "account_id": user_a.id(),
        }))
        .await?
        .json()?;

    let (user_b_merkle_proof, user_b_v_account): (serde_json::Value, serde_json::Value) = v
        .sandbox
        .view(v.venear.id(), "get_proof")
        .args_json(json!({
            "account_id": user_b.id(),
        }))
        .await?
        .json()?;

    let user_c = v.create_account_with_lockup().await?;

    let (user_c_merkle_proof, user_c_v_account): (serde_json::Value, serde_json::Value) = v
        .sandbox
        .view(v.venear.id(), "get_proof")
        .args_json(json!({
            "account_id": user_c.id(),
        }))
        .await?
        .json()?;

    // Sandbox: Against and Abstain votes should be rejected
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Against,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Against vote should fail during Sandbox"
    );

    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Abstain,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Abstain vote should fail during Sandbox"
    );

    // Vote For during Sandbox — this graduates to Voting
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::For,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "user_a: Failed to vote For: {:#?}",
        outcome
    );

    // Should have been scheduled after meeting 30% threshold
    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Scheduled,
        "Should be Scheduled after meeting 30% threshold"
    );
    // For votes from Sandbox carry over
    assert_eq!(
        proposal["votes"][0]["total_votes"].as_u64().unwrap(),
        1,
        "For vote from Sandbox should carry over"
    );

    // Fast-forward past scheduled period so voting starts
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Voting)
        .await?;
    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Voting,);

    // Change vote to Against (now in Voting, change is allowed)
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Against,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "user_a: Failed to change vote: {:#?}",
        outcome
    );

    let vote: Option<u8> = v
        .sandbox
        .view(v.voting_id(), "get_vote")
        .args_json(json!({
            "account_id": user_a.id(),
            "proposal_id": proposal_id,
        }))
        .await?
        .json()?;
    assert_eq!(vote, Some(1));

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["votes"][0]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["votes"][1]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["votes"][2]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 1);

    // Attempt to vote with an invalid proof
    let outcome = user_b
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Abstain,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_b_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_b: Voted with invalid proof: {:#?}",
        outcome
    );

    // Attempt to vote from the different account
    let outcome = user_c
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Abstain,
            "merkle_proof": user_b_merkle_proof,
            "v_account": user_b_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_c: Voted for account user_b: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["votes"][0]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["votes"][1]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["votes"][2]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 1);

    // Valid vote from user_b
    let outcome = user_b
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Abstain,
            "merkle_proof": user_b_merkle_proof,
            "v_account": user_b_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "user_b: Failed to vote: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["votes"][0]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["votes"][1]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["votes"][2]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 2);

    // Attempt to vote from user_c with different root
    let outcome = user_c
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::For,
            "merkle_proof": user_c_merkle_proof,
            "v_account": user_c_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_c: Voted after snapshot: {:#?}",
        outcome
    );

    // Changing vote from user_a
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::For,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "user_a: Failed to change vote: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["votes"][0]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["votes"][1]["total_votes"].as_u64().unwrap(), 0);
    assert_eq!(proposal["votes"][2]["total_votes"].as_u64().unwrap(), 1);
    assert_eq!(proposal["total_votes"]["total_votes"].as_u64().unwrap(), 2);

    // Fast forward past voting end
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Succeeded)
        .await?;
    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Succeeded);

    Ok(())
}

#[tokio::test]
async fn test_voting_v2_reject_proposal() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(1000))
        .await?;

    let proposal_id_1 = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id_1,
        MajorityType::Simple,
    )
    .await?;

    // Vote For to move from Sandbox to Scheduled
    vote_for_option(&v, &user_a, proposal_id_1, VoteOption::For).await?;

    // Fast forward to Voting status
    v.fast_forward_to_proposal_status_v2(proposal_id_1, ProposalStatus::Voting)
        .await?;

    let proposal = v.get_proposal(proposal_id_1).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Voting,
        "Proposal 1 should be in Voting status"
    );

    // Regular user cannot reject during voting
    let outcome = user_a
        .call(v.voting_id(), "reject_proposal")
        .args_json(json!({
            "proposal_id": proposal_id_1,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(250))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "User should not be able to reject proposal: {:#?}",
        outcome
    );

    // Reviewer cannot reject proposals
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .reviewer
        .call(v.voting_id(), "reject_proposal")
        .args_json(json!({
            "proposal_id": proposal_id_1,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(250))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Reviewer should not be able to reject proposal: {:#?}",
        outcome
    );

    // Council can reject (veto) during voting
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .council
        .call(v.voting_id(), "reject_proposal")
        .args_json(json!({
            "proposal_id": proposal_id_1,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(250))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Council should be able to reject proposal during voting: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id_1).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Rejected);
    assert_eq!(
        proposal["rejecter_id"].as_str().unwrap(),
        v.voting.as_ref().unwrap().council.id().as_str(),
        "rejecter_id should be set to the council member"
    );

    let proposal_id_2 = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id_2,
        MajorityType::Simple,
    )
    .await?;

    // Vote For to move from Sandbox to Scheduled
    vote_for_option(&v, &user_a, proposal_id_2, VoteOption::For).await?;

    let proposal = v.get_proposal(proposal_id_2).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Scheduled,
        "Proposal 2 should be in Scheduled status"
    );

    // Council can reject (veto) during scheduled period
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .council
        .call(v.voting_id(), "reject_proposal")
        .args_json(json!({
            "proposal_id": proposal_id_2,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(250))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Council should be able to reject proposal during scheduled period: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id_2).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Rejected);
    assert_eq!(
        proposal["rejecter_id"].as_str().unwrap(),
        v.voting.as_ref().unwrap().council.id().as_str(),
        "rejecter_id should be set to the council member"
    );

    Ok(())
}

#[tokio::test]
async fn test_voting_v2_governance() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    let voting_owner = v.voting.as_ref().unwrap().owner.clone();

    let original_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;

    let original_venear_account_id: AccountId =
        serde_json::from_value(original_config["venear_account_id"].clone())?;
    let new_venear_account_id: AccountId = "new_venear_account_id".parse()?;
    assert_ne!(original_venear_account_id, new_venear_account_id);

    // Attempt to change config with a regular user
    let outcome = user
        .call(v.voting_id(), "set_venear_account_id")
        .args_json(json!({
            "venear_account_id": new_venear_account_id,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Regular user should not be able to change config: {:#?}",
        outcome
    );

    // Change config with the owner
    let outcome = voting_owner
        .call(v.voting_id(), "set_venear_account_id")
        .args_json(json!({
            "venear_account_id": new_venear_account_id,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to change config: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let venear_account_id: AccountId =
        serde_json::from_value(new_config["venear_account_id"].clone())?;
    assert_eq!(venear_account_id, new_venear_account_id);

    // Reviewers
    let original_reviewer_ids: Vec<AccountId> =
        serde_json::from_value(original_config["reviewer_ids"].clone())?;
    let new_reviewer_ids: Vec<AccountId> =
        vec!["new_reviewer_1".parse()?, "new_reviewer_2".parse()?];
    assert_ne!(original_reviewer_ids, new_reviewer_ids);

    // Attempt to change config with a regular user
    let outcome = user
        .call(v.voting_id(), "set_reviewer_ids")
        .args_json(json!({
            "reviewer_ids": new_reviewer_ids,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Regular user should not be able to change config: {:#?}",
        outcome
    );

    // Change config with the owner
    let outcome = voting_owner
        .call(v.voting_id(), "set_reviewer_ids")
        .args_json(json!({
            "reviewer_ids": new_reviewer_ids,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to change config: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let reviewer_ids: Vec<AccountId> = serde_json::from_value(new_config["reviewer_ids"].clone())?;
    assert_eq!(reviewer_ids, new_reviewer_ids);

    // Voting duration
    let original_voting_duration_ns: near_sdk::json_types::U64 =
        serde_json::from_value(original_config["voting_duration_ns"].clone())?;
    let new_voting_duration_sec: u32 = 1000;
    let new_voting_duration_ns: near_sdk::json_types::U64 =
        (new_voting_duration_sec as u64 * 10u64.pow(9)).into();
    assert_ne!(original_voting_duration_ns, new_voting_duration_ns);

    // Attempt to change config with a regular user
    let outcome = user
        .call(v.voting_id(), "set_voting_duration")
        .args_json(json!({
            "voting_duration_sec": new_voting_duration_sec,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Regular user should not be able to change config: {:#?}",
        outcome
    );

    // Change config with the owner
    let outcome = voting_owner
        .call(v.voting_id(), "set_voting_duration")
        .args_json(json!({
            "voting_duration_sec": new_voting_duration_sec,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to change config: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let voting_duration_ns: near_sdk::json_types::U64 =
        serde_json::from_value(new_config["voting_duration_ns"].clone())?;
    assert_eq!(voting_duration_ns, new_voting_duration_ns);

    // Bond amount
    let original_bond_amount: NearToken =
        serde_json::from_value(original_config["bond_amount"].clone())?;
    let new_bond_amount: NearToken = NearToken::from_near(2);
    assert_ne!(original_bond_amount, new_bond_amount);

    // Attempt to change config with a regular user
    let outcome = user
        .call(v.voting_id(), "set_bond_amount")
        .args_json(json!({
            "bond_amount": new_bond_amount,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Regular user should not be able to change config: {:#?}",
        outcome
    );

    // Change config with the owner
    let outcome = voting_owner
        .call(v.voting_id(), "set_bond_amount")
        .args_json(json!({
            "bond_amount": new_bond_amount,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to change config: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let bond_amount: NearToken = serde_json::from_value(new_config["bond_amount"].clone())?;
    assert_eq!(bond_amount, new_bond_amount);

    // Note, vote storage fee cannot be changed without contract upgrade

    // Council IDs
    let original_council_ids: Vec<AccountId> =
        serde_json::from_value(original_config["council_ids"].clone())?;
    let new_council_ids: Vec<AccountId> = vec!["new_council_1".parse()?, "new_council_2".parse()?];
    assert_ne!(original_council_ids, new_council_ids);

    let outcome = user
        .call(v.voting_id(), "set_council_ids")
        .args_json(json!({
            "council_ids": new_council_ids,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Regular user should not be able to change council_ids: {:#?}",
        outcome
    );

    let outcome = voting_owner
        .call(v.voting_id(), "set_council_ids")
        .args_json(json!({
            "council_ids": new_council_ids,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to change council_ids: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let council_ids: Vec<AccountId> = serde_json::from_value(new_config["council_ids"].clone())?;
    assert_eq!(council_ids, new_council_ids);

    // Guardians

    let original_guardians: Vec<AccountId> =
        serde_json::from_value(original_config["guardians"].clone())?;
    let new_guardian = v.sandbox.dev_create_account().await?;

    let new_guardians: Vec<AccountId> =
        vec!["new_guardian_1.near".parse()?, new_guardian.id().clone()];
    assert_ne!(original_guardians, new_guardians);

    // Attempt set_guardians
    let outcome = user
        .call(v.voting_id(), "set_guardians")
        .args_json(json!({
            "guardians": new_guardians
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "User should not be able to set guardians",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let guardians: Vec<AccountId> = serde_json::from_value(config["guardians"].clone())?;
    assert_eq!(guardians, original_guardians);

    let outcome = voting_owner
        .call(v.voting_id(), "set_guardians")
        .args_json(json!({
            "guardians": new_guardians
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to set guardians",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let guardians: Vec<AccountId> = serde_json::from_value(config["guardians"].clone())?;
    assert_eq!(guardians, new_guardians);

    // Quorum & approval threshold config

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    assert_eq!(config["quorum_threshold_bps"].as_u64().unwrap(), 3500);
    assert_eq!(
        config["simple_majority_threshold_bps"].as_u64().unwrap(),
        5000
    );
    assert_eq!(
        config["strong_majority_threshold_bps"].as_u64().unwrap(),
        6667
    );

    // Regular user cannot set quorum params
    let outcome = user
        .call(v.voting_id(), "set_quorum_threshold_bps")
        .args_json(json!({ "quorum_threshold_bps": 5000u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    let outcome = user
        .call(v.voting_id(), "set_quorum_floor")
        .args_json(json!({ "quorum_floor": NearToken::from_near(100) }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    let outcome = user
        .call(v.voting_id(), "set_simple_majority_threshold_bps")
        .args_json(json!({ "simple_majority_threshold_bps": 6000u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    let outcome = user
        .call(v.voting_id(), "set_strong_majority_threshold_bps")
        .args_json(json!({ "strong_majority_threshold_bps": 7000u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    // Owner can set quorum params
    let outcome = voting_owner
        .call(v.voting_id(), "set_quorum_threshold_bps")
        .args_json(json!({ "quorum_threshold_bps": 5000u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_success(), "Failed: {:#?}", outcome);

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    assert_eq!(config["quorum_threshold_bps"].as_u64().unwrap(), 5000);

    let new_floor = NearToken::from_near(100);
    let outcome = voting_owner
        .call(v.voting_id(), "set_quorum_floor")
        .args_json(json!({ "quorum_floor": new_floor }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_success(), "Failed: {:#?}", outcome);

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let floor: NearToken = serde_json::from_value(config["quorum_floor"].clone())?;
    assert_eq!(floor, new_floor);

    let outcome = voting_owner
        .call(v.voting_id(), "set_simple_majority_threshold_bps")
        .args_json(json!({ "simple_majority_threshold_bps": 5100u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_success(), "Failed: {:#?}", outcome);

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    assert_eq!(
        config["simple_majority_threshold_bps"].as_u64().unwrap(),
        5100
    );

    let outcome = voting_owner
        .call(v.voting_id(), "set_strong_majority_threshold_bps")
        .args_json(json!({ "strong_majority_threshold_bps": 7500u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_success(), "Failed: {:#?}", outcome);

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    assert_eq!(
        config["strong_majority_threshold_bps"].as_u64().unwrap(),
        7500
    );

    // Validation: quorum_threshold_bps > 10000 should fail
    let outcome = voting_owner
        .call(v.voting_id(), "set_quorum_threshold_bps")
        .args_json(json!({ "quorum_threshold_bps": 10001u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    // Validation: simple_majority_threshold_bps > 10000 should fail
    let outcome = voting_owner
        .call(v.voting_id(), "set_simple_majority_threshold_bps")
        .args_json(json!({ "simple_majority_threshold_bps": 10001u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    // Validation: strong_majority_threshold_bps > 10000 should fail
    let outcome = voting_owner
        .call(v.voting_id(), "set_strong_majority_threshold_bps")
        .args_json(json!({ "strong_majority_threshold_bps": 10001u16 }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(outcome.is_failure());

    // Change owner account ID
    let new_owner_account = v.sandbox.dev_create_account().await?;
    let original_owner_account_id: AccountId =
        serde_json::from_value(original_config["owner_account_id"].clone())?;
    let new_owner_account_id: AccountId = new_owner_account.id().clone();
    assert_ne!(original_owner_account_id, new_owner_account_id);

    // Attempt propose_new_owner_account_id
    let outcome = user
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({
            "new_owner_account_id": new_owner_account_id
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "User should not be able to propose new owner_account_id",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, original_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert!(proposed_new_owner_account_id.is_none());

    let outcome = voting_owner
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({
            "new_owner_account_id": new_owner_account_id
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to propose new owner_account_id",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, original_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert_eq!(
        proposed_new_owner_account_id.as_ref(),
        Some(&new_owner_account_id)
    );

    // Cancel proposal
    let outcome = voting_owner
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({
            "new_owner_account_id": None::<String>
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "The current owner should be able to cancel the proposal"
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, original_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert!(proposed_new_owner_account_id.is_none());

    let outcome = voting_owner
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({
            "new_owner_account_id": new_owner_account_id
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to propose new owner_account_id",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, original_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert_eq!(
        proposed_new_owner_account_id.as_ref(),
        Some(&new_owner_account_id)
    );

    // Accept the ownership by different account
    let outcome = user
        .call(v.voting_id(), "accept_ownership")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "User should not be able to accept the ownership",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, original_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert_eq!(
        proposed_new_owner_account_id.as_ref(),
        Some(&new_owner_account_id)
    );

    // Accept ownership by the new owner
    let outcome = new_owner_account
        .call(v.voting_id(), "accept_ownership")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "The new owner should be able to accept the ownership",
    );

    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId = serde_json::from_value(config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, new_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(config["proposed_new_owner_account_id"].clone())?;
    assert!(proposed_new_owner_account_id.is_none());

    // Propose a config with the new owner
    let outcome = new_owner_account
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({
            "new_owner_account_id": original_owner_account_id,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "New owner should be able to change config: {:#?}",
        outcome
    );

    let new_config: serde_json::Value =
        v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    let owner_account_id: AccountId =
        serde_json::from_value(new_config["owner_account_id"].clone())?;
    assert_eq!(owner_account_id, new_owner_account_id);
    let proposed_new_owner_account_id: Option<AccountId> =
        serde_json::from_value(new_config["proposed_new_owner_account_id"].clone())?;
    assert_eq!(
        proposed_new_owner_account_id.as_ref(),
        Some(&original_owner_account_id)
    );

    Ok(())
}

#[tokio::test]
async fn test_voting_v2_pause() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;
    let voting_owner = &v.voting.as_ref().unwrap().owner;

    // Attempt to pause the contract
    let outcome = user_a
        .call(v.voting_id(), "pause")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "User should not be able to pause the contract",
    );

    let is_paused: bool = v
        .sandbox
        .view(v.voting_id(), "is_paused")
        .await?
        .json()
        .unwrap();
    assert!(!is_paused, "Contract should not be paused");

    // Pause the contract by the guardian
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .guardian
        .call(v.voting_id(), "pause")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Guardian should be able to pause the contract",
    );

    let is_paused: bool = v
        .sandbox
        .view(v.voting_id(), "is_paused")
        .await?
        .json()
        .unwrap();
    assert!(is_paused, "Contract should be paused");

    // Check if guardian can unpause the contract
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .guardian
        .call(v.voting_id(), "unpause")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Guardian should not be able to unpause the contract",
    );

    let is_paused: bool = v
        .sandbox
        .view(v.voting_id(), "is_paused")
        .await?
        .json()
        .unwrap();
    assert!(is_paused, "Contract should be paused");

    // Unpause the contract by the owner
    let outcome = voting_owner
        .call(v.voting_id(), "unpause")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to unpause the contract",
    );

    let is_paused: bool = v
        .sandbox
        .view(v.voting_id(), "is_paused")
        .await?
        .json()
        .unwrap();
    assert!(!is_paused, "Contract should not be paused");

    // Prepare for pause testing
    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;
    let proposal_id_2 = create_proposal_v2(&v, &user_a, None).await?;
    assert_ne!(proposal_id, proposal_id_2);

    let (user_a_merkle_proof, user_a_v_account): (serde_json::Value, serde_json::Value) = v
        .sandbox
        .view(v.venear.id(), "get_proof")
        .args_json(json!({
            "account_id": user_a.id(),
        }))
        .await?
        .json()?;

    let (user_b_merkle_proof, user_b_v_account): (serde_json::Value, serde_json::Value) = v
        .sandbox
        .view(v.venear.id(), "get_proof")
        .args_json(json!({
            "account_id": user_b.id(),
        }))
        .await?
        .json()?;

    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Voting)
        .await?;

    // Change vote to Against (now in Voting)
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Against,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "user_a: Failed to vote: {:#?}",
        outcome
    );

    // Pause the contract by the owner
    let outcome = voting_owner
        .call(v.voting_id(), "pause")
        .args_json(json!({}))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(100))
        .transact()
        .await?;
    assert!(
        outcome.is_success(),
        "Owner should be able to pause the contract",
    );
    let is_paused: bool = v
        .sandbox
        .view(v.voting_id(), "is_paused")
        .await?
        .json()
        .unwrap();

    assert!(is_paused, "Contract should be paused");

    // Attempt to change vote while paused
    let outcome = user_a
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::For,
            "merkle_proof": user_a_merkle_proof,
            "v_account": user_a_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_a: Voted while paused: {:#?}",
        outcome
    );

    // Attempt to vote by user_b while paused
    let outcome = user_b
        .call(v.voting_id(), "vote")
        .args_json(json!({
            "proposal_id": proposal_id,
            "vote": VoteOption::Against,
            "merkle_proof": user_b_merkle_proof,
            "v_account": user_b_v_account,
        }))
        .deposit(NearToken::from_millinear(15))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_b: Voted while paused: {:#?}",
        outcome
    );

    // Attempt to create a v2 proposal while paused
    let outcome = user_b
        .call(v.voting_id(), "create_proposal")
        .args_json(json!({
            "metadata": {
                "title": "Test Proposal",
                "description": "This is a test proposal",
            },
            "flow": "V2",
        }))
        .deposit(NearToken::from_millinear(10_100))
        .gas(Gas::from_tgas(50))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "user_b: Created proposal while paused: {:#?}",
        outcome
    );

    // Attempt to approve a proposal while paused
    assert!(
        approve_proposal_v2(
            &v,
            &v.voting.as_ref().unwrap().reviewer,
            proposal_id_2,
            MajorityType::Simple
        )
        .await
        .is_err(),
        "Reviewer should not be able to approve proposal while paused"
    );

    // Attempt to reject a proposal while paused (council call, but contract is paused)
    let outcome = v
        .voting
        .as_ref()
        .unwrap()
        .council
        .call(v.voting_id(), "reject_proposal")
        .args_json(json!({
            "proposal_id": proposal_id_2,
        }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(Gas::from_tgas(250))
        .transact()
        .await?;
    assert!(
        outcome.is_failure(),
        "Council should not be able to reject proposal while paused: {:#?}",
        outcome
    );

    Ok(())
}

#[tokio::test]
async fn test_voting_v2_proposal_expiration() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;

    // Fast-forward past the expiration window
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Expired)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Expired,
        "Proposal should be Expired after expiration window"
    );

    // Attempt to approve — should fail because the proposal is expired
    assert!(
        approve_proposal_v2(
            &v,
            &v.voting.as_ref().unwrap().reviewer,
            proposal_id,
            MajorityType::Simple
        )
        .await
        .is_err(),
        "Should not be able to approve an expired proposal"
    );

    // Bond should be claimable from expired proposals
    let balance_before = user_a.view_account().await?.balance;
    let outcome = claim_bond(&v, &user_a, proposal_id).await?;
    assert!(
        outcome.is_success(),
        "claim_bond should succeed for expired proposals: {:?}",
        outcome.failures()
    );
    let balance_after = user_a.view_account().await?.balance;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(proposal["bond_amount"].as_str().unwrap(), "0");
    assert!(balance_after > balance_before);

    Ok(())
}

#[tokio::test]
async fn test_v2_quorum_succeeded() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    // Lock NEAR to get veNEAR
    v.transfer_and_lock(&user_a, NearToken::from_near(1000))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(500))
        .await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    // Both vote For
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Voting)
        .await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Succeeded)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Succeeded,
        "Proposal should succeed with enough For votes"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_quorum_defeated_insufficient_votes() -> Result<(), Box<dyn std::error::Error>> {
    // Default quorum_threshold_bps=3500 (35%). user_a holds 300/(300+1000)=23% < 35%.
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(300))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    // Only user_a votes, below 35% quorum threshold
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;

    // Fast forward past voting end
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Defeated)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Defeated,
        "Proposal should be Defeated after voting ends"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_quorum_defeated_succeed_failed() -> Result<(), Box<dyn std::error::Error>> {
    // Low quorum so it's met, but more Against than For
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(500))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    // user_a votes For, user_b votes Against (more power)
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Voting)
        .await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::Against).await?;

    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Defeated)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Defeated,
        "Proposal should be defeated: more Against than For"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_quorum_with_abstain() -> Result<(), Box<dyn std::error::Error>> {
    // Default quorum_threshold_bps=3500 (35%). user_a holds 300/(300+1000)=23% < 35%.
    // user_a alone can't meet quorum, but user_b's Abstain pushes total to 100%.
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(300))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    // user_b votes For to graduate from Sandbox (1000/1300 = 77% > 30%)
    vote_for_option(&v, &user_b, proposal_id, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Voting)
        .await?;
    // user_a votes For (23% alone < 35% quorum), user_b changes to Abstain
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::Abstain).await?;

    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Succeeded)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Succeeded,
        "Abstain should count for quorum, and For/(For+Against) = 100% >= 50%"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_proposal_with_transfer_action() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(300))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    // Fund the voting contract so it can execute transfers
    let voting_id: AccountId = v.voting_id().clone();
    v.sandbox
        .root_account()?
        .transfer_near(&voting_id, NearToken::from_near(5))
        .await?
        .into_result()?;

    let recipient = v.sandbox.dev_create_account().await?;
    let recipient_balance_before = recipient.view_account().await?.balance;

    // Create proposal with a Transfer action
    let proposal_id = create_proposal_v2(
        &v,
        &user_a,
        Some(json!([{
            "Transfer": {
                "receiver_id": recipient.id().to_string(),
                "amount": NearToken::from_near(1).as_yoctonear().to_string(),
            }
        }])),
    )
    .await?;

    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::For).await?;

    // Try to execute while still in Scheduled status — should fail
    let outcome = execute_proposal(&v, &user_b, proposal_id).await?;
    assert!(
        outcome.is_failure(),
        "Execute should fail when proposal is not Executable"
    );

    // Should go to Executable (not Succeeded) because it has actions
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Executable)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Executable,
        "Proposal with actions should be Executable after voting succeeds"
    );

    // Anyone can execute
    let outcome = execute_proposal(&v, &user_b, proposal_id).await?;
    assert!(
        outcome.is_success(),
        "Execute proposal failed: {:#?}",
        outcome
    );

    // Verify status is now Succeeded
    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Succeeded,
        "Proposal should be Succeeded after execution"
    );

    // Verify the transfer happened
    let recipient_balance_after = recipient.view_account().await?.balance;
    assert!(
        recipient_balance_after.as_yoctonear() - recipient_balance_before.as_yoctonear()
            >= NearToken::from_near(1).as_yoctonear(),
        "Recipient should have received 1 NEAR"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_proposal_with_function_call_actions() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let voting = v.voting.as_ref().unwrap();
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(300))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    // Propose transferring ownership to the voting contract itself,
    // so governance proposals can change its config.
    voting
        .owner
        .call(v.voting_id(), "propose_new_owner_account_id")
        .args_json(json!({ "new_owner_account_id": v.voting_id() }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;

    // Single proposal with two actions
    let new_fee = NearToken::from_millinear(500);
    let fee_args = near_sdk::json_types::Base64VecU8(
        serde_json::to_vec(&json!({ "bond_amount": new_fee })).unwrap(),
    );
    let proposal_id = create_proposal_v2(
        &v,
        &user_a,
        Some(json!([
            {
                "FunctionCall": {
                    "receiver_id": v.voting_id().to_string(),
                    "method_name": "accept_ownership",
                    "args": near_sdk::json_types::Base64VecU8(b"{}".to_vec()),
                    "deposit": "1",
                    "gas": Gas::from_tgas(5).as_gas().to_string(),
                }
            },
            {
                "FunctionCall": {
                    "receiver_id": v.voting_id().to_string(),
                    "method_name": "set_bond_amount",
                    "args": fee_args,
                    "deposit": "1",
                    "gas": Gas::from_tgas(5).as_gas().to_string(),
                }
            }
        ])),
    )
    .await?;

    approve_proposal_v2(&v, &voting.reviewer, proposal_id, MajorityType::Simple).await?;
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::For).await?;

    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Executable)
        .await?;

    let outcome = execute_proposal(&v, &user_a, proposal_id).await?;
    assert!(
        outcome.is_success(),
        "Execute proposal failed: {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Succeeded,);

    // Verify both actions executed
    let config: serde_json::Value = v.sandbox.view(v.voting_id(), "get_config").await?.json()?;
    assert_eq!(
        config["owner_account_id"].as_str().unwrap(),
        v.voting_id().as_str(),
        "Voting contract should now own itself"
    );
    assert_eq!(
        config["bond_amount"].as_str().unwrap(),
        new_fee.as_yoctonear().to_string(),
        "bond_amount should have been updated by the proposal"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_execute_proposal_failure_is_terminal() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(300))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    // Create proposal calling a nonexistent method to trigger failure
    let proposal_id = create_proposal_v2(
        &v,
        &user_a,
        Some(json!([{
            "FunctionCall": {
                "receiver_id": v.voting_id().to_string(),
                "method_name": "nonexistent_method_that_will_fail",
                "args": "",
                "deposit": "0",
                "gas": Gas::from_tgas(5).as_gas().to_string(),
            }
        }])),
    )
    .await?;

    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;
    vote_for_option(&v, &user_b, proposal_id, VoteOption::For).await?;

    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Executable)
        .await?;

    // Execute — should fail because the method doesn't exist
    let outcome = execute_proposal(&v, &user_a, proposal_id).await?;
    assert!(
        outcome.is_success(),
        "The execute call should succeed (callback handles failure): {:#?}",
        outcome
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Failed,
        "Proposal should be Failed after execution failure"
    );

    // Try to execute again — should fail
    let outcome = execute_proposal(&v, &user_a, proposal_id).await?;
    assert!(
        outcome.is_failure(),
        "Should not be able to execute a Failed proposal"
    );

    Ok(())
}

#[tokio::test]
async fn test_v2_slash_proposal() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    let reviewer = v.voting.as_ref().unwrap().reviewer.clone();

    // Non-reviewer cannot slash
    let proposal_id = create_proposal_v2(&v, &user, None).await?;
    let outcome = slash_proposal(&v, &user, proposal_id).await?;
    assert!(
        outcome.is_failure(),
        "Non-reviewer should not be able to slash"
    );

    // Reviewer slashes proposal — bond is forfeited
    let outcome = slash_proposal(&v, &reviewer, proposal_id).await?;
    assert!(
        outcome.is_success(),
        "slash_proposal should succeed: {:?}",
        outcome.failures()
    );

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Slashed,);
    assert_ne!(
        proposal["bond_amount"].as_str().unwrap(),
        "0",
        "bond_amount should remain non-zero (forfeited)"
    );

    // Cannot claim bond from slashed proposal
    let outcome = claim_bond(&v, &user, proposal_id).await?;
    assert!(
        outcome.is_failure(),
        "Cannot claim bond from slashed proposal"
    );

    Ok(())
}

#[tokio::test]
async fn test_bond_claim_by_non_proposer_fails() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Expired)
        .await?;

    let outcome = claim_bond(&v, &user_b, proposal_id).await?;
    assert!(
        outcome.is_failure(),
        "Non-proposer should not be able to claim bond"
    );
    Ok(())
}

#[tokio::test]
async fn test_bond_double_claim_fails() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;

    let proposal_id = create_proposal_v2(&v, &user, None).await?;
    v.fast_forward_to_proposal_status_v2(proposal_id, ProposalStatus::Expired)
        .await?;

    let outcome = claim_bond(&v, &user, proposal_id).await?;
    assert!(outcome.is_success(), "First claim should succeed");

    let outcome = claim_bond(&v, &user, proposal_id).await?;
    assert!(outcome.is_failure(), "Second claim should fail");
    Ok(())
}

#[tokio::test]
async fn test_bond_zero_amount() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .bond_amount(NearToken::from_yoctonear(0))
        .build()
        .await?;
    let user = v.create_account_with_lockup().await?;
    let reviewer = v.voting.as_ref().unwrap().reviewer.clone();

    let proposal_id = create_proposal_v2(&v, &user, None).await?;
    let proposal = v.get_proposal(proposal_id).await?;

    assert_eq!(get_status(&proposal)?, ProposalStatus::Created,);
    assert_eq!(
        proposal["bond_amount"].as_str().unwrap(),
        "0",
        "bond_amount should be 0 when configured as zero"
    );

    approve_proposal_v2(&v, &reviewer, proposal_id, MajorityType::Simple).await?;
    let proposal_after = v.get_proposal(proposal_id).await?;

    assert_eq!(get_status(&proposal_after)?, ProposalStatus::Sandbox,);
    assert_eq!(
        proposal_after["bond_amount"].as_str().unwrap(),
        "0",
        "bond_amount should remain 0 after approval"
    );
    Ok(())
}

#[tokio::test]
async fn test_strong_majority() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;
    let user_c = v.create_account_with_lockup().await?;

    // user_a: 600 NEAR, user_b: 400 NEAR, user_c: 201 NEAR
    v.transfer_and_lock(&user_a, NearToken::from_near(600))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(400))
        .await?;
    v.transfer_and_lock(&user_c, NearToken::from_near(201))
        .await?;

    // --- Proposal 1: Simple majority, 60/40 split — should pass ---
    let proposal_simple = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_simple,
        MajorityType::Simple,
    )
    .await?;

    vote_for_option(&v, &user_a, proposal_simple, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_simple, ProposalStatus::Voting)
        .await?;
    vote_for_option(&v, &user_b, proposal_simple, VoteOption::Against).await?;

    v.fast_forward_to_proposal_status_v2(proposal_simple, ProposalStatus::Succeeded)
        .await?;

    let proposal = v.get_proposal(proposal_simple).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Succeeded,
        "60/40 split should pass simple majority"
    );

    // --- Proposal 2: Strong majority, 60/40 split — should be defeated ---
    let proposal_strong_fail = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_strong_fail,
        MajorityType::Strong,
    )
    .await?;

    vote_for_option(&v, &user_a, proposal_strong_fail, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_strong_fail, ProposalStatus::Voting)
        .await?;
    vote_for_option(&v, &user_b, proposal_strong_fail, VoteOption::Against).await?;

    v.fast_forward_to_proposal_status_v2(proposal_strong_fail, ProposalStatus::Succeeded)
        .await?;

    let proposal = v.get_proposal(proposal_strong_fail).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Defeated,
        "60/40 split should fail strong majority (~66.67%)"
    );

    // --- Proposal 3: Strong majority, barely passing — 801 For / 400 Against = 66.69% ---
    let proposal_strong_pass = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_strong_pass,
        MajorityType::Strong,
    )
    .await?;

    // user_a (600) + user_c (201) = 801 For, user_b (400) Against → 801/1201 = 66.69%
    vote_for_option(&v, &user_a, proposal_strong_pass, VoteOption::For).await?;
    v.fast_forward_to_proposal_status_v2(proposal_strong_pass, ProposalStatus::Voting)
        .await?;
    vote_for_option(&v, &user_c, proposal_strong_pass, VoteOption::For).await?;
    vote_for_option(&v, &user_b, proposal_strong_pass, VoteOption::Against).await?;

    v.fast_forward_to_proposal_status_v2(proposal_strong_pass, ProposalStatus::Succeeded)
        .await?;

    let proposal = v.get_proposal(proposal_strong_pass).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Succeeded,
        "801/1201 (66.69%) should barely pass strong majority"
    );

    Ok(())
}

#[tokio::test]
async fn test_sandbox_expiry_defeated() -> Result<(), Box<dyn std::error::Error>> {
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;
    let user_b = v.create_account_with_lockup().await?;

    // user_a: 200 NEAR, user_b: 1000 NEAR. Total = 1200. 30% = 360.
    // user_a voting For (200) won't meet the 360 threshold.
    v.transfer_and_lock(&user_a, NearToken::from_near(200))
        .await?;
    v.transfer_and_lock(&user_b, NearToken::from_near(1000))
        .await?;

    let proposal_id = create_proposal_v2(&v, &user_a, None).await?;
    approve_proposal_v2(
        &v,
        &v.voting.as_ref().unwrap().reviewer,
        proposal_id,
        MajorityType::Simple,
    )
    .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(get_status(&proposal)?, ProposalStatus::Sandbox,);

    // user_a votes For — 200/1200 = 16.7% < 30%, stays in Sandbox
    vote_for_option(&v, &user_a, proposal_id, VoteOption::For).await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Sandbox,
        "Should still be in Sandbox (16.7% < 30%)"
    );

    // Fast forward past sandbox period end
    let proposal = v.get_proposal(proposal_id).await?;
    let approval_time: u64 = proposal["approval_time_ns"].as_str().unwrap().parse()?;
    let sandbox_duration: u64 = proposal["sandbox_duration_ns"].as_str().unwrap().parse()?;
    let sandbox_end = approval_time + sandbox_duration;
    v.fast_forward(sandbox_end, sandbox_duration / NS_IN_SECOND, 20)
        .await?;

    let proposal = v.get_proposal(proposal_id).await?;
    assert_eq!(
        get_status(&proposal)?,
        ProposalStatus::Defeated,
        "Sandbox should expire to Defeated when threshold not met"
    );

    Ok(())
}

#[tokio::test]
async fn test_two_proposals_scheduled_concurrently() -> Result<(), Box<dyn std::error::Error>> {
    // Under the concurrency model, two approved V2 proposals share active slots rather than
    // serializing voting windows. Both can graduate Sandbox and run Voting in parallel.
    let v = VenearTestWorkspaceBuilder::default()
        .with_voting()
        .build()
        .await?;
    let user_a = v.create_account_with_lockup().await?;

    v.transfer_and_lock(&user_a, NearToken::from_near(1000))
        .await?;

    let proposal_1 = create_proposal_v2(&v, &user_a, None).await?;
    let proposal_2 = create_proposal_v2(&v, &user_a, None).await?;
    let reviewer = &v.voting.as_ref().unwrap().reviewer;
    approve_proposal_v2(&v, reviewer, proposal_1, MajorityType::Simple).await?;
    approve_proposal_v2(&v, reviewer, proposal_2, MajorityType::Simple).await?;

    // Both should be in Sandbox immediately (cap is 3, so both fit).
    let p1 = v.get_proposal(proposal_1).await?;
    let p2 = v.get_proposal(proposal_2).await?;
    assert_eq!(get_status(&p1)?, ProposalStatus::Sandbox,);
    assert_eq!(get_status(&p2)?, ProposalStatus::Sandbox,);

    // Graduate proposal 1 from Sandbox.
    vote_for_option(&v, &user_a, proposal_1, VoteOption::For).await?;
    let p1 = v.get_proposal(proposal_1).await?;
    assert_eq!(get_status(&p1)?, ProposalStatus::Scheduled,);
    let p1_voting_start: u64 = p1["voting_start_time_ns"].as_str().unwrap().parse()?;

    // Graduate proposal 2. Its voting_start is computed from the current block time
    // (not from `p1_voting_end`), so both windows overlap instead of serializing.
    vote_for_option(&v, &user_a, proposal_2, VoteOption::For).await?;
    let p2 = v.get_proposal(proposal_2).await?;
    assert_eq!(get_status(&p2)?, ProposalStatus::Scheduled,);
    let p2_voting_start: u64 = p2["voting_start_time_ns"].as_str().unwrap().parse()?;

    // In sandbox feature mode, `next_voting_start_ns(now) = now + 120s`. The two calls happen
    // within a few blocks, so the difference is bounded well below one full cycle.
    let scheduling_delay_ns: u64 = 120 * NS_IN_SECOND;
    let delta = p2_voting_start.saturating_sub(p1_voting_start);
    assert!(
        delta < scheduling_delay_ns,
        "Proposals should be scheduled concurrently, not serialized (delta={}ns)",
        delta
    );

    // Fast-forward to proposal 1's voting window. Because p2_voting_start < p1_voting_start + 120s,
    // proposal 2 should also have crossed its own voting_start by then.
    v.fast_forward_to_proposal_status_v2(proposal_1, ProposalStatus::Voting)
        .await?;
    let p1 = v.get_proposal(proposal_1).await?;
    let p2 = v.get_proposal(proposal_2).await?;
    assert_eq!(
        get_status(&p1)?,
        ProposalStatus::Voting,
        "Proposal 1 should be Voting"
    );
    assert_eq!(
        get_status(&p2)?,
        ProposalStatus::Voting,
        "Proposal 2 should also be Voting in parallel, not waiting for proposal 1 to finish"
    );

    Ok(())
}
