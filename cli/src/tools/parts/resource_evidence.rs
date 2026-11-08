// SPDX-License-Identifier: Apache-2.0
// Host-observed content versions. No model text supplies these attestations.

fn resource_content_version(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok("absent:v1".into()),
        Err(err) => return Err(err.to_string()),
    };
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|err| err.to_string())?;
        let mut hash = Sha256::new();
        hash.update(target.as_os_str().as_encoded_bytes());
        return Ok(format!("symlink:v1:sha256:{:x}", hash.finalize()));
    }
    if !metadata.is_file() {
        return Err("Version evidence requires a regular file or a missing path".into());
    }
    const LIMIT: u64 = 64 * 1024 * 1024;
    if metadata.len() > LIMIT {
        return Err("Resource exceeds the 64 MiB evidence limit".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(|err| err.to_string())?;
    if !file.metadata().map_err(|err| err.to_string())?.is_file() {
        return Err("Resource changed type during observation".into());
    }
    let mut hasher = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > LIMIT {
            return Err("Resource grew beyond the evidence limit".into());
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn validation_snapshot(
    args: &Map<String, Value>,
    ctx: &ToolContext,
) -> Result<BTreeMap<String, String>, String> {
    let targets = args
        .get("validation_targets")
        .and_then(Value::as_array)
        .filter(|targets| !targets.is_empty())
        .ok_or("Validation requires concrete validation_targets")?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| ctx.allowed_dirs.first().cloned())
        .ok_or("Missing command working directory")?;
    let mut snapshot = BTreeMap::new();
    let mut remaining = 4096usize;
    let mut bytes_remaining = 256 * 1024 * 1024u64;
    fn visit(
        path: &Path,
        ctx: &ToolContext,
        remaining: &mut usize,
        bytes_remaining: &mut u64,
        snapshot: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        if *remaining == 0 {
            return Err(
                "Validation scope exceeds 4096 entries; specify the changed files directly".into(),
            );
        }
        *remaining -= 1;
        // Observe links as entries; never traverse their destination.
        let resolved = resolve_allowed_entry(&path.to_string_lossy(), &ctx.allowed_dirs)
            .map_err(|outcome| outcome.text)?;
        let metadata = fs::symlink_metadata(&resolved).ok();
        if metadata.as_ref().is_some_and(|value| value.is_dir()) {
            let mut children = fs::read_dir(&resolved)
                .map_err(|err| err.to_string())?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| err.to_string())?;
            children.sort();
            for child in children {
                visit(&child, ctx, remaining, bytes_remaining, snapshot)?;
            }
        } else {
            let size = metadata
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            *bytes_remaining = bytes_remaining
                .checked_sub(size)
                .ok_or("Validation scope exceeds the 256 MiB evidence budget")?;
            let uri = url::Url::from_file_path(&resolved)
                .map_err(|_| "Non-absolute validation path")?
                .to_string();
            snapshot.insert(uri, resource_content_version(&resolved)?);
        }
        Ok(())
    }
    for target in targets {
        let target = target
            .as_str()
            .filter(|target| !target.trim().is_empty() && *target != "*")
            .ok_or("Invalid validation target")?;
        let path = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            cwd.join(target)
        };
        visit(
            &path,
            ctx,
            &mut remaining,
            &mut bytes_remaining,
            &mut snapshot,
        )?;
    }
    Ok(snapshot)
}

fn validation_evidence_unavailable(mut outcome: ToolOutcome, code: &str, message: String) -> ToolOutcome {
    outcome.error = true;
    outcome.text.push_str(&format!("\n{message}"));
    // Typed data is the model's canonical output. Updating presentation alone
    // leaves it seeing exit=0 with no explanation for the failed verification.
    let mut data = outcome.data.take().unwrap_or_else(|| json!({"text":outcome.text}));
    if !data.is_object() { data = json!({"output":data}); }
    data["verification_status"] = json!("unavailable");
    data["verification_error"] = json!(code);
    data["verification_message"] = json!(message);
    outcome.data = Some(data);
    if let Some(receipt) = outcome.execution.as_mut() {
        receipt.checks.clear();
    }
    outcome
}

