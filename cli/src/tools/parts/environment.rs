// SPDX-License-Identifier: Apache-2.0
// Read-only dependency discovery. No command execution or inferred authority.
fn environment_programs(names: &[String], search_path: &std::ffi::OsStr) -> Value {
    let directories: Vec<_> = std::env::split_paths(search_path)
        .filter(|path| path.is_absolute()).take(128).collect();
    Value::Array(names.iter().map(|name| {
        let path = directories.iter().map(|directory| directory.join(name)).find(|path| {
            let Ok(metadata) = fs::metadata(path) else { return false; };
            if !metadata.is_file() { return false; }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            { true }
        });
        json!({"name":name,"found":path.is_some(),"path":path,
            "functional_status":"not_checked"})
    }).collect())
}

fn environment_probe(args: &Map<String, Value>, _ctx: &ToolContext) -> ToolOutcome {
    let names = match args.get("executables") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= 32 => {
            let mut names = Vec::new();
            for item in items {
                let Some(name) = item.as_str() else { return error("Expected executable names".into()); };
                if name.is_empty() || name.len() > 128
                    || !name.as_bytes()[0].is_ascii_alphanumeric() && !name.starts_with('_')
                    || !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.+-".contains(&c)) {
                    return error("Executable names must be simple names, not paths or commands".into());
                }
                names.push(name.to_string());
            }
            names
        }
        _ => return error("Expected at most 32 executable names".into()),
    };
    let search_path = std::env::var_os("PATH").unwrap_or_default();
    let managers = ["apt-get", "apk", "dnf", "yum", "brew", "python3", "uv", "node", "npm", "cargo"]
        .map(String::from);
    ok_data(json!({
        "schema":"nola.environment_probe.v1",
        "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "executables":environment_programs(&names, &search_path),
        "package_managers_and_runtimes":environment_programs(&managers, &search_path),
        "search_scope":"first 128 absolute PATH directories; relative entries excluded",
        "network":"not_checked", "installation_permission":"not_established",
        "writable_locations":"not_checked",
        "next_action":"Inspect task manifests and concrete permissions. Prefer project-local environments. Use authorized shell to install only required dependencies; verify version/import and a minimal functional operation, then retry the original operation. PATH presence does not establish these checks."
    }), "Environment executable discovery (no programs executed)".into())
}

fn missing_program_dependency(program: &str, capability: &str, packages: Value, retry: Value) -> ToolOutcome {
    error_data(json!({
        "error":"missing_dependency", "capability":capability,
        "dependency":{"kind":"executable","name":program,"package_candidates":packages},
        "operation_completed":false,"retry_after_environment_change":retry,
        "recovery":{
            "discovery_tool":"environment_probe",
            "installation_permission":"not_granted_by_this_result",
            "steps":["Discover available runtimes/package managers and inspect actual installation scope.",
                "Install the required dependency only through an authorized shell workflow, preferring project-local isolation. If unavailable or denied, choose an authorized equivalent implementation.",
                "Check the installed executable/version and perform a minimal functional test; do not infer success from a piped installer exit code.",
                "Retry the original operation after successful preparation; do not repeat it unchanged while the dependency is absent."],
            "alternative":"A project-local renderer may create a PNG through authorized shell; read_file can then supply that PNG. Text extraction does not replace required visual inspection."
        }
    }))
}

#[cfg(test)]
mod environment_probe_tests {
    use super::*;

    #[test]
    fn discovery_does_not_execute_programs_or_claim_permission() {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("fixture-renderer");
        fs::write(&program, "not executable code").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let absent = environment_programs(&["fixture-renderer".into()], root.path().as_os_str());
            assert_eq!(absent[0]["found"], false);
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let result = environment_programs(&["fixture-renderer".into(),"absent".into()], root.path().as_os_str());
        assert_eq!(result[0]["found"], true);
        assert_eq!(result[0]["functional_status"], "not_checked");
        assert_eq!(result[1]["found"], false);
        assert_eq!(environment_programs(&["fixture-renderer".into()], std::ffi::OsStr::new("."))[0]["found"], false);
    }

    #[test]
    fn missing_dependency_preserves_retry_without_authorizing_installation() {
        let result = missing_program_dependency("renderer", "document_page", json!({}),
            json!({"tool":"read_file","arguments":{"path":"/work/a.pdf","pdf_page":2}}));
        assert!(result.error);
        let data = result.data.unwrap();
        assert_eq!(data["error"], "missing_dependency");
        assert_eq!(data["operation_completed"], false);
        assert_eq!(data["recovery"]["installation_permission"], "not_granted_by_this_result");
        assert_eq!(data["retry_after_environment_change"]["arguments"]["pdf_page"], 2);
    }
}
