// SPDX-License-Identifier: Apache-2.0
//! Model-facing language service configuration projection and query planning.
use super::*;
use nola_domain::language_service::{LanguageServiceConfiguration, validate_language_services};

pub(super) fn list(context: &ToolContext) -> ToolOutcome {
    if let Err(message) = validate_language_services(&context.language_services) { return error(message); }
    let mut services = Vec::new();
    for service in &context.language_services {
        let fingerprint = match service.fingerprint() {
            Ok(value) => value,
            Err(message) => return error(message),
        };
        services.push(json!({"id":service.id,"project_root":service.project_root,
            "language_ids":service.language_ids,"configuration_fingerprint":fingerprint,
            "platform_supported":cfg!(unix)}));
    }
    let data = json!({"services":services,"configuration_is_authorization":false});
    let mut result = ok(data.to_string(), format!("{} configured language services", services.len()));
    result.data = Some(data);
    result
}

pub(super) fn configured_service<'a>(context: &'a ToolContext, input: &Value)
    -> Result<&'a LanguageServiceConfiguration, String> {
    validate_language_services(&context.language_services)?;
    let id = input.get("service_id").and_then(Value::as_str).ok_or("service_id is required")?;
    let fingerprint = input.get("configuration_fingerprint").and_then(Value::as_str)
        .ok_or("configuration_fingerprint is required; inspect language_service_list")?;
    let service = context.language_services.iter().find(|service| service.id == id)
        .ok_or("language service is not configured in this invocation")?;
    if service.fingerprint()? != fingerprint {
        return Err("language service configuration changed; rediscover before requesting authorization".into());
    }
    Ok(service)
}

pub(super) struct QueryPlan {
    pub service: LanguageServiceConfiguration,
    pub executable: PathBuf,
    pub root: PathBuf,
    pub file: PathBuf,
    pub language: String,
    pub supporting_documents: Vec<(PathBuf, String)>,
}

const MAX_SUPPORTING_DOCUMENTS: usize = 16;
const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

fn add_snapshot_bytes(current: usize, next: usize) -> Result<usize, String> {
    current.checked_add(next).filter(|total| *total <= MAX_SNAPSHOT_BYTES)
        .ok_or_else(|| "language service snapshots exceed 4 MiB".into())
}

fn verify_supporting_snapshots(documents: &[(PathBuf, String, String)],
    read: impl Fn(&Path) -> Result<String, String>) -> Result<(), String> {
    for (path, _, text) in documents {
        if read(path)? != *text {
            return Err("language service response is stale: supporting document changed".into());
        }
    }
    Ok(())
}

pub(super) fn query_plan(context: &ToolContext, input: &Value) -> Result<QueryPlan, String> {
    if !cfg!(unix) { return Err("language service execution is unavailable on this platform".into()); }
    validate_query_parameters(input)?;
    let service = configured_service(context, input)?.clone();
    let language = input.get("language_id").and_then(Value::as_str).ok_or("language_id is required")?;
    if !service.language_ids.iter().any(|id| id == language) {
        return Err("language is not configured for this service".into());
    }
    let path = Path::new(input.get("path").and_then(Value::as_str).ok_or("absolute document path is required")?);
    if !path.is_absolute() { return Err("language service document path must be absolute".into()); }
    let root = service.project_root.canonicalize().map_err(|error| format!("Resolve language service root: {error}"))?;
    let file = path.canonicalize().map_err(|error| format!("Resolve language service document: {error}"))?;
    let executable = service.executable.canonicalize().map_err(|error| format!("Resolve language service executable: {error}"))?;
    if !root.is_dir() || !file.is_file() || !file.starts_with(&root) || !executable.is_file() {
        return Err("language service requires a valid executable and document inside its configured project".into());
    }
    let mut supporting_documents = Vec::new();
    if let Some(value) = input.get("supporting_documents") {
        let documents = value.as_array().ok_or("supporting_documents must be an array")?;
        if documents.len() > MAX_SUPPORTING_DOCUMENTS { return Err("too many supporting documents (maximum 16)".into()); }
        let mut seen = std::collections::HashSet::from([file.clone()]);
        for document in documents {
            let object = document.as_object().ok_or("supporting document must be an object")?;
            if object.len() != 2 { return Err("supporting document requires only path and language_id".into()); }
            let path = Path::new(object.get("path").and_then(Value::as_str).ok_or("supporting document path is required")?);
            let language = object.get("language_id").and_then(Value::as_str).ok_or("supporting document language_id is required")?;
            if !path.is_absolute() || !service.language_ids.iter().any(|id| id == language) {
                return Err("supporting document needs an absolute path and configured language".into());
            }
            let path = path.canonicalize().map_err(|error| format!("Resolve supporting document: {error}"))?;
            if !path.is_file() || !path.starts_with(&root) || !seen.insert(path.clone()) {
                return Err("supporting document must be unique and inside the configured project".into());
            }
            supporting_documents.push((path, language.to_owned()));
        }
    }
    Ok(QueryPlan { service, executable, root, file, language: language.into(), supporting_documents })
}

