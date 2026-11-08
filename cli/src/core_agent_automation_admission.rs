// SPDX-License-Identifier: Apache-2.0
// Native automation declarations are checked against the actual Host input.
// Artifact source IDs are asserted by the trusted native entry, never inferred
// from prompt text. Host verifies and freezes the exact bytes it actually loads.

fn actual_automation_execution_spec(
    config: &HostInvocationRequest,
) -> Result<Option<nola_domain::automation_execution::AutomationExecutionSpec>, String> {
    use nola_domain::automation_execution::{
        AutomationModelSelection, AutomationPermissionProfile,
    };
    let Some(declared) = &config.session.automation_execution_spec else {
        if config.session.automation_execution_lease.is_some() || !config.session.automation_artifacts.is_empty() {
            return Err("automation authority lacks its execution spec".into());
        }
        return Ok(None);
    };
    declared.validate()?;
    if config.session.automation_execution_lease.is_none() {
        return Err("automation spec has no occurrence authority".into());
    }
    let profile = effective_permission_profile(config);
    if !config.runtime.core_permission_rules.is_empty()
        || !config.runtime.permissions.is_empty()
        || config.tool_settings.auto_approve != profile.auto_approve()
        || config.tool_settings.network_policy != profile.network_policy()
        || config
            .tool_settings.allowed_dirs
            .iter()
            .any(|path| path != &config.tool_settings.project_root)
        || !config.tool_settings.bash_prefixes.is_empty()
    {
        return Err("automation permission profile cannot be overridden by separate grants".into());
    }
    let actual = nola_domain::automation_execution::AutomationExecutionSpec {
        prompt: config.input.message.clone(),
        execution_scope: nola_domain::automation_execution::AutomationExecutionScope::LocalWorkspace { root: config
            .tool_settings.project_root
            .to_str()
            .ok_or("automation workspace is not UTF-8")?
            .to_string() },
        permission_profile: match profile {
            crate::permissions::PermissionProfile::Cautious => {
                AutomationPermissionProfile::Cautious
            }
            crate::permissions::PermissionProfile::AllowAll => {
                AutomationPermissionProfile::AllowAll
            }
        },
        model_selection: AutomationModelSelection::UseCurrentDefaultAtAdmission,
        context_refs: declared.context_refs.clone(),
        skill_refs: declared.skill_refs.clone(),
    };
    if &actual != declared {
        return Err("automation definition differs from actual Host configuration".into());
    }
    validate_automation_artifact_set(config)?;
    Ok(Some(actual))
}

fn automation_artifact_component_id(artifact: &crate::host_invocation_request::AutomationArtifact) -> String {
    format!("automation-artifact:{:?}:{}", artifact.kind, artifact.id)
}

fn verify_loaded_automation_artifacts(config: &HostInvocationRequest) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    if config.session.automation_execution_spec.is_some()
        && config.input.host_context_components.len() != config.session.automation_artifacts.len()
    {
        return Err("automation prompt contains unbound context components".into());
    }
    for artifact in &config.session.automation_artifacts {
        let id = automation_artifact_component_id(artifact);
        let mut matching = config
            .input.host_context_components
            .iter()
            .filter(|component| component.id == id);
        let component = matching
            .next()
            .ok_or("automation artifact was not loaded into the actual prompt context")?;
        let content = component
            .content
            .as_str()
            .ok_or("automation artifact body changed type")?;
        if let Some(expected) = &artifact.source {
            let actual = component
                .source
                .as_ref()
                .ok_or("automation artifact lost its source")?;
            if actual.authorization.as_deref() != Some(expected.authorization.as_str())
                || actual.observed_at.as_deref() != Some(expected.observed_at.as_str())
                || actual.maximum_age_minutes != Some(expected.maximum_age_minutes)
            {
                return Err(
                    "automation artifact source metadata differs from the loaded context".into(),
                );
            }
        }
        if matching.next().is_some()
            || format!("{:x}", Sha256::digest(content.as_bytes())) != artifact.sha256
        {
            return Err("automation loaded context differs from its artifact witness".into());
        }
    }
    Ok(())
}

