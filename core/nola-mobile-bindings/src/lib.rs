// SPDX-License-Identifier: Apache-2.0
//! Stable JSON FFI for the canonical Nola Store.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use nola_application::ApplicationCommand;
use nola_application_bootstrap::ApplicationRuntime;
use nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION;
use serde_json::{json, Value};

mod child_pump;
mod process_observation;
mod edited_resend;
mod expert_template_host;
pub use edited_resend::nola_core_application_attest_resend_host_policy;
pub use child_pump::*;
mod intelligence;
mod mcp_host;
pub use mcp_host::{nola_core_mobile_mcp_host,nola_core_mobile_mcp_retain};
mod software_update;
pub use software_update::nola_core_software_update_json;
mod projection;
mod submission;
pub use intelligence::{
    nola_core_intelligence_abi_version, nola_core_intelligence_clear,
    nola_core_intelligence_ingest, nola_core_intelligence_recall,
    nola_core_intelligence_snapshot,
};
pub use projection::*;
pub use submission::nola_core_application_submit;

/// Keeps the Core C ABI reachable when Cargo links `nola_core` as an upstream
/// rlib into this final static library. Merely depending on the crate is not
/// sufficient: the native linker otherwise drops every unreferenced exported
/// symbol. The Store ABI below and these Core symbols then ship in one archive
/// while the Rust dependency direction remains bindings -> Core + Store.
#[doc(hidden)]
#[no_mangle]
pub extern "C" fn nola_mobile_bindings_force_link_core() -> usize {
    let symbols = std::hint::black_box([
        nola_core::nola_core_compose_agent_prompt_json as *const (),
        nola_core::nola_core_prompt_composer_abi_version as *const (),
        nola_core::nola_core_resource_access_abi_version as *const (),
        nola_core::nola_core_resource_access_evaluate_json as *const (),
        nola_core::nola_core_parse_agent_model_output as *const (),
        nola_core::nola_core_stitch_agent_model_continuation as *const (),
        nola_core::nola_core_context_plan as *const (),
        nola_core::nola_core_context_engine_abi_version as *const (),
        nola_core::nola_core_context_prepare as *const (),
        nola_core::nola_core_data_source_abi_version as *const (),
        nola_core::nola_core_data_source_resolve as *const (),
        nola_core::nola_core_data_source_seal_contract as *const (),
        nola_core::nola_core_lifecycle_hook_abi_version as *const (),
        nola_core::nola_core_lifecycle_hook_build_registry as *const (),
        nola_core::nola_core_lifecycle_hook_plan as *const (),
        nola_core::nola_core_lifecycle_hook_project_execution as *const (),
        nola_core::nola_core_memory_select_json as *const (),
        nola_core::nola_core_memory_selection_abi_version as *const (),
        nola_core::nola_core_prompt_cache_abi_version as *const (),
        nola_core::nola_core_prompt_cache_diagnose as *const (),
        nola_core::nola_core_tool_policy_abi_version as *const (),
        nola_core::nola_core_tool_policy_mcp_allowed as *const (),
        nola_core::nola_core_tool_policy_resolve as *const (),
        nola_core::nola_core_free_c_string as *const (),
    ]);
    symbols.iter().fold(0usize, |digest, symbol| {
        digest.rotate_left(1) ^ (*symbol as usize)
    })
}

pub const NOLA_APPLICATION_ABI_VERSION: u32 = 1;

pub struct NolaCoreApplication {
    application: Arc<EmbeddedApplication>,
}

pub(crate) struct EmbeddedApplication {
    runtime: Mutex<Option<ApplicationRuntime>>,
    database_path: PathBuf,
    read_pool: Option<nola_application_bootstrap::ReadStorePool>,
    owner_available: std::sync::atomic::AtomicBool,
    _ownership: Option<std::fs::File>,
}