// Shared by permission resolution and execution. Invalid queries must not
// reach process approval or launch just to fail parameter validation later.
fn validate_query_parameters(input: &Value) -> Result<(), String> {
    match input.get("operation").and_then(Value::as_str) {
        Some("workspace_symbols") => {
            let query = input.get("query").and_then(Value::as_str)
                .ok_or("workspace_symbols requires query")?;
            if query.chars().count() > 1024 { return Err("workspace symbol query exceeds 1024 characters".into()); }
        }
        Some("document_symbols") => {},
        Some("definitions" | "references" | "hover" | "implementations") => {
            for key in ["line", "character"] {
                if input.get(key).and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok()).filter(|value| *value > 0).is_none() {
                    return Err(format!("{key} must be a positive 32-bit integer"));
                }
            }
        }
        _ => return Err("unsupported language service operation".into()),
    }
    Ok(())
}

pub(super) fn query(args: &Map<String, Value>, context: &ToolContext) -> ToolOutcome {
    #[cfg(not(unix))]
    { let _ = (args, context); error("language service execution is unavailable on this platform".into()) }
    #[cfg(unix)]
    {
        match execute_query(args, context) {
            Ok(data) => { let mut result = ok(data.to_string(), "Language service query completed".into()); result.data = Some(data); result }
            Err(message) => error(message),
        }
    }
}

#[cfg(unix)]
const LSP_QUERY_POOL_IDLE_TTL: Duration = Duration::from_secs(300);
#[cfg(unix)]
const LSP_QUERY_POOL_MAX_SESSIONS: usize = 8;

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LspPoolKey {
    workspace_root: PathBuf,
    server_argv: Vec<String>,
    environment: Vec<(String, String)>,
}

#[cfg(unix)]
impl LspPoolKey {
    fn from_plan(plan: &QueryPlan) -> Self {
        let mut server_argv = Vec::with_capacity(1 + plan.service.arguments.len());
        server_argv.push(plan.executable.to_string_lossy().into_owned());
        server_argv.extend(plan.service.arguments.iter().cloned());
        let environment = plan
            .service
            .environment
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Self {
            workspace_root: plan.root.clone(),
            server_argv,
            environment,
        }
    }
}

#[cfg(unix)]
type ActiveCancellation = Arc<Mutex<Option<super::language_service_transport::Cancellation>>>;

#[cfg(unix)]
struct ActiveCancellationGuard {
    slot: ActiveCancellation,
}

#[cfg(unix)]
impl ActiveCancellationGuard {
    fn install(
        slot: &ActiveCancellation,
        cancellation: super::language_service_transport::Cancellation,
    ) -> Self {
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(cancellation);
        }
        Self {
            slot: Arc::clone(slot),
        }
    }
}

#[cfg(unix)]
impl Drop for ActiveCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.slot.lock() {
            *guard = None;
        }
    }
}

#[cfg(unix)]
struct OpenDocumentEntry {
    language_id: String,
    document: super::language_service_document::Document,
}

#[cfg(unix)]
enum PooledQueryError {
    Transport(String),
    Query(String),
}

#[cfg(unix)]
impl PooledQueryError {
    fn into_message(self) -> String {
        match self {
            Self::Transport(message) | Self::Query(message) => message,
        }
    }
}

#[cfg(unix)]
struct PooledLspSession {
    pid: u32,
    protocol: super::language_service_protocol::Protocol,
    transport: super::language_service_transport::Transport,
    cancellation: ActiveCancellation,
    open_documents: HashMap<PathBuf, OpenDocumentEntry>,
    last_used: Instant,
    healthy: bool,
}

