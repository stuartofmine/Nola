use nola_application::{ApplicationCommand, WorkspaceCheckpoint};

use super::{DbWorkspaceCheckpoint, NolaStore};
use crate::application_port::CliApplicationPort;

impl NolaStore {
    pub fn save_workspace_checkpoint(
        &self,
        checkpoint: &DbWorkspaceCheckpoint,
    ) -> Result<(), String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let inserted: bool = application.execute(ApplicationCommand::SaveWorkspaceCheckpoint {
            input: stored_checkpoint(checkpoint),
        })?;
        if !inserted {
            return Err(format!(
                "workspace checkpoint {} already exists",
                checkpoint.id
            ));
        }
        Ok(())
    }

    pub fn list_workspace_checkpoints(
        &self,
        root: &str,
    ) -> Result<Vec<DbWorkspaceCheckpoint>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let checkpoints: Vec<WorkspaceCheckpoint> =
            application.execute(ApplicationCommand::ListWorkspaceCheckpoints {
                root: root.to_string(),
            })?;
        Ok(checkpoints.into_iter().map(db_checkpoint).collect())
    }

    pub fn delete_workspace_checkpoint(&self, id: &str) -> Result<(), String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let _: bool = application
            .execute(ApplicationCommand::DeleteWorkspaceCheckpoint { id: id.to_string() })?;
        Ok(())
    }
}

fn stored_checkpoint(value: &DbWorkspaceCheckpoint) -> WorkspaceCheckpoint {
    WorkspaceCheckpoint {
        id: value.id.clone(),
        root: value.root.clone(),
        git_root: value.git_root.clone(),
        revision: value.revision.clone(),
        source: value.source.clone(),
        label: value.label.clone(),
        created_at_ms: i64::try_from(value.created_at).unwrap_or(i64::MAX),
        status: value.status.clone(),
        untracked: value.untracked.clone(),
    }
}

fn db_checkpoint(value: WorkspaceCheckpoint) -> DbWorkspaceCheckpoint {
    DbWorkspaceCheckpoint {
        id: value.id,
        root: value.root,
        git_root: value.git_root,
        revision: value.revision,
        source: value.source,
        label: value.label,
        created_at: u64::try_from(value.created_at_ms.max(0)).unwrap_or(u64::MAX),
        status: value.status,
        untracked: value.untracked,
    }
}