fn validate_automation_artifact_set(config: &HostInvocationRequest) -> Result<(), String> {
    use crate::host_invocation_request::AutomationArtifactKind;
    use nola_domain::automation_execution::AutomationContextKind;
    let spec = config
        .session.automation_execution_spec
        .as_ref()
        .ok_or("missing automation spec")?;
    let mut expected = std::collections::BTreeSet::new();
    for reference in &spec.context_refs {
        let kind = match reference.kind {
            AutomationContextKind::PersonalData => "personal_data",
            AutomationContextKind::WebSource => "web_source",
        };
        if !expected.insert((kind, reference.id.as_str())) {
            return Err("duplicate automation context reference".into());
        }
    }
    for id in &spec.skill_refs {
        if !expected.insert(("skill", id.as_str())) {
            return Err("duplicate automation Skill reference".into());
        }
    }
    for artifact in &config.session.automation_artifacts {
        let kind = match artifact.kind {
            AutomationArtifactKind::PersonalData => "personal_data",
            AutomationArtifactKind::WebSource => "web_source",
            AutomationArtifactKind::Skill => "skill",
        };
        if !expected.remove(&(kind, artifact.id.as_str()))
            || !artifact.path.is_absolute()
            || artifact.sha256.len() != 64
            || !artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("automation artifact identity/path/digest mismatch".into());
        }
    }
    if !expected.is_empty() {
        return Err("automation reference has no loaded artifact".into());
    }
    Ok(())
}

pub(crate) fn admit_automation_artifacts(config: &mut HostInvocationRequest) -> Result<(), String> {
    use crate::host_invocation_request::AutomationArtifactKind;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    if actual_automation_execution_spec(config)?.is_none() {
        return Ok(());
    }
    if !config.input.context_files.is_empty()
        || config.input.host_turn_context.is_some()
        || !config.input.host_attachments.is_empty()
        || !config.input.host_context_components.is_empty()
    {
        return Err(
            "automation context must use source-bound artifacts, not unbound context channels"
                .into(),
        );
    }
    let catalog = if config
        .session.automation_artifacts
        .iter()
        .any(|artifact| artifact.kind == AutomationArtifactKind::Skill)
    {
        SkillRuntimeContext::from_configured_dirs(config.tool_settings.skill_dirs.clone()).index()?
    } else {
        Vec::new()
    };
    let mut components = Vec::new();
    let mut remaining = 16 * 1024 * 1024usize;
    for artifact in &config.session.automation_artifacts {
        let path = artifact
            .path
            .canonicalize()
            .map_err(|error| format!("automation artifact unavailable: {error}"))?;
        if artifact.kind == AutomationArtifactKind::Skill {
            let item = catalog
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(artifact.id.as_str()))
                .ok_or("automation Skill is not in the actual Host catalog")?;
            let root = item
                .get("base_directory")
                .and_then(Value::as_str)
                .ok_or("automation Skill has no materialized source")?;
            let instruction = std::path::Path::new(root)
                .join("SKILL.md")
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if instruction != path {
                return Err(
                    "automation Skill artifact is not its catalog instruction source".into(),
                );
            }
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|error| error.to_string())?
            .take(remaining.min(4 * 1024 * 1024) as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > remaining.min(4 * 1024 * 1024) {
            return Err("automation artifact exceeds admission budget".into());
        }
        remaining -= bytes.len();
        if format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256 {
            return Err("automation artifact changed before admission".into());
        }
        if artifact.kind != AutomationArtifactKind::Skill {
            let source = artifact
                .source
                .as_ref()
                .ok_or("automation data artifact requires authorization and freshness")?;
            if !matches!(
                source.authorization.as_str(),
                "authorized" | "available" | "not_required"
            ) || source.maximum_age_minutes == 0
                || source.maximum_age_minutes > 10_080
            {
                return Err("automation data artifact authorization/freshness is invalid".into());
            }
            let observed = chrono::DateTime::parse_from_rfc3339(&source.observed_at)
                .map_err(|_| "automation data artifact timestamp is invalid")?;
            let age = chrono::Utc::now()
                .signed_duration_since(observed)
                .num_seconds();
            if age < -300 || age > (source.maximum_age_minutes * 60) as i64 {
                return Err("automation data artifact is stale or from the future".into());
            }
        }
        let text = String::from_utf8(bytes).map_err(|_| "automation artifact is not UTF-8")?;
        components.push(ContextComponent {
            kind: match artifact.kind {
                AutomationArtifactKind::PersonalData => ContextComponentKind::PlatformData,
                AutomationArtifactKind::WebSource => ContextComponentKind::WebRetrieval,
                // Explicitly supplied instruction document, not a fabricated
                // Skill invocation or a permission grant.
                AutomationArtifactKind::Skill => ContextComponentKind::Document,
            },
            id: automation_artifact_component_id(artifact),
            content: Value::String(text),
            source: Some(nola_core::context_engine::ContextComponentSource {
                uri: Some(path.to_string_lossy().into_owned()),
                title: Some(artifact.id.clone()),
                authorization: artifact
                    .source
                    .as_ref()
                    .map(|source| source.authorization.clone()),
                observed_at: artifact
                    .source
                    .as_ref()
                    .map(|source| source.observed_at.clone()),
                maximum_age_minutes: artifact
                    .source
                    .as_ref()
                    .map(|source| source.maximum_age_minutes),
                ..Default::default()
            }),
        });
    }
    config.input.host_context_components.extend(components);
    Ok(())
}

