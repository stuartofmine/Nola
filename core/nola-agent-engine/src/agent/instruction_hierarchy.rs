// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Select and identify scoped instruction sources deterministically.
// - Boundary: Instruction text guides the model; it never grants tool authorization.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInstructionKind {
    CorePolicy,
    HostFact,
    UserInstruction,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstructionSource {
    pub source_id: String,
    pub kind: AgentInstructionKind,
    pub scope_uri: String,
    pub scope_depth: usize,
    pub origin_uri: String,
    pub content_digest: String,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub cacheable: bool,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstructionIdentity {
    pub source_id: String,
    pub kind: AgentInstructionKind,
    pub scope_uri: String,
    pub scope_depth: usize,
    pub origin_uri: String,
    pub content_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub selection_reason: String,
    #[serde(default)]
    pub cacheable: bool,
}

impl Default for AgentInstructionKind {
    fn default() -> Self {
        Self::UserInstruction
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstructionManifest {
    #[serde(default)]
    pub active_scope_uri: String,
    #[serde(default)]
    pub sources: Vec<AgentInstructionIdentity>,
    #[serde(default)]
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionSelectionError {
    EmptySourceId,
    DuplicateSourceId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInstructionSelection {
    pub sources: Vec<AgentInstructionSource>,
    pub manifest: AgentInstructionManifest,
}

impl AgentInstructionSource {
    pub fn scoped_user_instruction(
        source_id: impl Into<String>,
        scope_uri: impl Into<String>,
        origin_uri: impl Into<String>,
        content: impl Into<String>,
        cacheable: bool,
    ) -> Self {
        let content = normalize_content(&content.into());
        let scope_uri = normalize_scope_uri(&scope_uri.into());
        Self {
            source_id: source_id.into(),
            kind: AgentInstructionKind::UserInstruction,
            scope_depth: scope_depth(&scope_uri),
            scope_uri,
            origin_uri: normalize_scope_uri(&origin_uri.into()),
            content_digest: sha256(&content),
            observed_at: None,
            cacheable,
            content,
        }
    }

    fn normalize(mut self) -> Self {
        self.source_id = self.source_id.trim().to_string();
        self.scope_uri = normalize_scope_uri(&self.scope_uri);
        self.origin_uri = normalize_scope_uri(&self.origin_uri);
        self.content = normalize_content(&self.content);
        self.scope_depth = scope_depth(&self.scope_uri);
        self.content_digest = sha256(&self.content);
        self
    }

    fn identity(&self) -> AgentInstructionIdentity {
        AgentInstructionIdentity {
            source_id: self.source_id.clone(),
            kind: self.kind,
            scope_uri: self.scope_uri.clone(),
            scope_depth: self.scope_depth,
            origin_uri: self.origin_uri.clone(),
            content_digest: self.content_digest.clone(),
            observed_at: self.observed_at.clone(),
            selection_reason: "scope_contains_active".to_string(),
            cacheable: self.cacheable,
        }
    }
}

pub fn select_active_instructions(
    active_workspace_uri: &str,
    sources: impl IntoIterator<Item = AgentInstructionSource>,
) -> Result<AgentInstructionSelection, InstructionSelectionError> {
    let active_scope_uri = normalize_scope_uri(active_workspace_uri);
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    for source in sources.into_iter().map(AgentInstructionSource::normalize) {
        if source.source_id.is_empty() {
            return Err(InstructionSelectionError::EmptySourceId);
        }
        if !seen.insert(source.source_id.clone()) {
            return Err(InstructionSelectionError::DuplicateSourceId(
                source.source_id,
            ));
        }
        if scope_contains(&source.scope_uri, &active_scope_uri) {
            selected.push(source);
        }
    }
    selected.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.scope_depth.cmp(&right.scope_depth))
            .then(left.scope_uri.cmp(&right.scope_uri))
            .then(left.source_id.cmp(&right.source_id))
            .then(left.content_digest.cmp(&right.content_digest))
    });
    let identities = selected
        .iter()
        .map(AgentInstructionSource::identity)
        .collect::<Vec<_>>();
    let digest_material = serde_json::to_vec(&identities).unwrap_or_default();
    let manifest = AgentInstructionManifest {
        active_scope_uri,
        sources: identities,
        digest: hex::encode(Sha256::digest(digest_material)),
    };
    Ok(AgentInstructionSelection {
        sources: selected,
        manifest,
    })
}

fn scope_contains(scope: &str, active: &str) -> bool {
    if scope == "global://" {
        return true;
    }
    scope == active
        || active
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalize_scope_uri(value: &str) -> String {
    let value = value.trim().replace('\\', "/");
    if value.eq_ignore_ascii_case("global") || value == "global://" {
        return "global://".to_string();
    }
    let mut parts = Vec::new();
    let absolute = value.starts_with('/');
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let normalized = parts.join("/");
    if absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

fn scope_depth(scope: &str) -> usize {
    if scope == "global://" {
        0
    } else {
        scope.split('/').filter(|part| !part.is_empty()).count()
    }
}

fn normalize_content(content: &str) -> String {
    content
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

fn sha256(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}
