// SPDX-License-Identifier: Apache-2.0
//! Task workspace affinity is immutable evidence, never an access grant.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshotPlan {
    pub source_head: String,
    pub state_sha256: String,
}
impl SourceSnapshotPlan {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.source_head.len(), 40 | 64)
            || !self
                .source_head
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || self.state_sha256.len() != 64
            || !self
                .state_sha256
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err("Invalid source snapshot identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshotBinding {
    pub source_root: String,
    pub plan: SourceSnapshotPlan,
    pub tree: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRepositoryIdentity {
    pub common_directory: String,
    pub common_directory_identity: String,
}
impl TaskRepositoryIdentity {
    pub fn validate(&self) -> Result<(), String> {
        let path = Path::new(&self.common_directory);
        if !path.is_absolute()
            || self.common_directory.len() > 4096
            || self.common_directory.contains('\0')
            || path
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            || self.common_directory_identity.is_empty()
            || self.common_directory_identity.len() > 256
            || self.common_directory_identity.chars().any(char::is_control)
        {
            return Err("Invalid frozen project repository identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWorktreeDirectoryIdentity {
    pub root: String,
    pub git_directory: String,
    pub common_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWorktreeSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot: Option<SourceSnapshotBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_directory_identity: Option<TaskWorktreeDirectoryIdentity>,
    pub root: String,
    pub expected_head: String,
    pub expected_common_directory: String,
}

/// A conversation-owned execution checkout, distinct from its source project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimaryWorktreeBinding {
    pub source_root: String,
    pub source_repository: TaskRepositoryIdentity,
    pub selection: TaskWorktreeSelection,
}

impl PrimaryWorktreeBinding {
    pub fn validate(&self) -> Result<(), String> {
        self.selection.validate()?;
        self.source_repository.validate()?;
        let source = Path::new(&self.source_root);
        if !source.is_absolute()
            || self.source_root.contains('\0')
            || source
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            || self.source_root == self.selection.root
            || self.selection.expected_common_directory != self.source_repository.common_directory
            || self
                .selection
                .expected_directory_identity
                .as_ref()
                .map(|ids| ids.common_directory.as_str())
                != Some(self.source_repository.common_directory_identity.as_str())
            || self
                .selection
                .source_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.source_root != self.source_root)
        {
            return Err("Primary execution workspace differs from its source authority".into());
        }
        Ok(())
    }
}

impl TaskWorktreeSelection {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(snapshot) = &self.source_snapshot {
            snapshot.plan.validate()?;
            let path = Path::new(&snapshot.source_root);
            if !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
                || snapshot.source_root.contains('\0')
                || snapshot.source_root == self.root
                || !matches!(snapshot.tree.len(), 40 | 64)
                || !snapshot
                    .tree
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err("Invalid source snapshot binding".into());
            }
        }
        for value in [&self.root, &self.expected_common_directory] {
            let path = Path::new(value);
            if value.len() > 4096
                || value.contains('\0')
                || !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            {
                return Err("Worktree selection requires exact absolute paths".into());
            }
        }
        if !matches!(self.expected_head.len(), 40 | 64)
            || !self
                .expected_head
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Worktree baseline must be a complete commit ID".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskWorktreeBinding {
    pub selection: TaskWorktreeSelection,
    pub git_directory: String,
    pub root_identity: String,
    pub git_directory_identity: String,
    pub common_directory_identity: String,
}

impl TaskWorktreeBinding {
    pub fn directory_identity(&self) -> TaskWorktreeDirectoryIdentity {
        TaskWorktreeDirectoryIdentity {
            root: self.root_identity.clone(),
            git_directory: self.git_directory_identity.clone(),
            common_directory: self.common_directory_identity.clone(),
        }
    }
    pub fn repository_identity(&self) -> TaskRepositoryIdentity {
        TaskRepositoryIdentity {
            common_directory: self.selection.expected_common_directory.clone(),
            common_directory_identity: self.common_directory_identity.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.selection.validate()?;
        for value in [
            &self.selection.root,
            &self.selection.expected_common_directory,
            &self.git_directory,
        ] {
            let path = Path::new(value);
            if value.len() > 4096
                || !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            {
                return Err("Worktree binding requires exact absolute paths".into());
            }
        }
        let head = &self.selection.expected_head;
        if !matches!(head.len(), 40 | 64)
            || !head
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || [
                &self.root_identity,
                &self.git_directory_identity,
                &self.common_directory_identity,
            ]
            .iter()
            .any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
            || !Path::new(&self.git_directory)
                .starts_with(Path::new(&self.selection.expected_common_directory).join("worktrees"))
        {
            return Err("Invalid linked worktree identity".into());
        }
        Ok(())
    }

    pub fn validate_execution_root(&self, roots: &[String]) -> Result<(), String> {
        self.validate()?;
        if roots.iter().any(|root| {
            !Path::new(root).is_absolute()
                || Path::new(root)
                    .components()
                    .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        }) {
            return Err("Invalid frozen workspace authorization roots".into());
        }
        if !roots
            .iter()
            .any(|root| Path::new(&self.selection.root).starts_with(root))
        {
            return Err("Worktree execution root is no longer authorized".into());
        }
        Ok(())
    }

    pub fn validate_authorized_roots(&self, roots: &[String]) -> Result<(), String> {
        self.validate_execution_root(roots)?;
        if self
            .selection
            .source_snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                !roots
                    .iter()
                    .any(|root| Path::new(&snapshot.source_root).starts_with(root))
            })
        {
            return Err("Source snapshot root is not authorized".into());
        }
        if !roots
            .iter()
            .any(|root| Path::new(&self.selection.expected_common_directory).starts_with(root))
        {
            return Err(
                "Shared Git metadata requires a previously authorized workspace root".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSandboxRequirement {
    /// Request Codex's native workspace-write policy; not a generic sandbox attestation.
    CodexWorkspaceWrite,
}