impl EmbeddedApplication {
    pub(crate) fn can_serve_read_replica(&self) -> bool {
        self.owner_available
            .load(std::sync::atomic::Ordering::Acquire)
            && self
                .read_pool
                .as_ref()
                .is_some_and(|pool| pool.is_enabled())
    }

    pub(crate) fn try_execute_on_read_replica(
        &self,
        command: ApplicationCommand,
    ) -> Result<Result<Value, String>, ApplicationCommand> {
        if !command.is_read_only_query()
            || !command.can_execute_on_read_replica()
            || !self.can_serve_read_replica()
        {
            return Err(command);
        }
        let Some(pool) = self.read_pool.as_ref() else {
            return Err(command);
        };
        let Ok(mut replica) = pool.checkout() else {
            return Err(command);
        };
        Ok(nola_application::execute_read_only_query(
            &mut *replica,
            command,
        ))
    }

    pub(crate) fn execute_command(&self, command: ApplicationCommand) -> Result<Value, String> {
        let command = match self.try_execute_on_read_replica(command) {
            Ok(result) => return result,
            Err(command) => command,
        };
        let mut locked = self.lock()?;
        locked.execute(command)
    }

    pub(crate) fn lock(&self) -> Result<nola_application_bootstrap::RuntimeOwnerGuard<'_>, String> {
        match nola_application_bootstrap::acquire_owned_runtime(&self.runtime, &self.database_path)
        {
            Ok(guard) => {
                self.owner_available
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(guard)
            }
            Err(error) => {
                self.owner_available
                    .store(false, std::sync::atomic::Ordering::Release);
                if let Some(pool) = self.read_pool.as_ref() {
                    pool.clear();
                }
                Err(error)
            }
        }
    }

    pub(crate) fn reopen_locked(
        &self,
        runtime: &mut nola_application_bootstrap::RuntimeOwnerGuard<'_>,
    ) -> Result<(), String> {
        if let Some(pool) = self.read_pool.as_ref() {
            pool.clear();
        }
        match runtime.reopen() {
            Ok(()) => {
                self.owner_available
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.owner_available
                    .store(false, std::sync::atomic::Ordering::Release);
                Err(error)
            }
        }
    }
}

#[derive(Default)]
struct RuntimeRegistry {
    by_identity: HashMap<PathBuf, Weak<EmbeddedApplication>>,
    recency: Vec<Weak<EmbeddedApplication>>,
}

static RUNTIMES: OnceLock<Mutex<RuntimeRegistry>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_PINNED_APPLICATION: RefCell<Option<Weak<EmbeddedApplication>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_pinned_application(app: Option<&Arc<EmbeddedApplication>>) {
    TEST_PINNED_APPLICATION.with(|slot| {
        *slot.borrow_mut() = app.map(Arc::downgrade);
    });
}

/// Returns the most recently opened live file-backed `EmbeddedApplication` in this process.
pub(crate) fn active_embedded_application() -> Option<Arc<EmbeddedApplication>> {
    #[cfg(test)]
    if let Some(pinned) =
        TEST_PINNED_APPLICATION.with(|slot| slot.borrow().as_ref().and_then(Weak::upgrade))
    {
        return Some(pinned);
    }
    let mut registry = RUNTIMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry
        .by_identity
        .retain(|_, value| value.strong_count() > 0);
    registry.recency.retain(|value| value.strong_count() > 0);
    registry.recency.iter().rev().find_map(Weak::upgrade)
}

