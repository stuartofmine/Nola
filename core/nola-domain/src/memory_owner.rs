// SPDX-License-Identifier: Apache-2.0
//! Concrete memory ownership, separate from namespace and provenance.
//! Application constructs execution bindings from committed Run identities.

use crate::{MemoryNamespace, MemoryScope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryOwner {
    Profile {
        profile_id: String,
    },
    Workspace {
        profile_id: String,
        workspace_id: String,
    },
    Conversation {
        profile_id: String,
        conversation_id: String,
    },
    Turn {
        profile_id: String,
        conversation_id: String,
        turn_id: String,
    },
}

/// This value is execution metadata, never a model-supplied tool argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryExecutionBinding {
    pub profile_id: String,
    pub workspace_id: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
}

fn identity(value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err("memory owner requires a nonempty canonical identity".into());
    }
    Ok(())
}

/// The current product has one personal memory profile per canonical Store.
/// This identity is Store-relative, not a model configuration profile or account.
pub const LOCAL_MEMORY_PROFILE_ID: &str = "local";

impl MemoryOwner {
    pub fn local_profile() -> Self {
        Self::Profile {
            profile_id: LOCAL_MEMORY_PROFILE_ID.into(),
        }
    }

    pub fn validate_scope(
        &self,
        namespace: &MemoryNamespace,
        scope: MemoryScope,
    ) -> Result<(), String> {
        self.validate()?;
        let binding = match self {
            Self::Profile { profile_id } => MemoryExecutionBinding {
                profile_id: profile_id.clone(),
                workspace_id: None,
                conversation_id: None,
                turn_id: None,
            },
            Self::Workspace {
                profile_id,
                workspace_id,
            } => MemoryExecutionBinding {
                profile_id: profile_id.clone(),
                workspace_id: Some(workspace_id.clone()),
                conversation_id: None,
                turn_id: None,
            },
            Self::Conversation {
                profile_id,
                conversation_id,
            } => MemoryExecutionBinding {
                profile_id: profile_id.clone(),
                workspace_id: None,
                conversation_id: Some(conversation_id.clone()),
                turn_id: None,
            },
            Self::Turn {
                profile_id,
                conversation_id,
                turn_id,
            } => MemoryExecutionBinding {
                profile_id: profile_id.clone(),
                workspace_id: None,
                conversation_id: Some(conversation_id.clone()),
                turn_id: Some(turn_id.clone()),
            },
        };
        if binding.owner_for(namespace, scope)? != *self {
            return Err("memory owner does not match its namespace and scope".into());
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Profile { profile_id } => identity(profile_id),
            Self::Workspace {
                profile_id,
                workspace_id,
            } => {
                identity(profile_id)?;
                identity(workspace_id)
            }
            Self::Conversation {
                profile_id,
                conversation_id,
            } => {
                identity(profile_id)?;
                identity(conversation_id)
            }
            Self::Turn {
                profile_id,
                conversation_id,
                turn_id,
            } => {
                identity(profile_id)?;
                identity(conversation_id)?;
                identity(turn_id)
            }
        }
    }

    pub fn is_visible_to(&self, binding: &MemoryExecutionBinding) -> bool {
        if self.validate().is_err() || binding.validate().is_err() {
            return false;
        }
        match self {
            Self::Profile { profile_id } => profile_id == &binding.profile_id,
            Self::Workspace {
                profile_id,
                workspace_id,
            } => {
                profile_id == &binding.profile_id
                    && binding.workspace_id.as_ref() == Some(workspace_id)
            }
            Self::Conversation {
                profile_id,
                conversation_id,
            } => {
                profile_id == &binding.profile_id
                    && binding.conversation_id.as_ref() == Some(conversation_id)
            }
            Self::Turn {
                profile_id,
                conversation_id,
                turn_id,
            } => {
                profile_id == &binding.profile_id
                    && binding.conversation_id.as_ref() == Some(conversation_id)
                    && binding.turn_id.as_ref() == Some(turn_id)
            }
        }
    }
}

impl MemoryExecutionBinding {
    pub fn local_profile() -> Self {
        Self {
            profile_id: LOCAL_MEMORY_PROFILE_ID.into(),
            workspace_id: None,
            conversation_id: None,
            turn_id: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        identity(&self.profile_id)?;
        for id in [&self.workspace_id, &self.conversation_id, &self.turn_id]
            .into_iter()
            .flatten()
        {
            identity(id)?;
        }
        if self.turn_id.is_some() && self.conversation_id.is_none() {
            return Err("turn memory binding requires its conversation".into());
        }
        Ok(())
    }

    /// Resolve a selected scope without deriving identity from content or citations.
    pub fn owner_for(
        &self,
        namespace: &MemoryNamespace,
        scope: MemoryScope,
    ) -> Result<MemoryOwner, String> {
        self.validate()?;
        if matches!(namespace, MemoryNamespace::Custom(name) if name.trim().is_empty()) {
            return Err("custom memory namespace requires a name".into());
        }
        let profile_id = self.profile_id.clone();
        let owner = match scope {
            MemoryScope::Global => match namespace {
                MemoryNamespace::Personal | MemoryNamespace::Custom(_) => {
                    MemoryOwner::Profile { profile_id }
                }
                MemoryNamespace::Workspace => MemoryOwner::Workspace {
                    profile_id,
                    workspace_id: self
                        .workspace_id
                        .clone()
                        .ok_or("workspace memory requires a bound workspace")?,
                },
                MemoryNamespace::Conversation => {
                    return Err("conversation namespace requires a local scope".into())
                }
            },
            MemoryScope::Conversation => MemoryOwner::Conversation {
                profile_id,
                conversation_id: self
                    .conversation_id
                    .clone()
                    .ok_or("conversation memory requires a bound conversation")?,
            },
            MemoryScope::Turn => MemoryOwner::Turn {
                profile_id,
                conversation_id: self
                    .conversation_id
                    .clone()
                    .ok_or("turn memory requires a bound conversation")?,
                turn_id: self
                    .turn_id
                    .clone()
                    .ok_or("turn memory requires a bound turn")?,
            },
        };
        Ok(owner)
    }
}
