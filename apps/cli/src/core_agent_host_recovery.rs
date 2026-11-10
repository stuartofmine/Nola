// SPDX-License-Identifier: Apache-2.0
//! Host-private immutable reconstruction, bound by the canonical Run digest.
use crate::{host_invocation_request::HostInvocationRequest, store::NolaStore};
use nola_host_protocol::HostRunRecoveryReference;
use nola_host_protocol::{HostInvocationBinding, HostModelRouteBinding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ADAPTER: &str = "nola.cli.host.v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    request: HostInvocationRequest,
    binding: HostInvocationBinding,
    credentials: Vec<Option<String>>,
}

fn sha(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(super) fn capture(
    request: &HostInvocationRequest,
    conversation_id: &str,
    binding: &HostInvocationBinding,
) -> Result<Option<HostRunRecoveryReference>, String> {
    if !request.session.save_session {
        return Ok(None);
    }
    let store = NolaStore::open_default()?;
    capture_at(request, conversation_id, binding, &store)
}

fn capture_at(
    request: &HostInvocationRequest,
    conversation_id: &str,
    binding: &HostInvocationBinding,
    store: &NolaStore,
) -> Result<Option<HostRunRecoveryReference>, String> {
    let profiles = store.model_profiles()?;
    let mut vault = crate::host_credentials::HostCredentialVault::open(store.path())?;
    let mut frozen = request.clone();
    frozen.session.session_id = Some(conversation_id.into());
    frozen.session.resume_snapshot = None;
    frozen.session.edited_resend_operation_id = None;
    frozen.session.desktop_resume = None;
    frozen.session.desktop_cancel = None;
    frozen.session.host_task_execution = None;
    frozen.session.automation_execution_lease = None;
    frozen.session.automation_execution_spec = None;
    frozen.session.automation_artifacts.clear();
    // Committed input is restored from the Run; stale transport input must not
    // become a second submission on reconstruction.
    frozen.session.host_submission_id = None;
    frozen.session.host_user_message_id = None;
    frozen.session.host_assistant_message_id = None;
    frozen.session.expected_configuration_revision = None;
    frozen.session.expected_parent_message_id = None;
    let mut credentials = Vec::new();
    for (index, route) in std::iter::once(&binding.primary)
        .chain(&binding.fallbacks)
        .enumerate()
    {
        let HostModelRouteBinding::Http(route) = route else {
            credentials.push(None);
            continue;
        };
        // Query secrets require their own managed resolver. Never persist a
        // query in the descriptor or silently replace it with today's URL.
        if route.endpoint_query_sha256.is_some() {
            return Ok(None);
        }
        let secret = if index == 0 {
            request.model_settings.api_key.clone()
        } else {
            request.runtime.llm_fallbacks[index - 1]
                .api_key
                .clone()
                .unwrap_or_else(|| request.model_settings.api_key.clone())
        };
        if secret.is_empty() {
            credentials.push(None);
            continue;
        }
        let identity = crate::host_credentials::identity(&route.endpoint, route.protocol.as_str())?;
        let mut managed = None;
        for profile in &profiles {
            let source: serde_json::Value =
                serde_json::from_str(&profile.source_json).map_err(|_| "Invalid model source")?;
            let Some(endpoint_id) = source["endpointID"].as_str() else {
                continue;
            };
            if let Some(record) = vault.record(endpoint_id)? {
                if record.identity == identity && record.secret.as_deref() == Some(secret.as_str())
                {
                    managed = Some(endpoint_id.to_owned());
                    break;
                }
            }
        }
        let handle =
            managed.unwrap_or_else(|| format!("runtime-route-{}", sha(identity.as_bytes())));
        match vault.record(&handle)? {
            Some(record)
                if record.identity == identity
                    && record.secret.as_deref() == Some(secret.as_str()) => {}
            Some(_) => return Err("Frozen route credential was changed or revoked".into()),
            None => {
                vault.write(&handle, &identity, 0, Some(&secret), || Ok(()))?;
            }
        }
        credentials.push(Some(handle));
    }
    frozen.model_settings.api_key.clear();
    for route in &mut frozen.runtime.llm_fallbacks {
        route.api_key = None;
    }
    let descriptor = serde_json::to_string(&Descriptor {
        request: frozen,
        binding: binding.clone(),
        credentials,
    })
    .map_err(|_| "Cannot encode frozen Host descriptor")?;
    let handle = uuid::Uuid::new_v4().simple().to_string();
    vault.save_runtime_descriptor(&handle, &descriptor)?;
    Ok(Some(HostRunRecoveryReference {
        adapter: ADAPTER.into(),
        handle,
        sha256: sha(descriptor.as_bytes()),
    }))
}

pub(super) fn restore(
    reference: &HostRunRecoveryReference,
    conversation_id: &str,
    binding: &HostInvocationBinding,
) -> Result<HostInvocationRequest, String> {
    if reference.adapter != ADAPTER {
        return Err("Frozen Host adapter is unavailable".into());
    }
    let store = NolaStore::open_default()?;
    restore_at(reference, conversation_id, binding, &store)
}

fn restore_at(
    reference: &HostRunRecoveryReference,
    conversation_id: &str,
    binding: &HostInvocationBinding,
    store: &NolaStore,
) -> Result<HostInvocationRequest, String> {
    if reference.adapter != ADAPTER {
        return Err("Frozen Host adapter is unavailable".into());
    }
    let vault = crate::host_credentials::HostCredentialVault::open(store.path())?;
    let encoded = vault
        .runtime_descriptor(&reference.handle)?
        .ok_or("Frozen Host descriptor is unavailable")?;
    if sha(encoded.as_bytes()) != reference.sha256 {
        return Err("Frozen Host descriptor integrity failure".into());
    }
    let mut descriptor: Descriptor =
        serde_json::from_str(&encoded).map_err(|_| "Invalid frozen Host descriptor")?;
    binding
        .require_exact(&descriptor.binding)
        .map_err(str::to_owned)?;
    if descriptor.request.session.session_id.as_deref() != Some(conversation_id)
        || !descriptor.request.session.save_session
        || descriptor.credentials.len() != binding.fallbacks.len() + 1
    {
        return Err("Frozen Host descriptor ownership mismatch".into());
    }
    for (index, route) in std::iter::once(&binding.primary)
        .chain(&binding.fallbacks)
        .enumerate()
    {
        let HostModelRouteBinding::Http(route) = route else {
            continue;
        };
        let secret = match &descriptor.credentials[index] {
            None if route.credential_sha256 == nola_host_protocol::EMPTY_CREDENTIAL_SHA256 => {
                String::new()
            }
            None => return Err("Frozen credential reference is unavailable".into()),
            Some(handle) => {
                let record = vault
                    .record(handle)?
                    .ok_or("Frozen credential was removed")?;
                if record.identity
                    != crate::host_credentials::identity(&route.endpoint, route.protocol.as_str())?
                {
                    return Err("Frozen credential endpoint identity changed".into());
                }
                record.secret.ok_or("Frozen credential was revoked")?
            }
        };
        if sha(secret.as_bytes()) != route.credential_sha256 {
            return Err("Frozen credential identity changed".into());
        }
        if index == 0 {
            descriptor.request.model_settings.api_key = secret;
        } else {
            descriptor.request.runtime.llm_fallbacks[index - 1].api_key = Some(secret);
        }
    }
    descriptor.request.runtime.native_model_route = match &binding.primary {
        HostModelRouteBinding::Native(route) => Some(route.clone()),
        _ => None,
    };
    descriptor.request.runtime.siwc = match &binding.primary {
        HostModelRouteBinding::Http(route) => if let Some(account) = &route.siwc_account {
            let current = crate::model_siwc::SiwcHost::open(store.path())?.execution_binding(
                &account.account_id, account.account_revision, &account.credential_ref)?;
            if &current != account { return Err("Frozen SIWC account identity changed".into()); }
            Some(crate::model_siwc::SiwcExecutionContext {database:store.path().to_path_buf(),account:account.clone()})
        } else {None},
        _ => None,
    };
    descriptor.request.runtime.mac_launch = binding.mac_launch.clone();
    descriptor.request.runtime.output_admission = Some(binding.output_admission.clone());
    descriptor.request.runtime.admitted_max_retries = Some(binding.generation.max_retries);
    Ok(descriptor.request)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn descriptor_restores_exact_policy_and_rejects_wrong_owner_digest_and_revocation() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let mut request = HostInvocationRequest::for_test().unwrap();
        request.session.save_session = true;
        request.session.session_id = Some("original-owner".into());
        request.model_settings.endpoint = "https://fixture.invalid/v1".into();
        request.model_settings.api_key = "fixture-private-credential".into();
        request.tool_settings.project_root = root.path().into();
        request.runtime.output_admission = Some(
            crate::host_output_admission::admit(
                &request.model_invocation_settings().unwrap(),
                true,
            )
            .unwrap(),
        );
        let binding =
            crate::core_agent_runtime::fixture_host_invocation_binding(&mut request).unwrap();
        let reference = capture_at(&request, "original-owner", &binding, &store)
            .unwrap()
            .unwrap();
        let vault = crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        let encoded = vault
            .runtime_descriptor(&reference.handle)
            .unwrap()
            .unwrap();
        assert!(!encoded.contains("fixture-private-credential"));
        let restored = restore_at(&reference, "original-owner", &binding, &store).unwrap();
        assert_eq!(
            restored.tool_settings.project_root,
            request.tool_settings.project_root
        );
        assert_eq!(
            restored.model_settings.api_key,
            "fixture-private-credential"
        );
        assert!(restore_at(&reference, "foreign-owner", &binding, &store).is_err());
        let mut damaged = reference.clone();
        damaged.sha256 = "0".repeat(64);
        assert!(restore_at(&damaged, "original-owner", &binding, &store).is_err());
        let descriptor: Descriptor = serde_json::from_str(&encoded).unwrap();
        let endpoint_id = descriptor.credentials[0].as_ref().unwrap();
        let mut vault = crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        let record = vault.record(endpoint_id).unwrap().unwrap();
        vault
            .write(endpoint_id, &record.identity, record.revision, None, || {
                Ok(())
            })
            .unwrap();
        assert!(restore_at(&reference, "original-owner", &binding, &store).is_err());
    }

    #[test]
    fn missing_descriptor_and_foreign_adapter_never_fall_back_to_current_config() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        store.model_profiles().unwrap();
        let mut request = HostInvocationRequest::for_test().unwrap();
        let binding =
            crate::core_agent_runtime::fixture_host_invocation_binding(&mut request).unwrap();
        let mut reference = HostRunRecoveryReference {
            adapter: ADAPTER.into(),
            handle: "0".repeat(32),
            sha256: "0".repeat(64),
        };
        assert!(restore_at(&reference, "owner", &binding, &store).is_err());
        reference.adapter = "foreign.host".into();
        assert!(restore_at(&reference, "owner", &binding, &store).is_err());
    }
}

