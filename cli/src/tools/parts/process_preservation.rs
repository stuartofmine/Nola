// SPDX-License-Identifier: Apache-2.0
// Host-observed preservation resources, compiled into process restrictions.
// No command classification or user-language routing participates here.

#[derive(Default)]
struct ProcessPreservation {
    read_only: std::collections::BTreeSet<PathBuf>,
    recovery_capabilities: std::collections::BTreeSet<String>,
    guidance: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl ProcessPreservation {
    fn active(&self) -> bool {
        !self.read_only.is_empty()
    }

    fn command(&self) -> Result<Command, String> {
        if !self.active() {
            return Ok(Command::new("/bin/bash"));
        }
        #[cfg(target_os = "macos")]
        {
            if !Path::new("/usr/bin/sandbox-exec").is_file() {
                return Err(
                    "Process preservation backend is unavailable; execution was not started".into(),
                );
            }
            // Literal strings are encoded, not interpolated as SBPL source.
            let mut profile = String::from("(version 1)(allow default)");
            let mut ancestors = std::collections::BTreeSet::new();
            for path in &self.read_only {
                let path_text = path.to_str().ok_or("Preservation path is not UTF-8")?;
                profile.push_str(&format!(
                    "(deny file-write* (subpath {}))",
                    serde_json::to_string(path_text).unwrap()
                ));
                profile.push_str(&format!(
                    "(deny file-link (subpath {}))",
                    serde_json::to_string(path_text).unwrap()
                ));
                ancestors.extend(path.ancestors().map(Path::to_path_buf));
            }
            // Prevent moving a containing directory to escape a path rule.
            for path in ancestors {
                profile.push_str(&format!(
                    "(deny file-write-unlink (literal {}))",
                    serde_json::to_string(
                        path.to_str().ok_or("Preservation ancestor is not UTF-8")?
                    )
                    .unwrap()
                ));
            }
            let mut command = Command::new("/usr/bin/sandbox-exec");
            command.args(["-p", &profile, "/bin/bash"]);
            command.envs(&self.environment);
            Ok(command)
        }
        #[cfg(not(target_os = "macos"))]
        Err("This platform has no installed process preservation backend; execution was not started".into())
    }

    fn annotate(&self, outcome: &mut ToolOutcome) {
        if !self.active() {
            return;
        }
        outcome.text.push_str("\n\nHost preservation: observed resources are read-only to this process and its descendants. Permission approval and operation_kind do not lift this restriction. Command failure can still have other effects; inspect before retrying.\n");
        outcome.text.push_str(&self.guidance.join("\n"));
        // Structured tool results must carry the same restriction as the text
        // channel. Otherwise a model consuming shell_execution data sees only
        // EPERM and repeatedly asks for permission that cannot lift this guard.
        if let Some(Value::Object(data)) = outcome.data.as_mut() {
            data.insert("host_preservation".into(), json!({
                "active": true,
                "read_only_resources": self.read_only,
                "permission_approval_lifts_restriction": false,
                "recovery_capabilities": self.recovery_capabilities,
                "guidance": self.guidance,
                "failure_cause_confirmed": false,
            }));
        }
        outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
            source: "host_resource_preservation".into(),
            capability_ids: self.recovery_capabilities.iter().cloned().collect(),
            capability_descriptors: Vec::new(),
        });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod process_preservation_tests {
    use super::*;

    #[test]
    fn structured_result_explains_preservation_without_claiming_failure_cause() {
        let guard = ProcessPreservation {
            read_only: [PathBuf::from("/fixture/.git")].into(),
            recovery_capabilities: ["git_commit".into()].into(),
            guidance: vec!["Select concrete files through git_commit".into()],
            ..Default::default()
        };
        let mut outcome = error_data(json!({"type":"shell_execution", "exit_code":1}));
        guard.annotate(&mut outcome);
        let data = outcome.data.unwrap();
        assert_eq!(data["exit_code"], 1);
        assert_eq!(data["host_preservation"]["permission_approval_lifts_restriction"], false);
        assert_eq!(data["host_preservation"]["failure_cause_confirmed"], false);
        assert_eq!(data["host_preservation"]["recovery_capabilities"], json!(["git_commit"]));
        assert!(outcome.text.contains("Permission approval"));
    }

    #[test]
    fn a_preserved_pointer_file_cannot_be_overwritten_or_unlinked() {
        let root = tempfile::tempdir().unwrap();
        let pointer = root.path().join("pointer with \"quotes\"");
        fs::write(&pointer, "original target").unwrap();
        let guard = ProcessPreservation {
            read_only: [pointer.canonicalize().unwrap()].into(),
            ..Default::default()
        };
        for script in ["printf changed > \"$1\"", "rm -- \"$1\""] {
            let output = guard
                .command()
                .unwrap()
                .args(["-c", script, "fixture"])
                .arg(&pointer)
                .current_dir(root.path())
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert_eq!(fs::read_to_string(&pointer).unwrap(), "original target");
        }
    }

    #[test]
    fn opaque_descendants_cannot_write_or_rename_preserved_resources() {
        let root = tempfile::tempdir().unwrap();
        let protected = root.path().join("state with spaces");
        fs::create_dir(&protected).unwrap();
        fs::write(protected.join("value"), "before").unwrap();
        let guard = ProcessPreservation {
            read_only: [protected.canonicalize().unwrap()].into(),
            ..Default::default()
        };
        for script in [
            "printf changed > 'state with spaces/value'",
            "bash -c \"printf changed > 'state with spaces/value'\"",
            "mv 'state with spaces' moved",
            "ln 'state with spaces/value' alias && printf changed > alias",
        ] {
            let output = guard
                .command()
                .unwrap()
                .args(["-c", script])
                .current_dir(root.path())
                .output()
                .unwrap();
            assert!(!output.status.success(), "{script}");
            assert_eq!(
                fs::read_to_string(protected.join("value")).unwrap(),
                "before"
            );
        }
        assert!(guard
            .command()
            .unwrap()
            .args([
                "-c",
                "cat 'state with spaces/value'; printf allowed > other"
            ])
            .current_dir(root.path())
            .output()
            .unwrap()
            .status
            .success());
    }
}
