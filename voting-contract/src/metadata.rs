use crate::*;

/// Old metadata format (v1) that includes voting_options.
#[derive(Clone)]
#[near(serializers=[borsh])]
pub struct ProposalMetadataV1 {
    pub title: Option<String>,
    pub description: Option<String>,
    pub link: Option<String>,
    pub voting_options: Vec<String>,
}

#[derive(Clone)]
#[near(serializers=[borsh])]
pub enum VProposalMetadata {
    V1(ProposalMetadataV1),
    Current(ProposalMetadata),
}

impl From<ProposalMetadata> for VProposalMetadata {
    fn from(current: ProposalMetadata) -> Self {
        Self::Current(current)
    }
}

impl From<VProposalMetadata> for ProposalMetadata {
    fn from(value: VProposalMetadata) -> Self {
        match value {
            VProposalMetadata::V1(v1) => ProposalMetadata {
                title: v1.title,
                description: v1.description,
                link: v1.link,
            },
            VProposalMetadata::Current(current) => current,
        }
    }
}

/// Metadata for a proposal.
#[derive(Clone)]
#[near(serializers=[borsh, json])]
pub struct ProposalMetadata {
    /// The title of the proposal.
    pub title: Option<String>,

    /// The description of the proposal.
    pub description: Option<String>,

    /// The link to the proposal.
    pub link: Option<String>,
}
