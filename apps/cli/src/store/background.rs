// SPDX-License-Identifier: Apache-2.0
//! Runtime Task projection for background shell processes.
//!
//! Background work is durably owned by the canonical Runtime Task aggregate;
//! this adapter only maps the shell tool's process metadata to that aggregate.

use serde::{Deserialize, Serialize};

use nola_application::{
    ApplicationCommand, CreateRuntimeTask, RuntimeTaskCommand, RuntimeTaskPriority,
    RuntimeTaskResourceBudget, RuntimeTaskResourceUsage, RuntimeTaskScheduling,
};

use super::{DbBackgroundTask, DbRuntimeTask, NolaStore};

const SHELL_PROCESS_KIND: &str = "shell_process";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellProcessMetadata {
    #[serde(default,skip_serializing_if="Option::is_none")]
    process_sandbox: Option<crate::process_sandbox::FrozenProcessSandbox>,
    terminal: Option<nola_domain::terminal_input::TerminalSessionConfiguration>,
    command: String,
    cwd: String,
    pid: u32,
    stdout_path: String,
    stderr_path: String,
    status_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_contract_json: Option<String>,
    #[serde(
        default,
        rename = "_nolaExecution",
        skip_serializing_if = "Option::is_none"
    )]
    sop_invocation: Option<nola_core::agent::ToolInvocationContext>,
}

impl NolaStore {
    pub fn save_background_task(&self, task: &DbBackgroundTask) -> Result<(), String> {
        self.save_background_task_with_scheduling(
            task,
            default_background_scheduling(task.started_at_ms),
        )
    }

    pub fn save_background_task_with_scheduling(
        &self,
        task: &DbBackgroundTask,
        scheduling: RuntimeTaskScheduling,
    ) -> Result<(), String> {
        let mut application = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let _: DbRuntimeTask = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: create_runtime_task_from_background(task, scheduling)?,
            },
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn background_task(&self, id: &str) -> Result<Option<DbBackgroundTask>, String> {
        self.runtime_task(id)?
            .filter(active_shell_task)
            .map(background_task_from_runtime)
            .transpose()
    }

    pub fn list_background_tasks(&self) -> Result<Vec<DbBackgroundTask>, String> {
        self.list_all_runtime_tasks()?
            .into_iter()
            .filter(active_shell_task)
            .map(background_task_from_runtime)
            .collect()
    }
}

fn active_shell_task(task: &DbRuntimeTask) -> bool {
    task.kind == SHELL_PROCESS_KIND
        && !task.cancel_requested
        && matches!(
            task.status.as_str(),
            "queued" | "running" | "retry_waiting" | "waiting_permission" | "suspended"
        )
}

fn create_runtime_task_from_background(
    task: &DbBackgroundTask,
    scheduling: RuntimeTaskScheduling,
) -> Result<CreateRuntimeTask, String> {
    if let Some(size) = &task.terminal {
        size.validate()?;
        if task.owner_session_id.as_deref().is_none_or(str::is_empty) {
            return Err("terminal requires an owning conversation".into());
        }
    }
    let metadata = ShellProcessMetadata {
        process_sandbox: task.process_sandbox.clone(),
        terminal: task.terminal.clone(),
        command: task.command.clone(),
        cwd: task.cwd.clone(),
        pid: task.pid,
        stdout_path: task.stdout_path.clone(),
        stderr_path: task.stderr_path.clone(),
        status_path: task.status_path.clone(),
        output_contract_json: task.output_contract_json.clone(),
        sop_invocation: task.sop_invocation.clone(),
    };
    Ok(CreateRuntimeTask {
        id: task.id.clone(),
        kind: SHELL_PROCESS_KIND.to_string(),
        title: task.command.clone(),
        project_root: task.project_root.clone(),
        owner_session_id: task.owner_session_id.clone(),
        parent_task_id: None,
        parent_tool_call_id: None,
        child_session_id: None,
        agent_type: None,
        prompt: Some(task.command.clone()),
        initial_progress_summary: Some("Background process queued".to_string()),
        initial_activity: Some(task.cwd.clone()),
        output_path: Some(task.stdout_path.clone()),
        initially_unread: false,
        input_watermark: None,
        created_at_ms: task.started_at_ms,
        metadata: serde_json::to_value(&metadata)
            .map_err(|error| format!("encode background process metadata failed: {error}"))?,
        scheduling,
    })
}

fn default_background_scheduling(started_at_ms: i64) -> RuntimeTaskScheduling {
    RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: started_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(2 * 60 * 60 * 1_000),
            max_output_bytes: Some(2 * 1024 * 1024),
            max_cpu_time_ms: None,
            max_memory_bytes: None,
            max_processes: None,
        },
        output_bytes: 0,
        resource_usage: RuntimeTaskResourceUsage::default(),
        budget_exhaustion: None,
    }
}

pub(crate) fn background_task_from_runtime(
    task: DbRuntimeTask,
) -> Result<DbBackgroundTask, String> {
    let metadata: ShellProcessMetadata =
        serde_json::from_str(&task.metadata_json).map_err(|error| {
            format!(
                "decode background process metadata {} failed: {error}",
                task.id
            )
        })?;
    if let Some(size) = &metadata.terminal { size.validate()?; }
    Ok(DbBackgroundTask {
        process_sandbox: metadata.process_sandbox,
        terminal: metadata.terminal,
        id: task.id,
        project_root: task.project_root,
        command: metadata.command,
        cwd: metadata.cwd,
        pid: metadata.pid,
        stdout_path: metadata.stdout_path,
        stderr_path: metadata.stderr_path,
        status_path: metadata.status_path,
        started_at_ms: task.started_at_ms.unwrap_or(task.created_at_ms),
        owner_session_id: task.owner_session_id,
        output_contract_json: metadata.output_contract_json,
        sop_invocation: metadata.sop_invocation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_process_uses_runtime_task_and_keeps_stopped_history() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!(
            "nola-background-runtime-task-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let database = root.join("nola.db");
        let store = NolaStore::open(&database)?;
        let task = DbBackgroundTask {
        process_sandbox: None,
            terminal: None,
            id: "bash-test-1".to_string(),
            project_root: "/workspace".to_string(),
            command: "sleep 10".to_string(),
            cwd: "/workspace".to_string(),
            pid: 42,
            stdout_path: root.join("stdout").to_string_lossy().to_string(),
            stderr_path: root.join("stderr").to_string_lossy().to_string(),
            status_path: root.join("status").to_string_lossy().to_string(),
            started_at_ms: 123,
            owner_session_id: Some("session-test".to_string()),
            output_contract_json: None,
            sop_invocation: None,
        };

        store.save_background_task(&task)?;
        let loaded = store
            .background_task(&task.id)?
            .ok_or_else(|| "saved background task missing".to_string())?;
        assert_eq!(loaded.command, task.command);
        assert_eq!(loaded.sop_invocation, task.sop_invocation);
        assert_eq!(store.list_background_tasks()?.len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }
}