#[cfg(unix)]
impl PooledLspSession {
    fn spawn(
        plan: &QueryPlan,
        deadline: Instant,
        cancellation: super::language_service_transport::Cancellation,
    ) -> Result<Self, String> {
        use super::language_service_protocol::{Incoming, Protocol};
        use super::language_service_transport::{Cancellation, Transport};
        use std::process::{Command, Stdio};

        let root_uri = url::Url::from_directory_path(&plan.root)
            .map_err(|_| "language service root must be absolute")?;
        let mut command = Command::new(&plan.executable);
        command
            .args(&plan.service.arguments)
            .current_dir(&plan.root)
            .env_clear()
            .envs(&plan.service.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = crate::task_runtime::OwnedShellChild::spawn_owned(&mut command)
            .map_err(|error| format!("Start language service: {error}"))?;
        let pid = child.id();

        let cancellation_slot: ActiveCancellation = Arc::new(Mutex::new(None));
        let poll_slot = Arc::clone(&cancellation_slot);
        let poll_cancellation: Cancellation = Arc::new(move || {
            poll_slot
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(|cb| cb()))
                .unwrap_or(false)
        });
        let _guard = ActiveCancellationGuard::install(&cancellation_slot, cancellation);
        let transport =
            Transport::attach(child, deadline, poll_cancellation).map_err(|error| error.to_string())?;
        let mut session = Self {
            pid,
            protocol: Protocol::default(),
            transport,
            cancellation: cancellation_slot,
            open_documents: HashMap::new(),
            last_used: Instant::now(),
            healthy: true,
        };
        let initialize = session.protocol.initialize(root_uri.as_str())?;
        session
            .send_raw(&initialize, deadline)
            .map_err(PooledQueryError::into_message)?;
        let Incoming::Initialized(notification) = session
            .await_response(deadline)
            .map_err(PooledQueryError::into_message)?
        else {
            return Err("language service did not initialize".into());
        };
        session
            .send_raw(&notification, deadline)
            .map_err(PooledQueryError::into_message)?;
        session.protocol.require_document_sync()?;
        session.last_used = Instant::now();
        Ok(session)
    }

    fn is_usable(&self) -> bool {
        if !self.healthy || self.last_used.elapsed() >= LSP_QUERY_POOL_IDLE_TTL {
            return false;
        }
        unsafe { libc::kill(self.pid as libc::pid_t, 0) == 0 }
    }

    fn send_raw(&mut self, message: &Value, deadline: Instant) -> Result<(), PooledQueryError> {
        self.transport.send(message, deadline).map_err(|error| {
            self.healthy = false;
            PooledQueryError::Transport(error.to_string())
        })
    }

    fn await_response(
        &mut self,
        deadline: Instant,
    ) -> Result<super::language_service_protocol::Incoming, PooledQueryError> {
        use super::language_service_protocol::Incoming;

        for _ in 0..4096 {
            let message = self.transport.receive(deadline).map_err(|error| {
                self.healthy = false;
                PooledQueryError::Transport(error.to_string())
            })?;
            match self.protocol.receive(message) {
                Ok(Incoming::Reply(reply)) => self.send_raw(&reply, deadline)?,
                Ok(Incoming::Notification) => {}
                Ok(Incoming::Failure(error)) => {
                    return Err(PooledQueryError::Query(format!(
                        "language service query failed: {error}"
                    )));
                }
                Ok(response) => return Ok(response),
                Err(error) => {
                    self.healthy = false;
                    return Err(PooledQueryError::Transport(error));
                }
            }
        }
        self.protocol.abort();
        self.healthy = false;
        Err(PooledQueryError::Transport(
            "language service unsolicited-message limit exceeded".into(),
        ))
    }

