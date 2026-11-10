use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Local;

mod background;
mod checkpoint;
mod document;
mod event;
mod memory;
mod model;
mod plan;
mod run_lock;
mod schedule;
mod session;
#[cfg(test)]
mod skill;
mod sop;
mod task;
mod web;
pub(crate) use web::web_model_selection;
mod web_conversation;

pub(crate) use web_conversation::{ConversationTitleSource, WebConversation};

pub(crate) use background::background_task_from_runtime;

static STABLE_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct NolaStore {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DbSessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone)]
pub struct DbSessionInfoWithFirstUserMessage {
    pub id: String,
    #[allow(dead_code)]
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub first_user_message_json: Option<String>,
    pub last_user_message_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbBackgroundTask {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub(crate) process_sandbox: Option<crate::process_sandbox::FrozenProcessSandbox>,
    pub terminal: Option<nola_domain::terminal_input::TerminalSessionConfiguration>,
    pub id: String,
    pub project_root: String,
    pub command: String,
    pub cwd: String,
    pub pid: u32,
    pub stdout_path: String,
    pub stderr_path: String,
    pub status_path: String,
    pub started_at_ms: i64,
    pub owner_session_id: Option<String>,
    pub output_contract_json: Option<String>,
    pub sop_invocation: Option<nola_core::agent::ToolInvocationContext>,
}

pub type DbRuntimeTask = nola_application::RuntimeTask;
pub type DbRuntimeTaskEvent = nola_application::RuntimeTaskEvent;

#[derive(Debug, Clone)]
pub struct DbWorkspaceCheckpoint {
    pub id: String,
    pub root: String,
    pub git_root: String,
    pub revision: String,
    pub source: String,
    pub label: String,
    pub created_at: u64,
    pub status: String,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DbSessionRunLock {
    pub session_id: String,
    pub owner: String,
    pub pid: u32,
    pub process_start: String,
    pub acquired_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct AgentTraceRunRecord {
    pub trace_id: String,
    pub run_id: String,
    pub message_id: Option<String>,
    pub status: String,
    pub last_sequence: u64,
    pub terminal_reason: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl NolaStore {
    pub fn open_default() -> Result<Self, String> {
        Self::open(&default_db_path()?)
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create {} failed: {err}", parent.display()))?;
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn application(
        &self,
    ) -> Result<crate::application_port::CliApplicationPort, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)
    }
}

pub(crate) fn default_db_path() -> Result<PathBuf, String> {
    crate::platform_paths::application_database_path()
}

fn now_string() -> String {
    Local::now().to_rfc3339()
}

fn stable_id(prefix: &str) -> String {
    let now = Local::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Local::now().timestamp_micros().saturating_mul(1_000));
    let sequence = STABLE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{now}-{}-{sequence:x}", std::process::id())
}

fn stable_numeric_id(raw: &str) -> u64 {
    let digits = raw
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u64>().unwrap_or(0)
}