fn open_owned_runtime(
    path: &str,
    key: Option<&std::path::Path>,
) -> Result<Arc<EmbeddedApplication>, String> {
    let ownership = if let Some(key) = key {
        use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
        let mut lock_path = key.as_os_str().to_owned();
        lock_path.push(".service.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(PathBuf::from(lock_path))
            .map_err(|error| format!("open Application ownership: {error}"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!(
                "Application database already has a writer: {}",
                std::io::Error::last_os_error()
            ));
        }
        Some(file)
    } else {
        None
    };
    let runtime = nola_application_bootstrap::open(path)?;
    let database_path = key
        .map(std::path::Path::to_owned)
        .unwrap_or_else(|| path.into());
    let read_pool = (path != ":memory:")
        .then(|| nola_application_bootstrap::ReadStorePool::new(database_path.clone()));
    Ok(Arc::new(EmbeddedApplication {
        runtime: Mutex::new(Some(runtime)),
        database_path,
        read_pool,
        owner_available: std::sync::atomic::AtomicBool::new(true),
        _ownership: ownership,
    }))
}

thread_local! {
    static LAST_APPLICATION_OPEN_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn record_application_open_error(message: impl Into<String>) {
    let message = message.into().replace('\0', "\\0");
    LAST_APPLICATION_OPEN_ERROR.with(|slot| {
        *slot.borrow_mut() = CString::new(message).ok();
    });
}

#[no_mangle]
pub extern "C" fn nola_core_application_abi_version() -> u32 {
    NOLA_APPLICATION_ABI_VERSION
}

/// Application Service transport identity consumed by native clients.
/// Exporting it through the framework prevents Swift and Rust from opening
/// competing sockets when the protocol changes.
#[no_mangle]
pub extern "C" fn nola_core_application_service_protocol_version() -> u32 {
    APPLICATION_SERVICE_PROTOCOL_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_application_open(
    path_ptr: *const u8,
    path_len: usize,
) -> *mut NolaCoreApplication {
    let Some(path) = utf8_input(path_ptr, path_len) else {
        record_application_open_error("database path is not valid UTF-8");
        return ptr::null_mut();
    };
    // Handles for the same canonical database share both hot Core state and
    // its command-admission mutex. In-memory databases remain independent.
    let application = if path == ":memory:" {
        open_owned_runtime(&path, None)
    } else {
        let raw = PathBuf::from(&path);
        if let Some(parent) = raw.parent().filter(|value| !value.as_os_str().is_empty()) {
            if let Err(error) = std::fs::create_dir_all(parent) {
                record_application_open_error(format!("create database directory: {error}"));
                return ptr::null_mut();
            }
        }
        let key = std::fs::canonicalize(&raw).or_else(|_| {
            let parent = raw
                .parent()
                .filter(|value| !value.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            std::fs::canonicalize(parent)
                .map(|parent| parent.join(raw.file_name().unwrap_or_default()))
        });
        match key {
            Err(error) => Err(format!("canonical database path: {error}")),
            Ok(key) => {
                let mut registry = match RUNTIMES.get_or_init(Default::default).lock() {
                    Ok(value) => value,
                    Err(_) => {
                        record_application_open_error("runtime registry lock poisoned");
                        return ptr::null_mut();
                    }
                };
                registry
                    .by_identity
                    .retain(|_, value| value.strong_count() > 0);
                registry.recency.retain(|value| value.strong_count() > 0);
                use std::os::unix::fs::MetadataExt;
                let identity_key = |path: &std::path::Path| {
                    std::fs::metadata(path)
                        .map(|meta| PathBuf::from(format!("inode:{}:{}", meta.dev(), meta.ino())))
                        .unwrap_or_else(|_| path.to_owned())
                };
                if let Some(runtime) = registry
                    .by_identity
                    .get(&identity_key(&key))
                    .and_then(Weak::upgrade)
                {
                    let weak = Arc::downgrade(&runtime);
                    registry
                        .recency
                        .retain(|entry| !Weak::ptr_eq(entry, &weak));
                    registry.recency.push(weak);
                    Ok(runtime)
                } else {
                    open_owned_runtime(&path, Some(&key)).map(|runtime| {
                        let weak = Arc::downgrade(&runtime);
                        registry
                            .by_identity
                            .insert(identity_key(&key), weak.clone());
                        registry.recency.push(weak);
                        runtime
                    })
                }
            }
        }
    };
    match application {
        Ok(application) => {
            LAST_APPLICATION_OPEN_ERROR.with(|slot| *slot.borrow_mut() = None);
            Box::into_raw(Box::new(NolaCoreApplication { application }))
        }
        Err(error) => {
            record_application_open_error(error);
            ptr::null_mut()
        }
    }
}

/// Returns a borrowed diagnostic string for the most recent Application open on the
/// current thread. The caller must copy it immediately and must not free it.
#[no_mangle]
pub extern "C" fn nola_core_application_last_open_error() -> *const c_char {
    LAST_APPLICATION_OPEN_ERROR.with(|slot| {
        slot.borrow()
            .as_ref()
            .map_or(ptr::null(), |message| message.as_ptr())
    })
}

#[no_mangle]
pub extern "C" fn nola_core_application_free(application: *mut NolaCoreApplication) {
    if application.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(application));
    }
}

#[no_mangle]
pub extern "C" fn nola_core_application_execute(
    application: *mut NolaCoreApplication,
    command_ptr: *const u8,
    command_len: usize,
) -> *mut c_char {
    if application.is_null() {
        return encode(json!({
            "ok": false,
            "error": {"code": "application_unavailable", "message": "Nola Application is unavailable"}
        }));
    }
    // Reject before utf8_input copies the caller's buffer. This legacy adapter
    // still serves mobile callers while they adopt versioned submissions.
    if command_len > nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES {
        return encode(json!({
            "ok": false,
            "error": {"code": "frame_too_large", "message": "Application command exceeds the frame limit"}
        }));
    }
    let Some(command_json) = utf8_input(command_ptr, command_len) else {
        return encode(json!({
            "ok": false,
            "error": {"code": "invalid_utf8", "message": "Application command is not UTF-8"}
        }));
    };
    let command: ApplicationCommand = match serde_json::from_str(&command_json) {
        Ok(command) => command,
        Err(error) => {
            return encode(json!({
                "ok": false,
                "error": {"code": "invalid_command", "message": error.to_string()}
            }))
        }
    };
    if !command.is_read_only_query() {
        return encode(json!({"ok": false, "error": {
            "code": "receipt_required", "message": "Use the versioned command envelope submission for mutations"
        }}));
    }
    let runtime = unsafe { &*application };
    let command = match runtime.application.try_execute_on_read_replica(command) {
        Ok(Ok(value)) => return encode(json!({"ok": true, "result": value})),
        Ok(Err(error)) => {
            return encode(json!({
                "ok": false,
                "error": {"code": "application_error", "message": error}
            }))
        }
        Err(command) => command,
    };
    let mut locked = match runtime.application.lock() {
        Ok(locked) => locked,
        Err(_) => {
            return encode(json!({
                "ok": false,
                "error": {"code": "application_lock_poisoned", "message": "Nola Application lock is poisoned"}
            }))
        }
    };
    let result = locked.execute(command);
    match result {
        Ok(value) => encode(json!({"ok": true, "result": value})),
        Err(error) => encode(json!({
            "ok": false,
            "error": {"code": "application_error", "message": error}
        })),
    }
}

fn utf8_input(pointer: *const u8, length: usize) -> Option<String> {
    if pointer.is_null() || length == 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    std::str::from_utf8(bytes).ok().map(ToOwned::to_owned)
}

fn encode(value: Value) -> *mut c_char {
    let serialized = match serde_json::to_string(&value) {
        Ok(serialized) => serialized,
        Err(_) => return ptr::null_mut(),
    };
    match CString::new(serialized) {
        Ok(value) => value.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
#[path = "../../tests/support/model_identity.rs"]
mod model_fixture;

#[cfg(test)]
mod tests {
    use super::{
        nola_core_application_execute, nola_core_application_free, nola_core_application_open,
    };
    use nola_core::agent::context_window::ContextWindowConfig;
    use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{EventCursor};
    use nola_core::agent::{
        AgentCoreConfig, AgentSessionConfiguration, AgentThreadState, SessionId,
    };
    use nola_core::nola_core_free_c_string;
    use nola_store::{BeginTurnInput, NolaStore};

    #[test]
    fn oversized_mobile_command_is_rejected_before_reading_or_copying_its_buffer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("bounded.db")
            .to_string_lossy()
            .to_string();
        let handle = nola_core_application_open(path.as_ptr(), path.len());
        assert!(!handle.is_null());
        struct Handle(*mut super::NolaCoreApplication);
        impl Drop for Handle {
            fn drop(&mut self) {
                nola_core_application_free(self.0);
            }
        }
        let handle = Handle(handle);
        // Intentionally insufficient storage verifies the size guard precedes
        // slice construction. No oversized allocation is required by the test.
        let byte = [b'{'];
        for length in [
            nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES + 1,
            usize::MAX,
        ] {
            let output = nola_core_application_execute(handle.0, byte.as_ptr(), length);
            assert!(!output.is_null());
            let result: serde_json::Value =
                serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(output) }.to_bytes())
                    .unwrap();
            nola_core_free_c_string(output);
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"]["code"], "frame_too_large");
        }
        let command = br#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#;
        let output = nola_core_application_execute(handle.0, command.as_ptr(), command.len());
        let result: serde_json::Value =
            serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(output) }.to_bytes()).unwrap();
        nola_core_free_c_string(output);
        assert_eq!(
            result["ok"], true,
            "oversized input must not poison the runtime"
        );
    }

    #[test]
    fn ffi_opens_application_and_executes_product_command() {
        let root = std::env::temp_dir().join(format!("nola-store-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("product.db").to_string_lossy().to_string();
        let handle = nola_core_application_open(path.as_ptr(), path.len());
        assert!(!handle.is_null());
        let command = br#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#;
        let output = nola_core_application_execute(handle, command.as_ptr(), command.len());
        assert!(!output.is_null());
        let value = unsafe { std::ffi::CStr::from_ptr(output) }
            .to_string_lossy()
            .to_string();
        assert_eq!(value, r#"{"ok":true,"result":[]}"#);
        nola_core_free_c_string(output);
        nola_core_application_free(handle);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ffi_loads_application_session_projection_after_reopen() {
        let root = std::env::temp_dir().join(format!(
            "nola-session-projection-ffi-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("product.db");
        let conversation_id = "ffi-session-projection";
        let mut state = AgentThreadState::new(
            SessionId::from_string(conversation_id.to_string()),
            AgentSessionConfiguration {
                model: Some("ffi-model".to_string()),
                max_model_iterations: 0,
                max_strategy_recovery_attempts: 3,
                tools: Vec::new(),
                context_window: ContextWindowConfig {
                    context_window_tokens: 65_536,
                    reserved_output_tokens: 8_192,
                    ..ContextWindowConfig::default()
                },
                memory_policy: Default::default(),
                turn_identity: super::model_fixture::host_identity(),
            },
        );
        state.conversation.context_window.used_tokens = 4_096;
        state.execution.permissions.revision = 12;
        state.skill_runtime.revision = 13;
        let checkpoint = CoreCheckpoint {
            schema_version: nola_core::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: conversation_id.to_string(),
            revision: 14,
            last_event_cursor: EventCursor { sequence: 15 },
            capability_catalog_revision: 0,
            created_at_ms: 1_786_900_100_000,
            reason: CheckpointReason::Manual,
            state,
        };
        {
            let mut store = NolaStore::open(&database).unwrap();
            store
                .begin_turn(&BeginTurnInput {
                    user_part_kind: "text".into(),
                    expected_configuration_revision: None,
                    expected_parent_message_id: None,
                    initial_events: Vec::new(),
                    assistant_turn_bindings: vec![],
                    initial_assistant_part_mutations: vec![],
                    memory_usage_entries: vec![],
                    model_attempt_mutations: vec![],
                    delegated_invocation_mutations: vec![],
                    delegated_permission_mutations: vec![],
                    conversation_id: conversation_id.to_string(),
                    conversation_title: "FFI projection".to_string(),
                    user_message_id: "user".to_string(),
                    user_part_id: "user:body".to_string(),
                    user_text: "hello".to_string(),
                    user_content_json: None,
                    additional_user_parts: Vec::new(),
                    assistant_message_id: "assistant".to_string(),
                    assistant_part_id: "assistant:body".to_string(),
                    run_id: "run".to_string(),
                    context_checkpoint_id: None,
                    checkpoint_revision: 14,
                    checkpoint_event_cursor: 15,
                    checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
                    invocation_config_json: serde_json::to_string(&AgentCoreConfig {
                        turn_identity: super::model_fixture::host_identity(),
                        ..Default::default()
                    })
                        .unwrap(),
                    invocation_config_digest: "ffi-config-digest".to_string(),
                    outbox_effects: Vec::new(),
                    created_at_ms: 1_786_900_100_000,
                })
                .unwrap();
        }

        let path = database.to_string_lossy().to_string();
        let handle = nola_core_application_open(path.as_ptr(), path.len());
        assert!(!handle.is_null());
        let command = serde_json::json!({
            "type": "load_session_projection",
            "conversation_id": conversation_id,
        })
        .to_string();
        let output = nola_core_application_execute(handle, command.as_ptr(), command.len());
        assert!(!output.is_null());
        let value: serde_json::Value = serde_json::from_str(
            unsafe { std::ffi::CStr::from_ptr(output) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"]["conversation_id"], conversation_id);
        assert_eq!(value["result"]["core_revision"], 14);
        assert_eq!(value["result"]["event_cursor"], 15);
        assert_eq!(value["result"]["permission_projection"]["revision"], 12);
        assert_eq!(value["result"]["skill_runtime_projection"]["revision"], 13);
        assert_eq!(value["result"]["context_usage"]["used_tokens"], 4_096);
        assert_eq!(
            value["result"]["context_usage"]["context_window_tokens"],
            65_536
        );
        nola_core_free_c_string(output);
        nola_core_application_free(handle);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ffi_data_source_commands_share_the_canonical_store_after_reopen() {
        let root =
            std::env::temp_dir().join(format!("nola-data-source-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("product.db");
        let path = database.to_string_lossy().to_string();
        let handle = nola_core_application_open(path.as_ptr(), path.len());
        assert!(!handle.is_null());

        let execute = |command: serde_json::Value| {
            let mut arguments = command.as_object().unwrap().clone();
            let kind = arguments.remove("type").unwrap();
            let command = serde_json::json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                "client_id":"ffi-data-source", "request_id":uuid::Uuid::new_v4().to_string(),
                "command":{"type":kind,"arguments":arguments}}).to_string();
            let output =
                super::nola_core_application_submit(handle, command.as_ptr(), command.len());
            assert!(!output.is_null());
            let value: serde_json::Value = serde_json::from_str(
                unsafe { std::ffi::CStr::from_ptr(output) }
                    .to_str()
                    .unwrap(),
            )
            .unwrap();
            nola_core_free_c_string(output);
            assert_eq!(value["ok"], true, "{value}");
            value["result"]["result"].clone()
        };
        let draft = execute(serde_json::json!({
            "type": "data_source",
            "command": {
                "type": "create_draft",
                "input": {
                "id": "mobile-source",
                "name": "Mobile source",
                "kind": "local_file",
                "configuration": {
                    "connector_id": "mobile", "location_kind": "local_path",
                    "location": "/data/mobile.json",
                    "parameters": {"bookmarkReference": "mobile-source"},
                    "secret_references": []
                },
                "access_policy": "conversationOptIn",
                "authorization": {"state": "authorized", "capability": "file.read", "observed_at_ms": 100},
                "freshness_policy": {"maximum_age_minutes": 60, "allow_stale_fallback": false},
                "capabilities": ["read", "context"],
                "privacy": "local-first",
                "created_at_ms": 100,
                "updated_at_ms": 100
                }
            }
        }));
        assert_eq!(draft["lifecycle"], "draft");
        assert_eq!(draft["default_enabled"], false);
        let pending = execute(serde_json::json!({
            "type": "data_source", "command": {
                "type": "request_validation", "id": "mobile-source",
                "expected_revision": draft["revision"], "requested_at_ms": 110
            }
        }));
        let mut contract: nola_core::data_source::DataSourceContract =
            serde_json::from_value(serde_json::json!({
                "sourceId": "mobile-source", "connectorId": "mobile", "connectorVersion": 1,
                "readerDescription": "Read authorized JSON through the mobile host",
                "readRecipe": {"method": "localFileRead", "operationId": "file.read",
                    "configurationKeys": ["location"], "outputSchema": {"type": "object"}},
                "schemaFields": ["value"], "redactedSample": "{\"value\":1}",
                "recordCount": 1, "maximumAgeMinutes": 60, "validationState": "verified",
                "validatedAt": "2026-09-05T00:00:00Z"
            }))
            .unwrap();
        contract.seal_validation();
        let verified = execute(serde_json::json!({
            "type": "data_source", "command": {
                "type": "record_validation", "id": "mobile-source",
                "validation_token": pending["validation"]["validation_token"],
                "result": {"status": "succeeded", "contract": contract}, "recorded_at_ms": 120
            }
        }));
        assert_eq!(verified["validation"]["status"], "verified");
        execute(serde_json::json!({
            "type": "data_source", "command": {
                "type": "set_default_enabled", "id": "mobile-source",
                "expected_revision": verified["revision"], "enabled": true, "updated_at_ms": 130
            }
        }));
        execute(serde_json::json!({
            "type": "data_source",
            "command": {
                "type": "set_conversation_override",
                "conversation_id": "conversation-mobile",
                "source_id": "mobile-source",
                "value": "enabled",
                "updated_at_ms": 200
            }
        }));
        nola_core_application_free(handle);

        let reopened = nola_core_application_open(path.as_ptr(), path.len());
        assert!(!reopened.is_null());
        let list = serde_json::json!({
            "type": "data_source",
            "command": {"type": "list"}
        })
        .to_string();
        let output = nola_core_application_execute(reopened, list.as_ptr(), list.len());
        let value: serde_json::Value = serde_json::from_str(
            unsafe { std::ffi::CStr::from_ptr(output) }
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"][0]["id"], "mobile-source");
        assert_eq!(value["result"][0]["default_enabled"], true);
        assert_eq!(value["result"][0]["validation"]["status"], "verified");
        assert_eq!(
            value["result"][0]["validation"]["contract"]["readRecipe"]["operationId"],
            "file.read"
        );
        assert_eq!(
            value["result"][0]["configuration"]["parameters"]["bookmarkReference"],
            "mobile-source"
        );
        nola_core_free_c_string(output);
        nola_core_application_free(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn embedded_read_replica_executes_concurrently_while_writer_lock_and_wal_write_are_held() {
        use super::*;
        use std::sync::{mpsc, Arc, Barrier};
        use std::time::{Duration, Instant};

        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("mobile-wal-concurrent.sqlite");
        let db_path_str = db_path.to_string_lossy().into_owned();

        let raw_handle = nola_core_application_open(db_path_str.as_ptr(), db_path_str.len());
        assert!(!raw_handle.is_null());

        let submit = |handle: *mut NolaCoreApplication, request_id: &str, cmd_type: &str, args: Value| -> Value {
            let envelope = json!({
                "protocol_version": APPLICATION_SERVICE_PROTOCOL_VERSION,
                "request_id": request_id,
                "client_id": "mobile-test",
                "expected_revision": null,
                "command": {
                    "type": cmd_type,
                    "arguments": args,
                }
            })
            .to_string();
            let out = nola_core_application_submit(handle, envelope.as_ptr(), envelope.len());
            assert!(!out.is_null());
            let parsed: Value = serde_json::from_str(
                unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap(),
            )
            .unwrap();
            nola_core_free_c_string(out);
            parsed
        };

        let exec_raw = |handle: *mut NolaCoreApplication, cmd: Value| -> Value {
            let raw = cmd.to_string();
            let out = nola_core_application_execute(handle, raw.as_ptr(), raw.len());
            assert!(!out.is_null());
            let parsed: Value = serde_json::from_str(
                unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap(),
            )
            .unwrap();
            nola_core_free_c_string(out);
            parsed
        };

        assert_eq!(
            submit(
                raw_handle,
                "seed-setting",
                "put_product_setting",
                json!({"key": "ui.theme", "value_json": "\"dark\"", "updated_at_ms": 100})
            )["ok"],
            true
        );
        assert_eq!(
            submit(
                raw_handle,
                "seed-conv",
                "create_conversation",
                json!({
                    "input": {
                        "id": "conv-mobile-1",
                        "title": "Mobile WAL Reader",
                        "settings_json": null,
                        "created_at_ms": 100,
                        "updated_at_ms": 100
                    }
                })
            )["ok"],
            true
        );

        let embedded = Arc::clone(&(unsafe { &*raw_handle }).application);
        let writer_ready = Arc::new(Barrier::new(2));
        let (reader_done_tx, reader_done_rx) = mpsc::channel();

        let embedded_writer = Arc::clone(&embedded);
        let writer_ready_clone = Arc::clone(&writer_ready);
        let db_path_writer = db_path.clone();
        let writer_thread = std::thread::spawn(move || {
            let _writer_guard = embedded_writer.lock().unwrap();
            let wal_conn = rusqlite::Connection::open(&db_path_writer).unwrap();
            wal_conn
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     UPDATE product_settings SET value_json='\"uncommitted\"', revision=99
                     WHERE key='ui.theme';",
                )
                .unwrap();
            writer_ready_clone.wait();
            reader_done_rx.recv().unwrap();
            wal_conn.execute_batch("ROLLBACK").unwrap();
        });

        writer_ready.wait();

        let started = Instant::now();

        // 1. get_product_setting via nola_core_application_execute
        let setting = exec_raw(
            raw_handle,
            json!({"type": "get_product_setting", "key": "ui.theme"}),
        );
        assert_eq!(setting["ok"], true);
        assert_eq!(setting["result"]["value_json"], "\"dark\"");

        // 2. list_conversations via nola_core_application_submit
        let conversations = submit(
            raw_handle,
            "read-convs",
            "list_conversations",
            json!({
                "query": {
                    "project_root": null,
                    "non_empty": false,
                    "limit": 32,
                    "offset": 0
                }
            }),
        );
        assert_eq!(conversations["ok"], true);
        assert_eq!(
            conversations["result"]["result"].as_array().unwrap().len(),
            1
        );

        // 3. load_conversation_projection_window_v1 via nola_core_application_execute
        let window = exec_raw(
            raw_handle,
            json!({
                "type": "load_conversation_projection_window_v1",
                "conversation_id": "conv-mobile-1",
                "before_ordinal": null,
                "limit": 20,
                "content_byte_limit": 65536
            }),
        );
        assert_eq!(window["ok"], true);
        assert_eq!(
            window["result"]["projection"]["conversation_id"],
            "conv-mobile-1"
        );

        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(200),
            "concurrent embedded WAL reads took {elapsed:?}, expected < 200ms"
        );

        reader_done_tx.send(()).unwrap();
        writer_thread.join().unwrap();
        nola_core_application_free(raw_handle);
    }
}