    fn close_unlisted_documents(
        &mut self,
        keep: &HashSet<PathBuf>,
        deadline: Instant,
    ) -> Result<(), PooledQueryError> {
        let to_close = self
            .open_documents
            .keys()
            .filter(|path| !keep.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in to_close {
            if let Some(entry) = self.open_documents.remove(&path) {
                self.send_raw(&entry.document.close(), deadline)?;
            }
        }
        Ok(())
    }

    fn sync_document(
        &mut self,
        path: &Path,
        language_id: &str,
        text: String,
        deadline: Instant,
    ) -> Result<bool, PooledQueryError> {
        use super::language_service_document::Document;

        let same_language = self
            .open_documents
            .get(path)
            .is_some_and(|entry| entry.language_id == language_id);
        if same_language {
            let change = self
                .open_documents
                .get_mut(path)
                .expect("checked open document")
                .document
                .change(text)
                .map_err(PooledQueryError::Query)?;
            if let Some(message) = change {
                self.send_raw(&message, deadline)?;
                return Ok(true);
            }
            return Ok(false);
        }
        if let Some(old) = self.open_documents.remove(path) {
            self.send_raw(&old.document.close(), deadline)?;
        }
        let (document, open) =
            Document::open(path, language_id, text).map_err(PooledQueryError::Query)?;
        self.send_raw(&open, deadline)?;
        self.open_documents.insert(
            path.to_path_buf(),
            OpenDocumentEntry {
                language_id: language_id.to_owned(),
                document,
            },
        );
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute(
        &mut self,
        plan: &QueryPlan,
        operation: super::language_service_protocol::Query,
        line: u32,
        character: u32,
        symbol_query: Option<&str>,
        primary_text: String,
        supporting: &[(PathBuf, String, String)],
        deadline: Instant,
        cancellation: super::language_service_transport::Cancellation,
        read_document: &dyn Fn(&Path) -> Result<String, String>,
    ) -> Result<Value, PooledQueryError> {
        use super::language_service_protocol::{Incoming, Query};

        let _guard = ActiveCancellationGuard::install(&self.cancellation, cancellation);
        let mut keep = HashSet::with_capacity(1 + supporting.len());
        keep.insert(plan.file.clone());
        for (path, _, _) in supporting {
            keep.insert(path.clone());
        }
        self.close_unlisted_documents(&keep, deadline)?;
        self.sync_document(&plan.file, &plan.language, primary_text, deadline)?;

        for (path, language, text) in supporting {
            if !self.protocol.supports(Query::DocumentSymbols) {
                return Err(PooledQueryError::Query(
                    "language service cannot synchronize supporting documents".into(),
                ));
            }
            let synced = self.sync_document(path, language, text.clone(), deadline)?;
            if synced {
                let uri = self
                    .open_documents
                    .get(path)
                    .expect("synced supporting document")
                    .document
                    .identity()
                    .uri;
                let barrier = self
                    .protocol
                    .query(Query::DocumentSymbols, json!({"textDocument":{"uri":uri}}))
                    .map_err(PooledQueryError::Query)?;
                self.send_raw(&barrier, deadline)?;
                if !matches!(self.await_response(deadline)?, Incoming::Result(_)) {
                    self.healthy = false;
                    return Err(PooledQueryError::Transport(
                        "unexpected supporting document synchronization response".into(),
                    ));
                }
            }
        }

        let snapshot = self
            .open_documents
            .get(&plan.file)
            .expect("synced primary document")
            .document
            .identity();
        if operation == Query::WorkspaceSymbols && self.protocol.supports(Query::DocumentSymbols) {
            let barrier = self
                .protocol
                .query(
                    Query::DocumentSymbols,
                    json!({"textDocument":{"uri":snapshot.uri}}),
                )
                .map_err(PooledQueryError::Query)?;
            self.send_raw(&barrier, deadline)?;
            if !matches!(self.await_response(deadline)?, Incoming::Result(_)) {
                self.healthy = false;
                return Err(PooledQueryError::Transport(
                    "unexpected document synchronization response phase".into(),
                ));
            }
        }

        let mut params = match operation {
            Query::WorkspaceSymbols => {
                let query_text = symbol_query
                    .ok_or_else(|| PooledQueryError::Query("workspace symbol query is required".into()))?;
                json!({"query": query_text})
            }
            Query::DocumentSymbols => json!({"textDocument":{"uri":snapshot.uri}}),
            _ => {
                let position = self
                    .open_documents
                    .get(&plan.file)
                    .expect("synced primary document")
                    .document
                    .position(line, character)
                    .map_err(PooledQueryError::Query)?;
                json!({"textDocument":{"uri":snapshot.uri}, "position":position})
            }
        };
        if operation == Query::References {
            params["context"] = json!({"includeDeclaration": true});
        }
        let request = self
            .protocol
            .query(operation, params)
            .map_err(PooledQueryError::Query)?;
        self.send_raw(&request, deadline)?;
        let Incoming::Result(result) = self.await_response(deadline)? else {
            self.healthy = false;
            return Err(PooledQueryError::Transport(
                "unexpected language service response phase".into(),
            ));
        };

        let latest_primary = read_document(&plan.file).map_err(PooledQueryError::Query)?;
        let primary_doc = &self
            .open_documents
            .get(&plan.file)
            .expect("synced primary document")
            .document;
        if !primary_doc.matches(&snapshot, &latest_primary) {
            return Err(PooledQueryError::Query(
                "language service response is stale: queried document changed".into(),
            ));
        }
        verify_supporting_snapshots(supporting, read_document).map_err(PooledQueryError::Query)?;

        let primary_evidence = primary_doc.evidence();
        let mut documents = vec![primary_evidence.clone()];
        for (path, _, _) in supporting {
            if let Some(entry) = self.open_documents.get(path) {
                documents.push(entry.document.evidence());
            }
        }
        self.last_used = Instant::now();
        Ok(json!({
            "code_evidence": primary_evidence,
            "result": result,
            "opened_documents": documents,
            "index_scope": "server_index_plus_explicit_documents",
            "workspace_snapshot_verified": false,
        }))
    }
}

#[cfg(unix)]
static LSP_QUERY_POOL: OnceLock<Mutex<HashMap<LspPoolKey, PooledLspSession>>> = OnceLock::new();

#[cfg(unix)]
fn acquire_pooled_session(key: &LspPoolKey) -> Option<PooledLspSession> {
    let pool = LSP_QUERY_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut evicted = Vec::new();
    let taken = {
        let Ok(mut guard) = pool.lock() else {
            return None;
        };
        let stale_keys = guard
            .iter()
            .filter(|(_, session)| !session.is_usable())
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        for stale_key in stale_keys {
            if let Some(session) = guard.remove(&stale_key) {
                evicted.push(session);
            }
        }
        guard.remove(key)
    };
    drop(evicted);
    taken
}

#[cfg(unix)]
fn release_pooled_session(key: LspPoolKey, mut session: PooledLspSession) {
    if !session.healthy {
        drop(session);
        return;
    }
    session.last_used = Instant::now();
    let pool = LSP_QUERY_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut evicted = Vec::new();
    {
        let Ok(mut guard) = pool.lock() else {
            drop(session);
            return;
        };
        let stale_keys = guard
            .iter()
            .filter(|(_, existing)| !existing.is_usable())
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        for stale_key in stale_keys {
            if let Some(existing) = guard.remove(&stale_key) {
                evicted.push(existing);
            }
        }
        while guard.len() >= LSP_QUERY_POOL_MAX_SESSIONS && !guard.contains_key(&key) {
            let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, existing)| existing.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(existing) = guard.remove(&oldest_key) {
                evicted.push(existing);
            }
        }
        if let Some(replaced) = guard.insert(key, session) {
            evicted.push(replaced);
        }
    }
    drop(evicted);
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn execute_pooled_query(
    plan: &QueryPlan,
    operation: super::language_service_protocol::Query,
    line: u32,
    character: u32,
    symbol_query: Option<&str>,
    text: String,
    supporting: &[(PathBuf, String, String)],
    deadline: Instant,
    cancellation: &RuntimeExecutionControl,
    read_document: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<Value, String> {
    use super::language_service_transport::Cancellation;

    let pool_key = LspPoolKey::from_plan(plan);
    let make_cancellation = || -> Cancellation {
        let cancellation = cancellation.clone();
        Arc::new(move || cancellation.is_requested())
    };
    let (mut session, reused_from_pool) = match acquire_pooled_session(&pool_key) {
        Some(session) => (session, true),
        None => (
            PooledLspSession::spawn(plan, deadline, make_cancellation())?,
            false,
        ),
    };
    let attempt = session.execute(
        plan,
        operation,
        line,
        character,
        symbol_query,
        text.clone(),
        supporting,
        deadline,
        make_cancellation(),
        read_document,
    );
    let mut data = match attempt {
        Ok(data) => {
            release_pooled_session(pool_key, session);
            data
        }
        Err(PooledQueryError::Transport(_))
            if reused_from_pool
                && !cancellation.is_requested()
                && Instant::now() < deadline =>
        {
            drop(session);
            let mut fresh = PooledLspSession::spawn(plan, deadline, make_cancellation())?;
            match fresh.execute(
                plan,
                operation,
                line,
                character,
                symbol_query,
                text,
                supporting,
                deadline,
                make_cancellation(),
                read_document,
            ) {
                Ok(data) => {
                    release_pooled_session(pool_key, fresh);
                    data
                }
                Err(error) => {
                    if fresh.healthy {
                        release_pooled_session(pool_key, fresh);
                    } else {
                        drop(fresh);
                    }
                    return Err(error.into_message());
                }
            }
        }
        Err(error) => {
            if session.healthy {
                release_pooled_session(pool_key, session);
            } else {
                drop(session);
            }
            return Err(error.into_message());
        }
    };
    data["service_id"] = json!(plan.service.id);
    data["configuration_fingerprint"] = json!(plan.service.fingerprint()?);
    Ok(data)
}

#[cfg(unix)]
fn execute_query(args: &Map<String, Value>, context: &ToolContext) -> Result<Value, String> {
    use super::language_service_protocol::Query;
    use std::io::Read;
    let identity = current_local_execution_identity().ok_or("language service requires an admitted execution identity")?;
    if context.permission_scope.owner_session_id().as_deref() != Some(identity.session_id.as_str()) {
        return Err("language service execution belongs to another conversation".into());
    }
    let plan = query_plan(context, &Value::Object(args.clone()))?;
    let operation = match args.get("operation").and_then(Value::as_str) {
        Some("definitions") => Query::Definition,
        Some("references") => Query::References,
        Some("hover") => Query::Hover,
        Some("document_symbols") => Query::DocumentSymbols,
        Some("workspace_symbols") => Query::WorkspaceSymbols,
        Some("implementations") => Query::Implementation,
        _ => return Err("unsupported language service operation".into()),
    };
    let position = |key: &str| -> Result<u32, String> {
        args.get(key).and_then(Value::as_u64).and_then(|value|u32::try_from(value).ok())
            .filter(|value|*value > 0).ok_or_else(||format!("{key} must be a positive integer"))
    };
    let (line, character) = if matches!(operation, Query::DocumentSymbols | Query::WorkspaceSymbols) {
        (1, 1)
    } else { (position("line")?, position("character")?) };
    let symbol_query = args.get("query").and_then(Value::as_str);
    if operation == Query::WorkspaceSymbols && symbol_query.is_none() {
        return Err("workspace_symbols requires query".into());
    }
    let read_document = |file: &Path| -> Result<String, String> {
        let path = resolve_allowed(file.to_str().ok_or("document path is not UTF-8")?, &context.allowed_dirs, true)
            .map_err(|outcome|outcome.text)?;
        if path != file { return Err("language service document identity changed".into()); }
        let mut bytes = Vec::new();
        fs::File::open(&path).map_err(|error|error.to_string())?
            .take((super::language_service_document::MAX_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes).map_err(|error|error.to_string())?;
        if bytes.len() > super::language_service_document::MAX_DOCUMENT_BYTES { return Err("document exceeds language service limit".into()); }
        String::from_utf8(bytes).map_err(|_| "language service document must be UTF-8".into())
    };
    let text = read_document(&plan.file)?;
    let mut total_bytes = text.len();
    let mut supporting = Vec::new();
    for (path, language) in &plan.supporting_documents {
        let text = read_document(path)?;
        total_bytes = add_snapshot_bytes(total_bytes, text.len())?;
        supporting.push((path.clone(), language.clone(), text));
    }
    if !matches!(operation, Query::DocumentSymbols | Query::WorkspaceSymbols) {
        super::language_service_document::position(&text, line, character)?;
    }
    if context.cancellation.is_requested() { return Err("language service query cancelled before launch".into()); }
    let deadline = Instant::now() + Duration::from_millis(plan.service.timeout_ms);
    execute_pooled_query(
        &plan,
        operation,
        line,
        character,
        symbol_query,
        text,
        &supporting,
        deadline,
        &context.cancellation,
        &read_document,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supporting_snapshots_reject_changed_or_unreadable_files() {
        let snapshots = vec![(PathBuf::from("/a"),"rust".into(),"original".into())];
        assert!(verify_supporting_snapshots(&snapshots, |_| Ok("original".into())).is_ok());
        assert!(verify_supporting_snapshots(&snapshots, |_| Ok("changed".into())).unwrap_err().contains("stale"));
        assert_eq!(verify_supporting_snapshots(&snapshots, |_| Err("denied".into())).unwrap_err(),"denied");
        assert_eq!(add_snapshot_bytes(MAX_SNAPSHOT_BYTES - 1,1).unwrap(),MAX_SNAPSHOT_BYTES);
        assert!(add_snapshot_bytes(MAX_SNAPSHOT_BYTES,1).is_err());
        assert!(add_snapshot_bytes(usize::MAX,1).is_err());
    }

    #[test]
    fn invalid_queries_are_rejected_before_process_resolution() {
        for input in [json!({}), json!({"operation":"rename"}),
            json!({"operation":"hover","line":1}),
            json!({"operation":"references","line":0,"character":1}),
            json!({"operation":"definitions","line":1,"character":4294967296_u64}),
            json!({"operation":"workspace_symbols"}),
            json!({"operation":"workspace_symbols","query":"x".repeat(1025)})] {
            assert!(validate_query_parameters(&input).is_err(), "{input}");
        }
        for input in [json!({"operation":"hover","line":1,"character":2}),
            json!({"operation":"document_symbols"}),
            json!({"operation":"workspace_symbols","query":""})] {
            assert!(validate_query_parameters(&input).is_ok(), "{input}");
        }
    }

    #[test]
    fn discovery_does_not_publish_launch_arguments_or_environment() {
        let root = tempfile::tempdir().unwrap();
        let mut context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Vec::new(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.path().to_path_buf()],
            policy: ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(), browser: BrowserRuntime::new(root.path().join("unused-browser")),
        };
        context.language_services.push(LanguageServiceConfiguration {
            id:"rust".into(),executable:root.path().join("server"),arguments:vec!["private-arg".into()],
            project_root:root.path().to_path_buf(),language_ids:vec!["rust".into()],
            environment: std::collections::BTreeMap::from([("TOKEN".into(),"private-token".into())]),timeout_ms:1000,
        });
        let result = list(&context);
        assert!(!result.error);
        assert!(!result.text.contains("private-arg"));
        assert!(!result.text.contains("private-token"));
        let fingerprint = context.language_services[0].fingerprint().unwrap();
        assert!(configured_service(&context, &json!({"service_id":"rust","configuration_fingerprint":fingerprint})).is_ok());
        assert!(configured_service(&context, &json!({"service_id":"rust","configuration_fingerprint":"stale"})).is_err());
        // No executable or project exists here. A language mismatch must be
        // rejected from configuration before filesystem/process resolution,
        // rather than silently invoking the scanner or another language server.
        if cfg!(unix) {
            let query = json!({"service_id":"rust", "configuration_fingerprint":fingerprint,
                "operation":"document_symbols", "language_id":"cpp", "path":"/workspace/main.cpp"});
            assert_eq!(query_plan(&context, &query).err().as_deref(),
                Some("language is not configured for this service"));
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().canonicalize().unwrap();
            let source = root.join("main.rs");
            let support = root.join("other.rs");
            fs::write(&source, "fn main() {}").unwrap();
            fs::write(&support, "fn helper() {}").unwrap();
            context.language_services[0].project_root = root.clone();
            context.language_services[0].executable = source.clone();
            let mut query = json!({"service_id":"rust", "configuration_fingerprint":context.language_services[0].fingerprint().unwrap(),
                "operation":"document_symbols","language_id":"rust","path":source,
                "supporting_documents":[{"path":support,"language_id":"rust"}]});
            assert_eq!(query_plan(&context,&query).unwrap().supporting_documents.len(),1);
            for invalid in [json!(null),json!({}), json!([{"path":"relative.rs","language_id":"rust"}]),
                json!([{"path":source,"language_id":"rust"}]),json!([{"path":support,"language_id":"cpp"}]),
                json!([{"path":support,"language_id":"rust","extra":true}]),
                json!([{"path":support,"language_id":"rust"},{"path":support,"language_id":"rust"}]),
                json!(vec![json!({"path":support,"language_id":"rust"});17])] {
                query["supporting_documents"] = invalid;
                assert!(query_plan(&context,&query).is_err(),"{query}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn lsp_pool_key_distinguishes_workspace_argv_and_environment() {
        let base_service = LanguageServiceConfiguration {
            id: "rust".into(),
            executable: PathBuf::from("/usr/bin/rust-analyzer"),
            arguments: vec!["--stdio".into()],
            project_root: PathBuf::from("/workspace/a"),
            language_ids: vec!["rust".into()],
            environment: BTreeMap::from([("RUST_LOG".into(), "error".into())]),
            timeout_ms: 5_000,
        };
        let plan_a = QueryPlan {
            service: base_service.clone(),
            executable: base_service.executable.clone(),
            root: base_service.project_root.clone(),
            file: PathBuf::from("/workspace/a/src/lib.rs"),
            language: "rust".into(),
            supporting_documents: Vec::new(),
        };
        let mut same_plan_different_file = QueryPlan {
            service: base_service.clone(),
            executable: base_service.executable.clone(),
            root: base_service.project_root.clone(),
            file: PathBuf::from("/workspace/a/src/main.rs"),
            language: "rust".into(),
            supporting_documents: Vec::new(),
        };
        same_plan_different_file.service.timeout_ms = 10_000;
        assert_eq!(
            LspPoolKey::from_plan(&plan_a),
            LspPoolKey::from_plan(&same_plan_different_file)
        );

        let mut other_root = QueryPlan {
            service: base_service.clone(),
            executable: base_service.executable.clone(),
            root: PathBuf::from("/workspace/b"),
            file: PathBuf::from("/workspace/b/src/lib.rs"),
            language: "rust".into(),
            supporting_documents: Vec::new(),
        };
        assert_ne!(LspPoolKey::from_plan(&plan_a), LspPoolKey::from_plan(&other_root));

        other_root.root = plan_a.root.clone();
        other_root.service.arguments.push("--extra".into());
        assert_ne!(LspPoolKey::from_plan(&plan_a), LspPoolKey::from_plan(&other_root));

        let mut other_env = QueryPlan {
            service: base_service,
            executable: plan_a.executable.clone(),
            root: plan_a.root.clone(),
            file: plan_a.file.clone(),
            language: "rust".into(),
            supporting_documents: Vec::new(),
        };
        other_env.service.environment.insert("RUST_LOG".into(), "debug".into());
        assert_ne!(LspPoolKey::from_plan(&plan_a), LspPoolKey::from_plan(&other_env));
    }

    #[cfg(unix)]
    #[test]
    fn lsp_query_pool_reuses_warm_session_and_respawns_after_process_exit() {
        let Some(python3) = std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("python3"))
                .find(|candidate| candidate.is_file())
        }) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("main.rs");
        fs::write(&source, "fn alpha() {}\n").unwrap();

        let mock_server = root.join("mock_query_lsp.py");
        fs::write(
            &mock_server,
            r#"import json, os, sys

def read_msg():
    cl = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        s = line.decode('utf-8', 'replace').strip()
        if not s:
            break
        if s.lower().startswith('content-length:'):
            cl = int(s.split(':', 1)[1].strip())
    if not cl:
        return None
    body = sys.stdin.buffer.read(cl)
    return json.loads(body.decode('utf-8'))

def send_msg(obj):
    data = json.dumps(obj).encode('utf-8')
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode('ascii') + data)
    sys.stdout.buffer.flush()

query_count = 0
while True:
    msg = read_msg()
    if msg is None:
        break
    method = msg.get("method")
    if method == "initialize":
        send_msg({
            "jsonrpc": "2.0",
            "id": msg["id"],
            "result": {
                "capabilities": {
                    "textDocumentSync": 1,
                    "documentSymbolProvider": True
                }
            }
        })
    elif method == "textDocument/documentSymbol":
        query_count += 1
        send_msg({
            "jsonrpc": "2.0",
            "id": msg["id"],
            "result": [{"name": f"pid:{os.getpid()}:count:{query_count}"}]
        })
    elif method == "shutdown":
        send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif method == "exit":
        break
"#,
        )
        .unwrap();

        let service = LanguageServiceConfiguration {
            id: "mock-rust".into(),
            executable: python3.canonicalize().unwrap(),
            arguments: vec!["-B".into(), mock_server.to_string_lossy().into_owned()],
            project_root: root.clone(),
            language_ids: vec!["rust".into()],
            environment: BTreeMap::new(),
            timeout_ms: 5_000,
        };
        let plan = QueryPlan {
            service: service.clone(),
            executable: service.executable.clone(),
            root: root.clone(),
            file: source.clone(),
            language: "rust".into(),
            supporting_documents: Vec::new(),
        };
        let key = LspPoolKey::from_plan(&plan);
        let cancel = RuntimeExecutionControl::default();
        let read_doc = |p: &Path| fs::read_to_string(p).map_err(|e| e.to_string());

        let first = execute_pooled_query(
            &plan,
            super::language_service_protocol::Query::DocumentSymbols,
            1,
            1,
            None,
            fs::read_to_string(&source).unwrap(),
            &[],
            Instant::now() + Duration::from_secs(5),
            &cancel,
            &read_doc,
        )
        .expect("first pooled query should succeed");
        let label1 = first["result"][0]["name"].as_str().unwrap().to_string();
        assert!(label1.ends_with(":count:1"), "{label1}");
        let pid1: u32 = label1.split(':').nth(1).unwrap().parse().unwrap();

        // Second query with updated content must reuse the SAME warm LSP process.
        fs::write(&source, "fn beta() {}\n").unwrap();
        let second = execute_pooled_query(
            &plan,
            super::language_service_protocol::Query::DocumentSymbols,
            1,
            1,
            None,
            fs::read_to_string(&source).unwrap(),
            &[],
            Instant::now() + Duration::from_secs(5),
            &cancel,
            &read_doc,
        )
        .expect("second pooled query should reuse warm session");
        let label2 = second["result"][0]["name"].as_str().unwrap().to_string();
        assert_eq!(label2, format!("pid:{pid1}:count:2"));

        // Kill the pooled server process and verify automatic respawn on the next query.
        unsafe {
            libc::kill(pid1 as libc::pid_t, libc::SIGKILL);
        }
        std::thread::sleep(Duration::from_millis(50));
        let third = execute_pooled_query(
            &plan,
            super::language_service_protocol::Query::DocumentSymbols,
            1,
            1,
            None,
            fs::read_to_string(&source).unwrap(),
            &[],
            Instant::now() + Duration::from_secs(5),
            &cancel,
            &read_doc,
        )
        .expect("third query should automatically respawn dead pooled server");
        let label3 = third["result"][0]["name"].as_str().unwrap().to_string();
        assert!(label3.ends_with(":count:1"), "{label3}");
        let pid2: u32 = label3.split(':').nth(1).unwrap().parse().unwrap();
        assert_ne!(pid1, pid2, "respawned LSP server must have a new PID");

        // Clean up the pooled test session.
        drop(acquire_pooled_session(&key));
    }
}