#[cfg(test)]
mod automation_actual_config_tests {
    use super::*;
    use nola_domain::automation_execution::*;

    fn fixture() -> HostInvocationRequest {
        let mut config = HostInvocationRequest::for_test().unwrap();
        config.input.message = "Run the scheduled task".into();
        config.tool_settings.permission_profile = Some(crate::permissions::PermissionProfile::Cautious);
        config.tool_settings.auto_approve = true;
        config.tool_settings.network_policy = "ask".into();
        config.tool_settings.allowed_dirs = vec![config.tool_settings.project_root.clone()];
        config.tool_settings.bash_prefixes.clear();
        config.runtime.permissions.clear();
        config.runtime.core_permission_rules.clear();
        config.session.automation_execution_lease = Some(AutomationExecutionLease {
            occurrence_id: "occurrence".into(),
            definition_revision: 1,
            worker_id: "worker".into(),
            lease_token: "lease".into(),
        });
        config.session.automation_execution_spec = Some(AutomationExecutionSpec {
            prompt: config.input.message.clone(),
            execution_scope: AutomationExecutionScope::LocalWorkspace { root: config.tool_settings.project_root.to_str().unwrap().into() },
            permission_profile: AutomationPermissionProfile::Cautious,
            model_selection: AutomationModelSelection::UseCurrentDefaultAtAdmission,
            context_refs: vec![],
            skill_refs: vec![],
        });
        config
    }

    #[test]
    fn actual_spec_rejects_prompt_workspace_and_permission_drift() {
        let _environment = crate::store::test_env_lock();
        let config = fixture();
        assert_eq!(
            actual_automation_execution_spec(&config).unwrap(),
            config.session.automation_execution_spec
        );
        for mutate in [
            (|value: &mut HostInvocationRequest| value.input.message.push_str(" changed")) as fn(&mut HostInvocationRequest),
            |value| value.tool_settings.project_root.push("other"),
            |value| value.tool_settings.auto_approve = false,
            |value| value.tool_settings.network_policy = "allow".into(),
            |value| value.tool_settings.allowed_dirs.push(std::path::PathBuf::from("/")),
        ] {
            let mut changed = config.clone();
            mutate(&mut changed);
            assert!(actual_automation_execution_spec(&changed).is_err());
        }
    }

