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
fn execute_query(args: &Map<String, Value>, context: &ToolContext) -> Result<Value, String> {
    use super::language_service_protocol::Query;
    use super::language_service_session::Session;
    use std::io::Read;
    use std::process::{Command, Stdio};
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(plan.service.timeout_ms);
    let mut command = Command::new(&plan.executable);
    command.args(&plan.service.arguments).current_dir(&plan.root)
        .env_clear().envs(&plan.service.environment)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let child = crate::task_runtime::OwnedShellChild::spawn_owned(&mut command).map_err(|error|format!("Start language service: {error}"))?;
    let cancellation = context.cancellation.clone();
    let mut session = Session::connect(child, &plan.root, &plan.file, &plan.language, text, deadline,
        std::sync::Arc::new(move || cancellation.is_requested()))?;
    let result = (|| {
        for (path, language, text) in &supporting {
            session.open_supporting(path, language, text.clone(), deadline)?;
        }
        let result = session.query(operation, line, character, symbol_query, deadline, || read_document(&plan.file))?;
        verify_supporting_snapshots(&supporting, read_document)?;
        Ok(result)
    })();
    let closed = session.close(deadline);
    match (result, closed) {
        (Ok(mut result), Ok(())) => {
            result["service_id"] = json!(plan.service.id);
            result["configuration_fingerprint"] = json!(plan.service.fingerprint()?);
            Ok(result)
        }
        (Err(error), Ok(())) => Err(error),
        (result, Err(error)) => Err(format!("language service lifecycle failed: {error}; query_succeeded={}", result.is_ok())),
    }
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
        let mut context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Vec::new(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![PathBuf::from("/workspace")],
            policy: ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(), browser: BrowserRuntime::new(PathBuf::from("/unused-browser")),
        };
        context.language_services.push(LanguageServiceConfiguration {
            id:"rust".into(),executable:PathBuf::from("/tools/server"),arguments:vec!["private-arg".into()],
            project_root:PathBuf::from("/workspace"),language_ids:vec!["rust".into()],
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
}
