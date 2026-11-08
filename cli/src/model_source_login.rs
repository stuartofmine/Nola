// SPDX-License-Identifier: Apache-2.0
//! Explicit local Codex-owned authentication. Never reads or exports tokens,
//! starts inference, or promotes an authenticated source to an API credential.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{
    ModelAccount, ModelAccountSnapshot, ModelAccountState, ModelSourceKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Stdio},
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const IDENTITY: &str = "codex-local-owner";
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub request_id: String,
    pub account_id: String,
    pub expected_revision: Option<i64>,
    pub display_name: String,
    pub executable: String,
    pub codex_home: String,
    pub refresh_tokens: bool,
    pub acknowledge_source_changes: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handle {
    pub request_id: String,
    pub account_id: String,
    pub expected_revision: Option<i64>,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub request_id: String,
    pub state: String,
    pub authorization_url: Option<String>,
    pub account: Option<ModelAccountSnapshot>,
    pub inference_supported: bool,
}
#[derive(Serialize, Deserialize, PartialEq)]
struct Owner {
    executable: String,
    codex_home: String,
    email: Option<String>,
}
struct Entry {
    request: Start,
    owner: PathBuf,
    created: Instant,
    snapshot: Snapshot,
    running: bool,
}
type Registry = BTreeMap<(PathBuf, String), Arc<Mutex<Entry>>>;
static OPERATIONS: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    OPERATIONS.get_or_init(Mutex::default)
}
fn db_key(store: &NolaStore) -> Result<PathBuf, String> {
    let _ = store.application()?;
    std::fs::canonicalize(store.path()).map_err(|_| "source_owner_unavailable".into())
}
fn account(store: &NolaStore, id: &str) -> Result<Option<ModelAccountSnapshot>, String> {
    store.application()?.execute(ApplicationCommand::Model {
        command: ModelCommand::GetAccount { id: id.into() },
    })
}
fn current(store: &NolaStore, request: &Start) -> Result<Option<ModelAccountSnapshot>, String> {
    let value = account(store, &request.account_id)?;
    if value.as_ref().map(|v| v.revision) != request.expected_revision {
        return Err("source_account_revision_changed".into());
    }
    if value
        .as_ref()
        .is_some_and(|v| v.account.source != ModelSourceKind::Codex)
    {
        return Err("source_account_owner_changed".into());
    }
    Ok(value)
}
fn selected_path(value: &str, directory: bool) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("source_path_must_be_absolute".into());
    }
    let meta = std::fs::symlink_metadata(&path).map_err(|_| "source_path_unavailable")?;
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err("source_path_not_regular".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o022 != 0
            || (!directory && meta.permissions().mode() & 0o111 == 0)
        {
            return Err("source_path_permissions_unsafe".into());
        }
    }
    std::fs::canonicalize(path).map_err(|_| "source_path_unavailable".into())
}
fn bound_owner(store: &NolaStore, snapshot: &ModelAccountSnapshot) -> Result<Owner, String> {
    let reference = snapshot
        .account
        .credential_ref
        .as_deref()
        .filter(|v| v.starts_with("source-owner-"))
        .ok_or("source_account_not_managed")?;
    let record = HostCredentialVault::open(store.path())?
        .record(reference)?
        .filter(|v| v.identity == IDENTITY)
        .ok_or("source_owner_unavailable")?;
    serde_json::from_str(record.secret.as_deref().ok_or("source_owner_unavailable")?)
        .map_err(|_| "source_owner_invalid".into())
}
pub fn start(store: &NolaStore, mut request: Start) -> Result<Snapshot, String> {
    if !request.acknowledge_source_changes || uuid::Uuid::parse_str(&request.request_id).is_err() {
        return Err("source_login_confirmation_required".into());
    }
    ModelAccount {
        id: request.account_id.clone(),
        display_name: request.display_name.clone(),
        source: ModelSourceKind::Codex,
        credential_ref: None,
        state: ModelAccountState::Unavailable,
    }
    .validate()?;
    request.executable = selected_path(&request.executable, false)?
        .to_string_lossy()
        .into_owned();
    request.codex_home = selected_path(&request.codex_home, true)?
        .to_string_lossy()
        .into_owned();
    let previous = current(store, &request)?;
    if request.refresh_tokens {
        let owner = bound_owner(store, previous.as_ref().ok_or("source_account_missing")?)?;
        if owner.executable != request.executable || owner.codex_home != request.codex_home {
            return Err("source_account_owner_changed".into());
        }
    }
    let db = db_key(store)?;
    let mut entries = registry().lock().map_err(|_| "source_login_unavailable")?;
    entries.retain(|_, value| {
        value
            .lock()
            .is_ok_and(|entry| entry.created.elapsed() < Duration::from_secs(600))
    });
    if entries.len() >= 32 || entries.contains_key(&(db.clone(), request.request_id.clone())) {
        return Err("source_login_busy_or_reused".into());
    }
    // One source owner across every Nola DB: rotating credentials belong to Codex.
    if entries.values().any(|v| {
        v.lock()
            .is_ok_and(|e| e.owner == PathBuf::from(&request.codex_home) && e.running)
    }) {
        return Err("source_owner_busy".into());
    }
    let snapshot = Snapshot {
        request_id: request.request_id.clone(),
        state: "starting".into(),
        authorization_url: None,
        account: None,
        inference_supported: false,
    };
    let entry = Arc::new(Mutex::new(Entry {
        owner: PathBuf::from(&request.codex_home),
        request: request.clone(),
        created: Instant::now(),
        snapshot: snapshot.clone(),
        running: true,
    }));
    entries.insert((db.clone(), request.request_id.clone()), entry.clone());
    std::thread::spawn(move || {
        let result = NolaStore::open(&db).and_then(|store| run(&store, &request, &entry));
        if let Ok(mut entry) = entry.lock() {
            // Keep the owner lease until the child has actually been reaped,
            // including after UI cancellation or a completed account commit.
            entry.running = false;
            if result.is_err() {
                if entry.snapshot.state != "cancelled" {
                    entry.snapshot.state = "failed".into();
                }
                entry.snapshot.authorization_url = None;
            }
        }
    });
    Ok(snapshot)
}
pub fn inspect(store: &NolaStore, handle: Handle, cancel: bool) -> Result<Snapshot, String> {
    let entries = registry().lock().map_err(|_| "source_login_unavailable")?;
    let entry = entries
        .get(&(db_key(store)?, handle.request_id))
        .ok_or("source_login_missing")?;
    let mut entry = entry.lock().map_err(|_| "source_login_unavailable")?;
    if entry.request.account_id != handle.account_id
        || entry.request.expected_revision != handle.expected_revision
    {
        return Err("source_login_binding_changed".into());
    }
    // A completed receipt must not resurrect an account changed independently
    // after completion; pending receipts still bind the original baseline.
    let expected = entry
        .snapshot
        .account
        .as_ref()
        .map(|value| value.revision)
        .or(entry.request.expected_revision);
    if account(store, &handle.account_id)?
        .as_ref()
        .map(|value| value.revision)
        != expected
    {
        return Err("source_account_revision_changed".into());
    }
    if cancel && matches!(entry.snapshot.state.as_str(), "starting" | "pending") {
        entry.snapshot.state = "cancelled".into();
        entry.snapshot.authorization_url = None;
    }
    Ok(entry.snapshot.clone())
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn send(process: &mut Process, value: Value) -> Result<(), String> {
    let stdin = process.0.stdin.as_mut().ok_or("source_process_closed")?;
    serde_json::to_writer(&mut *stdin, &value).map_err(|_| "source_process_closed")?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|_| "source_process_closed".into())
}
fn auth_url(value: &Value) -> Result<String, String> {
    let text = value
        .as_str()
        .filter(|v| v.len() <= 16_384)
        .ok_or("source_auth_url_invalid")?;
    let url = reqwest::Url::parse(text).map_err(|_| "source_auth_url_invalid")?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("auth.openai.com" | "chatgpt.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("source_auth_url_invalid".into());
    }
    Ok(text.into())
}
fn run(store: &NolaStore, request: &Start, entry: &Arc<Mutex<Entry>>) -> Result<(), String> {
    current(store, request)?;
    let mut command = std::process::Command::new(&request.executable);
    command
        .args(["app-server", "--listen", "stdio://"])
        .env_clear();
    // Preserve only public OS runtime context, not OPENAI_API_KEY, provider
    // tokens, proxy credentials, or a second inferred source home.
    for key in [
        "HOME",
        "PATH",
        "TMPDIR",
        "LANG",
        "SYSTEMROOT",
        "USERPROFILE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut process = Process(
        command
            .env("CODEX_HOME", &request.codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "source_process_unavailable")?,
    );
    let stdout = process.0.stdout.take().ok_or("source_process_closed")?;
    let (sender, receiver) = mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            if (&mut reader)
                .take(1_048_577)
                .read_until(b'\n', &mut bytes)
                .is_err()
                || bytes.is_empty()
                || bytes.len() > 1_048_576
            {
                break;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                break;
            };
            if sender.send(value).is_err() {
                break;
            }
        }
    });
    send(
        &mut process,
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"nola","title":"Nola","version":env!("CARGO_PKG_VERSION")}}}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut login_id: Option<String> = None;
    let mut expected_id = 1;
    loop {
        let cancelled = entry
            .lock()
            .map_err(|_| "source_login_unavailable")?
            .snapshot
            .state
            == "cancelled";
        if cancelled || Instant::now() >= deadline || current(store, request).is_err() {
            if let Some(id) = &login_id {
                let _ = send(
                    &mut process,
                    json!({"id":9,"method":"account/login/cancel","params":{"loginId":id}}),
                );
            }
            return Err("source_login_cancelled_or_stale".into());
        }
        let value = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Err("source_process_closed".into()),
        };
        if value.get("error").is_some() {
            return Err("source_login_rejected".into());
        }
        if value["id"].as_u64().is_some_and(|id| id != expected_id) {
            return Err("source_protocol_response_mismatch".into());
        }
        match value["id"].as_u64() {
            Some(1) => {
                if !value["result"].is_object() {
                    return Err("source_protocol_invalid".into());
                }
                send(&mut process, json!({"method":"initialized"}))?;
                if request.refresh_tokens {
                    expected_id = 3;
                    send(
                        &mut process,
                        json!({"id":3,"method":"account/read","params":{"refreshToken":false}}),
                    )?;
                } else {
                    expected_id = 2;
                    send(
                        &mut process,
                        json!({"id":2,"method":"account/login/start","params":{"type":"chatgpt"}}),
                    )?;
                }
            }
            Some(2) => {
                expected_id = 0; // Wait for exactly the matching completion.
                let result = &value["result"];
                if result["type"] != "chatgpt" {
                    return Err("source_protocol_invalid".into());
                }
                let id = result["loginId"]
                    .as_str()
                    .filter(|v| uuid::Uuid::parse_str(v).is_ok())
                    .ok_or("source_protocol_invalid")?;
                login_id = Some(id.into());
                let url = auth_url(&result["authUrl"])?;
                let mut entry = entry.lock().map_err(|_| "source_login_unavailable")?;
                if entry.snapshot.state == "cancelled" {
                    continue;
                }
                entry.snapshot.state = "pending".into();
                entry.snapshot.authorization_url = Some(url);
            }
            Some(3) => {
                let prior = current(store, request)?.ok_or("source_account_missing")?;
                let owner = bound_owner(store, &prior)?;
                let remote = &value["result"]["account"];
                if remote["type"] != "chatgpt" || owner.email.as_deref() != remote["email"].as_str()
                {
                    return Err("source_login_identity_changed".into());
                }
                expected_id = 4;
                send(
                    &mut process,
                    json!({"id":4,"method":"account/read","params":{"refreshToken":true}}),
                )?;
            }
            Some(4) => {
                return complete(store, request, entry, &value["result"]["account"]);
            }
            _ if value["method"] == "account/login/completed" => {
                if expected_id != 0
                    || login_id.as_deref() != value["params"]["loginId"].as_str()
                    || login_id.is_none()
                {
                    return Err("source_login_identity_changed".into());
                }
                if value["params"]["success"] != true {
                    return Err("source_login_rejected".into());
                }
                expected_id = 4;
                send(
                    &mut process,
                    json!({"id":4,"method":"account/read","params":{"refreshToken":false}}),
                )?;
            }
            _ => {} // Ignore notifications; never execute a child-initiated request.
        }
    }
}
fn complete(
    store: &NolaStore,
    request: &Start,
    entry: &Arc<Mutex<Entry>>,
    remote: &Value,
) -> Result<(), String> {
    if remote["type"] != "chatgpt" {
        return Err("source_login_not_authenticated".into());
    }
    let email = remote["email"].as_str().map(str::to_owned);
    if email.as_ref().is_some_and(|v| v.len() > 512) {
        return Err("source_protocol_invalid".into());
    }
    let mut entry = entry.lock().map_err(|_| "source_login_unavailable")?;
    if entry.snapshot.state == "cancelled" {
        return Err("source_login_cancelled".into());
    }
    let previous = current(store, request)?;
    if request.refresh_tokens {
        let owner = bound_owner(store, previous.as_ref().ok_or("source_account_missing")?)?;
        // Email is only a change detector, never proof of identity/entitlement.
        if owner.email != email {
            return Err("source_login_identity_changed".into());
        }
    }
    let owner = Owner {
        executable: request.executable.clone(),
        codex_home: request.codex_home.clone(),
        email,
    };
    let existing_handle = if request.refresh_tokens {
        previous
            .as_ref()
            .and_then(|v| v.account.credential_ref.clone())
    } else {
        None
    };
    let new_slot = existing_handle.is_none();
    let handle =
        existing_handle.unwrap_or_else(|| format!("source-owner-{}", uuid::Uuid::new_v4()));
    if new_slot {
        HostCredentialVault::open(store.path())?.write(
            &handle,
            IDENTITY,
            0,
            Some(&serde_json::to_string(&owner).map_err(|_| "source_owner_invalid")?),
            || current(store, request).map(|_| ()),
        )?;
    }
    let result: Result<ModelAccountSnapshot, String> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::SaveAccount {
                input: ModelAccount {
                    id: request.account_id.clone(),
                    display_name: request.display_name.clone(),
                    source: ModelSourceKind::Codex,
                    credential_ref: Some(handle.clone()),
                    state: ModelAccountState::Unavailable,
                },
                expected_revision: request.expected_revision,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        });
    let account = match result {
        Ok(saved) => saved,
        Err(error) => {
            // Only remove the unique slot created by THIS failed operation.
            // Never touch Codex credentials or any previously bound Nola slot.
            if new_slot
                && account(store, &request.account_id).is_ok_and(|saved| {
                    saved
                        .as_ref()
                        .and_then(|v| v.account.credential_ref.as_deref())
                        != Some(handle.as_str())
                })
            {
                let _ = HostCredentialVault::open(store.path())
                    .and_then(|mut vault| vault.purge_deleted_endpoint(&handle));
            }
            return Err(error);
        }
    };
    entry.snapshot.account = Some(account);
    entry.snapshot.state = "authenticated_source_only".into();
    entry.snapshot.authorization_url = None;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn fixture(delay: bool) -> (tempfile::TempDir, NolaStore, Start) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("source-home");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = root.path().join("fixture-codex");
        let script = format!(
            r##"#!/bin/sh
IFS= read -r line
echo '{{"id":1,"result":{{"userAgent":"fixture"}}}}'
IFS= read -r line
IFS= read -r line
case "$line" in
*'"id":3'*)
echo '{{"id":3,"result":{{"account":{{"type":"chatgpt","email":"fixture@example.invalid"}}}}}}'
IFS= read -r line
;;
*account/login/start*)
echo '{{"id":2,"result":{{"type":"chatgpt","loginId":"00000000-0000-4000-8000-000000000001","authUrl":"https://auth.openai.com/fixture"}}}}'
{}
echo '{{"method":"account/login/completed","params":{{"loginId":"00000000-0000-4000-8000-000000000001","success":true}}}}'
IFS= read -r line
;;
esac
echo '{{"id":4,"result":{{"account":{{"type":"chatgpt","email":"fixture@example.invalid","planType":"fixture"}}}}}}'
IFS= read -r line
"##,
            if delay { "/bin/sleep 2" } else { "" }
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let request = Start {
            request_id: uuid::Uuid::new_v4().to_string(),
            account_id: "fixture-source".into(),
            expected_revision: None,
            display_name: "Fixture Codex".into(),
            executable: executable.to_string_lossy().into_owned(),
            codex_home: home.to_string_lossy().into_owned(),
            refresh_tokens: false,
            acknowledge_source_changes: true,
        };
        (root, store, request)
    }
    fn handle(request: &Start) -> Handle {
        Handle {
            request_id: request.request_id.clone(),
            account_id: request.account_id.clone(),
            expected_revision: request.expected_revision,
        }
    }
    fn wait(store: &NolaStore, request: &Start, pending: bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = inspect(store, handle(request), false).unwrap();
            if (pending && result.state == "pending")
                || !matches!(result.state.as_str(), "starting" | "pending")
            {
                return result;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn managed_login_and_refresh_keep_tokens_in_source_and_inference_disabled() {
        let (_root, store, mut request) = fixture(false);
        start(&store, request.clone()).unwrap();
        let result = wait(&store, &request, false);
        assert_eq!(result.state, "authenticated_source_only");
        assert!(!result.inference_supported);
        let saved = result.account.unwrap();
        assert_eq!(saved.account.state, ModelAccountState::Unavailable);
        let owner = bound_owner(&store, &saved).unwrap();
        assert_eq!(owner.email.as_deref(), Some("fixture@example.invalid"));
        let old_handle = handle(&request);
        request.request_id = uuid::Uuid::new_v4().to_string();
        request.expected_revision = Some(saved.revision);
        request.refresh_tokens = true;
        start(&store, request.clone()).unwrap();
        let refreshed = wait(&store, &request, false);
        assert_eq!(refreshed.state, "authenticated_source_only");
        let refreshed = refreshed.account.unwrap();
        assert_eq!(refreshed.revision, saved.revision + 1);
        assert_eq!(
            refreshed.account.credential_ref,
            saved.account.credential_ref
        );
        assert!(!serde_json::to_string(&owner).unwrap().contains("token"));
        assert!(inspect(&store, old_handle, false).is_err());
    }
    #[test]
    fn cancellation_and_revision_changes_reject_late_login_results() {
        for cancel in [true, false] {
            let (_root, store, request) = fixture(true);
            start(&store, request.clone()).unwrap();
            assert_eq!(wait(&store, &request, true).state, "pending");
            let mut concurrent = request.clone();
            concurrent.request_id = uuid::Uuid::new_v4().to_string();
            assert!(start(&store, concurrent).is_err());
            let mut wrong = handle(&request);
            wrong.account_id = "different-owner".into();
            assert!(inspect(&store, wrong, true).is_err());
            if cancel {
                assert_eq!(
                    inspect(&store, handle(&request), true).unwrap().state,
                    "cancelled"
                );
            } else {
                let _: ModelAccountSnapshot = store
                    .application()
                    .unwrap()
                    .execute(ApplicationCommand::Model {
                        command: ModelCommand::SaveAccount {
                            input: ModelAccount {
                                id: request.account_id.clone(),
                                display_name: "Concurrent".into(),
                                source: ModelSourceKind::Codex,
                                credential_ref: None,
                                state: ModelAccountState::Disconnected,
                            },
                            expected_revision: None,
                            updated_at_ms: 1,
                        },
                    })
                    .unwrap();
                assert!(inspect(&store, handle(&request), false).is_err());
            }
            std::thread::sleep(Duration::from_millis(100));
            let saved = account(&store, &request.account_id).unwrap();
            if cancel {
                assert!(saved.is_none());
            } else {
                assert_eq!(saved.unwrap().account.display_name, "Concurrent");
            }
        }
    }
    #[test]
    fn confirmation_owner_and_redirect_boundaries_are_exact() {
        let (_root, store, mut request) = fixture(false);
        request.acknowledge_source_changes = false;
        assert!(start(&store, request.clone()).is_err());
        request.acknowledge_source_changes = true;
        request.refresh_tokens = true;
        assert!(start(&store, request.clone()).is_err());
        for url in [
            "http://auth.openai.com/a",
            "https://auth.openai.com.evil.invalid/a",
            "https://user@auth.openai.com/a",
            "https://auth.openai.com:444/a",
        ] {
            assert!(auth_url(&json!(url)).is_err());
        }
        assert!(auth_url(&json!("https://auth.openai.com/fixture")).is_ok());
    }
}