    #[test]
    fn selected_skill_is_loaded_from_its_catalog_and_rejects_changed_bytes() {
        use sha2::{Digest, Sha256};
        let _environment = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("review");
        std::fs::create_dir(&root).unwrap();
        let content = "---\nname: review\ndescription: isolated review\n---\nReview carefully.\n";
        let path = root.join("SKILL.md");
        std::fs::write(&path, content).unwrap();
        let runtime = SkillRuntimeContext::from_configured_dirs(Some(vec![root.clone()]));
        let id = runtime.index().unwrap()[0]["id"].as_str().unwrap().to_string();
        let mut config = fixture();
        config.input.host_context_components.clear();
        config.input.context_files.clear();
        config.input.host_attachments.clear();
        config.input.host_turn_context = None;
        config.tool_settings.skill_dirs = Some(vec![root]);
        config.session.automation_execution_spec.as_mut().unwrap().skill_refs = vec![id.clone()];
        config.session.automation_artifacts = vec![crate::host_invocation_request::AutomationArtifact {
            kind: crate::host_invocation_request::AutomationArtifactKind::Skill, id, path: path.clone(),
            sha256: format!("{:x}", Sha256::digest(content.as_bytes())), source: None,
        }];
        let mut admitted = config.clone();
        admit_automation_artifacts(&mut admitted).unwrap();
        assert_eq!(admitted.input.host_context_components.len(), 1);
        assert_eq!(admitted.input.host_context_components[0].content, content);
        verify_loaded_automation_artifacts(&admitted).unwrap();
        std::fs::write(&path, content.replace("carefully", "differently")).unwrap();
        assert!(admit_automation_artifacts(&mut config).unwrap_err().contains("changed before admission"));
        // Already admitted bytes remain frozen, regardless of later file edits.
        verify_loaded_automation_artifacts(&admitted).unwrap();
    }

    #[test]
    fn artifact_bytes_are_loaded_once_and_bound_not_reread_from_mutable_file() {
        use sha2::{Digest, Sha256};
        let _environment = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.txt");
        std::fs::write(&path, "actual web content").unwrap();
        let mut config = fixture();
        config.input.host_context_components.clear();
        config.input.context_files.clear();
        config.input.host_attachments.clear();
        config.input.host_turn_context = None;
        config
            .session.automation_execution_spec
            .as_mut()
            .unwrap()
            .context_refs
            .push(AutomationContextReference {
                kind: AutomationContextKind::WebSource,
                id: "source-one".into(),
            });
        config
            .session.automation_artifacts
            .push(crate::host_invocation_request::AutomationArtifact {
                kind: crate::host_invocation_request::AutomationArtifactKind::WebSource,
                id: "source-one".into(),
                path: path.clone(),
                sha256: format!("{:x}", Sha256::digest(b"actual web content")),
                source: Some(crate::host_invocation_request::AutomationArtifactSource {
                    authorization: "not_required".into(),
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    maximum_age_minutes: 60,
                }),
            });
        admit_automation_artifacts(&mut config).unwrap();
        verify_loaded_automation_artifacts(&config).unwrap();
        std::fs::write(&path, "changed after admission").unwrap();
        verify_loaded_automation_artifacts(&config).unwrap();
        assert_eq!(
            config.input.host_context_components[0].content,
            Value::String("actual web content".into())
        );
        config.input.host_context_components[0].content = Value::String("tampered".into());
        assert!(verify_loaded_automation_artifacts(&config).is_err());
        config.input.host_context_components.clear();
        assert!(admit_automation_artifacts(&mut config)
            .unwrap_err()
            .contains("changed before admission"));
    }

    #[test]
    fn declared_reference_without_exact_artifact_or_with_duplicate_is_rejected() {
        let _environment = crate::store::test_env_lock();
        let mut config = fixture();
        config
            .session.automation_execution_spec
            .as_mut()
            .unwrap()
            .skill_refs = vec!["not-loaded".into()];
        assert!(actual_automation_execution_spec(&config).is_err());
        config
            .session.automation_execution_spec
            .as_mut()
            .unwrap()
            .skill_refs
            .push("not-loaded".into());
        assert!(actual_automation_execution_spec(&config)
            .unwrap_err()
            .contains("duplicate"));
    }
}