/// Model-only, Host-private material. Intentionally neither Debug nor Serialize.
/// No old request, tool settings, scope, workspace or permission is returned.
pub(super) struct EditedResendModelMaterial {
    pub prepared:nola_domain::edited_resend::PreparedEditedResend,
    pub primary:HostModelRouteBinding,
    pub fallbacks:Vec<HostModelRouteBinding>,
    pub generation:nola_host_protocol::HostGenerationBinding,
    pub output:nola_host_protocol::HostOutputAdmission,
    pub credentials:Vec<String>,
    pub child_instructions:Option<Vec<nola_domain::expert_message::ExpertPromptDefinition>>,
    pub prompt_cache_key:Option<String>,
    pub prompt_cache_key_prefix:Option<String>,
}

/// The sole entry accepts only the durable target. UI-supplied reference/handle
/// or a serialized old Host request can never select the source credential.
pub(super) fn edited_resend_model_material(target:&str, authority:Option<&nola_application::expert_template_capture::HostExpertTemplateAuthority>) -> Result<EditedResendModelMaterial,String> {
    let mut application=crate::application_port::CliApplicationPort::persistent_default()?;
    let material=application.edited_resend_host_material(target)?.ok_or("edited_resend_preparation_not_found")?;
    let child_instructions=if material.expert_child {
        let authority=authority.cloned().ok_or("edited_expert_resend_current_host_identity_required")?;
        application.execute_expert_host_operation(&uuid::Uuid::new_v4().to_string(),
            nola_application::host_expert_template_operation::HostExpertTemplateOperation::LoadChildInstructions {
                conversation_id:target.into(), expected_run_id:material.prepared.run_id.clone(), authority,
            }).map_err(|error|error.to_string())?
    } else {None};
    let prepared=material.prepared;
    if prepared.input.target_conversation_id!=target || prepared.admitted_run_id.is_some() {
        return Err("edited_resend_target_already_admitted_or_foreign".into());
    }
    let primary:HostModelRouteBinding=serde_json::from_value(prepared.source.model.primary.clone()).map_err(|_|"edited_resend_invalid_primary_model")?;
    let fallbacks:Vec<HostModelRouteBinding>=serde_json::from_value(prepared.source.model.fallbacks.clone()).map_err(|_|"edited_resend_invalid_fallback_models")?;
    let generation:nola_host_protocol::HostGenerationBinding=serde_json::from_value(prepared.source.model.generation.clone()).map_err(|_|"edited_resend_invalid_generation")?;
    let output:nola_host_protocol::HostOutputAdmission=serde_json::from_value(prepared.source.model.output_admission.clone()).map_err(|_|"edited_resend_invalid_output_admission")?;
    let store=NolaStore::open_default()?;
    let vault=crate::host_credentials::HostCredentialVault::open(store.path())?;
    let descriptor=if let Some(reference)=material.model_credential_reference {
        if reference.adapter!=ADAPTER {return Err("edited_resend_model_credential_adapter_unavailable".into());}
        let encoded=vault.runtime_descriptor(&reference.handle)?.ok_or("edited_resend_model_credential_unavailable")?;
        if sha(encoded.as_bytes())!=reference.sha256 {return Err("edited_resend_model_credential_integrity_failure".into());}
        let descriptor:Descriptor=serde_json::from_str(&encoded).map_err(|_|"edited_resend_invalid_private_model_descriptor")?;
        // The private Store accessor already checked capability facts against
        // the original digest-bound Core config. The Host descriptor
        // contains only its model binding, so compare just that route fragment
        // while retaining those independently verified intrinsic facts.
        let fragment=nola_domain::edited_resend::FrozenResendModel::from_invocation(&serde_json::json!({
            "turn_identity":{"host_invocation":descriptor.binding,
                "model_capability":prepared.source.model.model_capability}
        })).ok_or("edited_resend_private_model_missing")?;
        if fragment!=prepared.source.model || descriptor.request.session.session_id.as_deref()!=Some(prepared.source.conversation_id.as_str())
            || !descriptor.request.session.save_session || descriptor.credentials.len()!=fallbacks.len()+1 {
            return Err("edited_resend_private_model_owner_mismatch".into());
        }
        Some(descriptor)
    } else {None};
    let mut credentials=Vec::new();
    for (index,route) in std::iter::once(&primary).chain(&fallbacks).enumerate() {
        let HostModelRouteBinding::Http(route)=route else {credentials.push(String::new());continue;};
        if route.endpoint_query_sha256.is_some() {return Err("edited_resend_endpoint_query_credential_unavailable".into());}
        let secret=if route.credential_sha256==nola_host_protocol::EMPTY_CREDENTIAL_SHA256 {String::new()} else {
            let descriptor=descriptor.as_ref().ok_or("edited_resend_model_credential_unavailable")?;
            let handle=descriptor.credentials[index].as_ref().ok_or("edited_resend_model_credential_unavailable")?;
            let record=vault.record(handle)?.ok_or("edited_resend_model_credential_removed")?;
            if record.identity!=crate::host_credentials::identity(&route.endpoint,route.protocol.as_str())? {
                return Err("edited_resend_model_credential_endpoint_changed".into());
            }
            record.secret.ok_or("edited_resend_model_credential_revoked")?
        };
        if sha(secret.as_bytes())!=route.credential_sha256 {return Err("edited_resend_model_credential_changed".into());}
        credentials.push(secret);
    }
    let prompt_cache_key=descriptor.as_ref().and_then(|d|d.request.runtime.openai_prompt_cache_key.clone());
    let prompt_cache_key_prefix=descriptor.as_ref().and_then(|d|d.request.runtime.openai_prompt_cache_key_prefix.clone());
    if prompt_cache_key.as_ref().map(|key|sha(key.as_bytes()))!=generation.prompt_cache_key_sha256
        || prompt_cache_key_prefix.as_ref().map(|key|sha(key.as_bytes()))!=generation.prompt_cache_key_prefix_sha256 {
        return Err("edited_resend_model_cache_material_unavailable".into());
    }
    Ok(EditedResendModelMaterial {prepared,primary,fallbacks,generation,output,credentials,child_instructions,prompt_cache_key,prompt_cache_key_prefix})
}
