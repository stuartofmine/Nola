// SPDX-License-Identifier: Apache-2.0
//! Original tool transcript identities, bound once to an accepted canonical Part.
use serde::{Deserialize, Serialize};
use crate::assistant_ownership::AssistantPartOwner;
use nola_domain::{CanonicalBodyReference, CanonicalPartOwner};

pub use nola_domain::tool_source::{ToolSourceKind,ToolSourceClaim,ToolSourceOriginalOwner,ToolSourceDerivationClaim,ToolSourceExportRange};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolSourceProvenance {
    AcceptedRun,
    VerifiedLocalDerivation {
        operation_id: String,
        target_owner: CanonicalPartOwner,
        source_owner: CanonicalPartOwner,
        source_binding_sha256: String,
        source_provenance: nola_domain::PartDerivationProvenance,
    },
    Replica { claim:ToolSourceClaim, replica_id:String, operation_id:String },
    Archive { claim:ToolSourceClaim, restore_id:String, sequence:u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSourceBinding {
    pub source_item_id: String,
    pub kind: ToolSourceKind,
    pub call_id: String,
    pub name: String,
    pub owner: AssistantPartOwner,
    pub provenance: ToolSourceProvenance,
    pub body_sha256: String,
    /// Independent immutable object; not the visible Part's JSON body reference.
    pub body: CanonicalBodyReference,
    pub created_revision: i64,
}
impl ToolSourceBinding {
    pub fn canonical_owner(&self) -> Result<CanonicalPartOwner,String> {
        match &self.provenance {
            ToolSourceProvenance::AcceptedRun => CanonicalPartOwner::new(self.owner.owner.conversation_id.clone(),
                self.owner.owner.assistant_message_id.clone(),self.owner.part_id.clone()).map_err(|e|e.to_string()),
            ToolSourceProvenance::VerifiedLocalDerivation {target_owner,..} => Ok(target_owner.clone()),
            ToolSourceProvenance::Replica {claim,..} | ToolSourceProvenance::Archive {claim,..} => Ok(claim.owner.clone()),
        }
    }
    pub fn claim(&self)->Result<ToolSourceClaim,String> {
        match &self.provenance {
            ToolSourceProvenance::Replica {claim,..} | ToolSourceProvenance::Archive {claim,..} => Ok(claim.clone()),
            _=> {
                let derivation=match &self.provenance {
                    ToolSourceProvenance::VerifiedLocalDerivation {operation_id,source_owner,source_binding_sha256,..}=>Some(ToolSourceDerivationClaim {
                        operation_id:operation_id.clone(),source_owner:source_owner.clone(),source_binding_sha256:source_binding_sha256.clone(),
                    }),
                    _=>None,
                };
                let claim=ToolSourceClaim {schema_version:1,source_item_id:self.source_item_id.clone(),kind:self.kind,
                    call_id:self.call_id.clone(),name:self.name.clone(),owner:self.canonical_owner()?,
                    original_owner:ToolSourceOriginalOwner {part_owner:CanonicalPartOwner::new(self.owner.owner.conversation_id.clone(),
                        self.owner.owner.assistant_message_id.clone(),self.owner.part_id.clone()).map_err(|e|e.to_string())?,
                        run_id:self.owner.owner.run_id.clone(),turn_id:self.owner.turn_id.clone().ok_or("original tool source lacks Turn")?},
                    body:self.body.clone(),body_sha256:self.body_sha256.clone(),derivation};
                claim.validate()?;
                Ok(claim)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSourceInput {
    pub source_item_id: String,
    pub kind: ToolSourceKind,
    pub call_id: String,
    pub name: String,
    pub owner: AssistantPartOwner,
    /// Exact serialized original AgentItem, never a model/display preview.
    pub body_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSourceRange {
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

/// History only. Possession of a binding grants no execution or tool authority.
pub trait ToolSourceRepository {
    fn tool_sources_for_part(&self, owner: &CanonicalPartOwner, revision: i64)
        -> Result<Vec<ToolSourceBinding>, String>;
    fn tool_source_by_id(&self, conversation: &str, source_item_id: &str, revision: i64)
        -> Result<Option<ToolSourceBinding>, String>;
    fn read_tool_source_range(&self, binding: &ToolSourceBinding, revision: i64,
        offset_bytes: u64, limit_bytes: usize) -> Result<ToolSourceRange, String>;
}