fn attach_validation_evidence(
    mut outcome: ToolOutcome,
    before: BTreeMap<String, String>,
    after: Result<BTreeMap<String, String>, String>,
) -> ToolOutcome {
    use nola_host_protocol::tool_result::*;
    if outcome.error
        || outcome
            .execution
            .as_ref()
            .is_some_and(|receipt| receipt.state == ExecutionState::Deferred)
    {
        return outcome;
    }
    let after = match after {
        Ok(after) => after,
        Err(message) => {
            return validation_evidence_unavailable(outcome, "snapshot_unavailable",
                format!("Verification evidence unavailable: {message}. Execution may already have taken effect; inspect current state before retrying."));
        }
    };
    if before != after {
        return validation_evidence_unavailable(outcome, "scope_changed_during_validation",
            "Validation scope changed during the command. No stable-version verification receipt was issued. Inspect the resulting state, then validate without modifying the target; do not repeat the mutation merely to obtain evidence.".into());
    }
    outcome.execution = Some(ExecutionReceipt {
        local_stop: outcome.execution.as_ref().and_then(|receipt| receipt.local_stop.clone()),
        state: ExecutionState::Completed,
        effects: Vec::new(),
        checks: after
            .into_iter()
            .map(|(resource_uri, version)| PostconditionCheck {
                kind: nola_host_protocol::tool_result::ExecutionCheckKind::ProcessExit,
                resource_uri,
                version,
                predicate: "process_exit_zero_with_stable_resource".into(),
                passed: true,
            })
            .collect(),
    });
    outcome
}

#[cfg(test)]
mod resource_evidence_tests {
    use super::*;

    #[test]
    fn content_versions_and_tombstones_are_host_observations() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sample");
        assert_eq!(resource_content_version(&path).unwrap(), "absent:v1");
        fs::write(&path, "abc").unwrap();
        assert_eq!(
            resource_content_version(&path).unwrap(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let outcome = ok("written".into(), "written".into()).with_file_artifact(&path, "write");
        let effect = &outcome.execution.unwrap().effects[0];
        assert_eq!(
            effect.after_version.as_deref(),
            Some(resource_content_version(&path).unwrap().as_str())
        );
        assert!(resource_content_version(root.path()).is_err());
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(resource_content_version(&link).unwrap().starts_with("symlink:v1:sha256:"));
        }
    }

    #[test]
    fn verification_failure_survives_typed_host_projection_even_with_zero_exit() {
        let before: BTreeMap<String, String> = [("file:///tmp/a".into(), "v1".into())].into();
        for (after, code) in [
            (Ok([("file:///tmp/a".into(), "v2".into())].into()), "scope_changed_during_validation"),
            (Err("fixture snapshot failure".into()), "snapshot_unavailable"),
        ] {
            let outcome = ok_data(json!({"type":"shell_execution","state":"completed","exit_code":0,
                "command":"large command".repeat(2000), "stdout":"executed", "stderr":""}), "exit=0".into());
            let outcome = attach_validation_evidence(outcome, before.clone(), after);
            assert!(outcome.error);
            assert!(outcome.execution.as_ref().is_none_or(|receipt| receipt.checks.is_empty()));
            let projected = clamp_outcome_with_writer(outcome, 2000, |_| Ok(PathBuf::from("/fixture/result.json")));
            let data = projected.data.as_ref().unwrap();
            assert_eq!(data["exit_code"], 0, "do not falsify process execution");
            assert_eq!(data["verification_status"], "unavailable");
            assert_eq!(data["verification_error"], code);
            assert!(!data["verification_message"].as_str().unwrap().is_empty());
            assert!(projected.text.chars().count() <= 2000);
            let wire = nola_host_protocol::tool_result::ToolResult {
                contract:nola_host_protocol::tool_result::ToolResultContract::V1,
                presentation:nola_host_protocol::tool_result::ToolPresentation {text:projected.text,summary:projected.display},
                data:projected.data,execution:projected.execution,
                full_output_ref:projected.full_output_ref,memory_usage:None,
            }.into_core_output();
            assert_eq!(wire["verification_error"], code);
            assert_eq!(wire["stdout"], "executed");
        }
    }

    #[test]
    fn shell_diagnostics_never_upgrade_process_exit_to_business_acceptance() {
        use nola_host_protocol::tool_result::ExecutionCheckKind;
        let versions: BTreeMap<String, String> = [("file:///tmp/a".into(), "v1".into())].into();
        for stderr in ["", "Traceback: KeyError", "warning: optional dependency unavailable", "测试失败"] {
            let outcome = ok_data(json!({"type":"shell_execution", "state":"completed",
                "exit_code":0, "stdout":"ALL_OK", "stderr":stderr}), "exit=0".into());
            let result = attach_validation_evidence(outcome, versions.clone(), Ok(versions.clone()));
            assert!(!result.error, "diagnostic text is not an exit status");
            assert_eq!(result.data.as_ref().unwrap()["stderr"], stderr);
            let checks = &result.execution.as_ref().unwrap().checks;
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].kind, ExecutionCheckKind::ProcessExit);
            assert_eq!(checks[0].version, "v1");
        }
    }

    #[test]
    fn only_successful_stable_foreground_results_receive_checks() {
        use nola_host_protocol::tool_result::*;
        let before: BTreeMap<String, String> = [("file:///tmp/a".into(), "v1".into())].into();
        let stable = attach_validation_evidence(
            ok("ok".into(), "ok".into()),
            before.clone(),
            Ok(before.clone()),
        );
        assert!(!stable.error);
        let check = &stable.execution.unwrap().checks[0];
        assert_eq!(check.version, "v1");
        assert_eq!(check.kind, ExecutionCheckKind::ProcessExit);
        let changed = attach_validation_evidence(
            ok("ok".into(), "ok".into()),
            before.clone(),
            Ok([("file:///tmp/a".into(), "v2".into())].into()),
        );
        assert!(changed.error);
        assert!(changed.execution.is_none());
        let failed =
            attach_validation_evidence(error("failed".into()), before.clone(), Ok(before.clone()));
        assert!(failed.execution.is_none());
        let mut deferred = ok("running".into(), "running".into());
        deferred.execution = Some(ExecutionReceipt {
            state: ExecutionState::Deferred,
            ..Default::default()
        });
        assert!(
            attach_validation_evidence(deferred, before.clone(), Ok(before))
                .execution
                .unwrap()
                .checks
                .is_empty()
        );
    }

    #[test]
    fn stable_validation_keeps_the_independent_current_local_stop_identity() {
        use nola_host_protocol::tool_result::*;
        let stop = LocalExecutionStop {
            identity: LocalExecutionIdentity {
                execution_id: "private-invocation".into(), host_id: "host".into(),
                session_id: "session".into(), turn_id: "turn".into(), effect_id: "effect".into(),
                request_id: "request".into(), call_id: "call".into(), delivery_attempt_id: None,
            },
            scope: LocalExecutionStopScope::OwnedProcessGroup { leader_pid: 42 },
        };
        let outcome = ok("actual process returned".into(), "returned".into()).with_execution(ExecutionReceipt {
            state: ExecutionState::Unknown, local_stop: Some(stop.clone()), ..Default::default()
        });
        let versions: BTreeMap<String, String> = [("file:///tmp/a".into(), "v1".into())].into();
        let result = attach_validation_evidence(outcome, versions.clone(), Ok(versions));
        let receipt = result.execution.unwrap();
        assert_eq!(receipt.local_stop, Some(stop));
        assert_eq!(receipt.state, ExecutionState::Completed);
        assert_eq!(receipt.checks.len(), 1);
        assert_eq!(receipt.checks[0].kind, ExecutionCheckKind::ProcessExit);
    }

    #[test]
    fn shell_effects_and_separate_checks_observe_the_same_actual_version() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("sample"), "payload").unwrap();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![root.path().to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.path().join("browser")),
        };
        let mut args = json!({"command":"cp sample copied", "cwd":root.path(), "operation_kind":"mutate_and_validate",
            "output_artifacts":[{"path":"copied","operation":"create"}], "validation_targets":["copied"]});
        let mutation = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
        assert!(!mutation.error, "{}", mutation.text);
        let mutation = mutation.execution.unwrap();
        assert!(
            mutation.checks.is_empty(),
            "a combined label cannot attest verification"
        );
        args["operation_kind"] = json!("validate");
        args["command"] = json!("test -f copied");
        args.as_object_mut().unwrap().remove("output_artifacts");
        let validation = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
        assert!(!validation.error, "{}", validation.text);
        let check = &validation.execution.unwrap().checks[0];
        assert_eq!(
            Some(&check.version),
            mutation.effects[0].after_version.as_ref()
        );
        assert_eq!(check.resource_uri, mutation.effects[0].resource_uri);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join("sample"), root.path().join("alias"))
                .unwrap();
            args["validation_targets"] = json!(["alias"]);
            let snapshot = validation_snapshot(args.as_object().unwrap(), &ctx).unwrap();
            assert_eq!(snapshot.len(), 1);
            assert!(snapshot.values().next().unwrap().starts_with("symlink:v1:sha256:"));
            args = json!({"command":"mkdir organized && mv sample alias organized/", "cwd":root.path(),
                "operation_kind":"mutate", "output_artifacts":[
                    {"path":"sample","operation":"delete"}, {"path":"alias","operation":"delete"},
                    {"path":"organized/sample","operation":"create"}, {"path":"organized/alias","operation":"create"}
                ]});
            let moved = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
            assert!(!moved.error, "{}", moved.text);
            let effects = moved.execution.unwrap().effects;
            assert_eq!(effects.len(), 4);
            assert!(effects.iter().all(|effect| effect.after_version.is_some()));
            let link_effect = effects.iter().find(|effect| effect.resource_uri.ends_with("/organized/alias")).unwrap();
            assert!(link_effect.after_version.as_ref().unwrap().starts_with("symlink:v1:sha256:"));
            args["validation_targets"] = json!(["organized"]);
            let snapshot = validation_snapshot(args.as_object().unwrap(), &ctx).unwrap();
            assert_eq!(snapshot.len(), 2);
            assert_eq!(snapshot.get(&link_effect.resource_uri), link_effect.after_version.as_ref());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_entries_keep_identity_without_reading_external_targets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("private");
        fs::write(&target, "first").unwrap();
        let link = root.path().join("shortcut");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let version = resource_content_version(&link).unwrap();
        fs::write(&target, "changed externally").unwrap();
        assert_eq!(resource_content_version(&link).unwrap(), version);
        fs::remove_file(&target).unwrap();
        assert_eq!(resource_content_version(&link).unwrap(), version);
        let resolved = resolve_allowed_entry(link.to_str().unwrap(), &[root.path().into()]).unwrap();
        assert_eq!(resolved, root.path().canonicalize().unwrap().join("shortcut"));
        let outcome = ok("moved".into(), "moved".into()).with_file_artifact(&link, "create");
        let receipt = outcome.execution.unwrap();
        assert_eq!(receipt.effects[0].after_version.as_ref(), Some(&version));
        assert!(receipt.effects[0].resource_uri.ends_with("/shortcut"));
        let escape = root.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &escape).unwrap();
        assert!(resolve_allowed_entry(escape.join("private").to_str().unwrap(), &[root.path().into()]).is_ok());
        let args = json!({"output_artifacts":[
            {"path":"report.txt","operation":"delete"},
            {"path":"shortcut","operation":"delete"},
            {"path":"organized/report.txt","operation":"create"},
            {"path":"organized/shortcut","operation":"create"}
        ]});
        let outputs = declared_bash_outputs(args.as_object().unwrap(), root.path(), &[root.path().into()]).unwrap();
        assert_eq!(outputs.len(), 4);
        assert_eq!(outputs[1].path, resolved);
    }
}
