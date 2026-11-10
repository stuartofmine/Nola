// SPDX-License-Identifier: Apache-2.0
// Paired clients can update one conversation permission profile, not arbitrary
// Host settings. The original source submission proves client/Run ownership.

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum PairedPermissionConfigurationOperation { Read, Patch }

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedPermissionConfigurationRequest {
    operation: PairedPermissionConfigurationOperation,
    expected_host_id: String,
    client_id: String,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
    expected_revision: Option<i64>,
    permission_profile: Option<nola_domain::PermissionProfile>,
    minimum_policy_revision: Option<i64>,
}

fn paired_permission_configuration_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let input: PairedPermissionConfigurationRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(_) => return mobile_encrypted_json_response(StatusCode(422),
            json!({"code":"invalid_permission_configuration_request"}), &token),
    };
    let result = (|| {
        validate_paired_permission_configuration(&input, host_identity())?;
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        paired_permission_configuration(&mut application, host_identity(), input)
    })();
    let (status, value) = match result {
        Ok(value) => value,
        Err(error) => (StatusCode(409), json!({"code":"permission_configuration_unavailable","error":error})),
    };
    mobile_encrypted_json_response(status, value, &token)
}

fn validate_paired_permission_configuration(input: &PairedPermissionConfigurationRequest, host: &str) -> Result<(), String> {
    if input.expected_host_id != host || [&input.expected_host_id, &input.client_id, &input.conversation_id,
        &input.expected_run_id, &input.assistant_message_id].iter().any(|id|
            id.is_empty() || id.trim() != id.as_str() || id.len() > 512 || id.chars().any(char::is_control)) {
        return Err("permission configuration owner mismatch".into());
    }
    if input.minimum_policy_revision.is_some_and(|revision| revision < 1) { return Err("invalid minimum policy revision".into()); }
    match input.operation {
        PairedPermissionConfigurationOperation::Read if input.expected_revision.is_none() && input.permission_profile.is_none() => Ok(()),
        PairedPermissionConfigurationOperation::Patch if input.expected_revision.is_some_and(|r| r > 0) && input.permission_profile.is_some() => Ok(()),
        _ => Err("permission configuration operation has invalid fields".into()),
    }
}

fn paired_permission_configuration(
    application: &mut crate::application_port::CliApplicationPort,
    host: &str,
    input: PairedPermissionConfigurationRequest,
) -> Result<(StatusCode, Value), String> {
    use nola_application::ApplicationCommand as Command;
    use nola_domain::conversation_configuration::ConfigurationCommitResult;
    validate_paired_permission_configuration(&input, host)?;
    let source: Option<nola_domain::remote_submission::SourceSubmissionReceipt> = application.execute(
        Command::ObservedSourceSubmissionReceipt { source_host_id: host.into(), client_id: input.client_id.clone(),
            conversation_id: input.conversation_id.clone(), run_id: input.expected_run_id.clone(),
            assistant_message_id: input.assistant_message_id.clone() })?;
    let source = source.ok_or("permission configuration has no admitted source client owner")?;
    source.validate()?;
    if source.phase != nola_domain::remote_submission::RemoteSubmissionPhase::Admitted {
        return Err("permission configuration source is not admitted".into());
    }
    let active = application.active_run(&input.conversation_id)?;
    if active.as_ref().is_some_and(|run| run.run_id != input.expected_run_id
        || run.assistant_message_id.as_deref() != Some(input.assistant_message_id.as_str())) {
        return Err("active permission configuration Run changed; rediscover the owner".into());
    }
    if matches!(input.operation, PairedPermissionConfigurationOperation::Patch) {
        let profile = input.permission_profile.ok_or("missing permission profile")?;
        let result: ConfigurationCommitResult = application.execute(Command::SetConversationPermission {
            conversation_id: input.conversation_id.clone(),
            expected_revision: input.expected_revision.ok_or("missing revision")?,
            expected_run_id: active.as_ref().map(|run| run.run_id.clone()),
            expected_source_host_id: None, profile, additional_configuration:None,
            updated_at_ms: Utc::now().timestamp_millis(),
        })?;
        match result {
            ConfigurationCommitResult::Committed { .. } => {},
            ConfigurationCommitResult::Conflict { expected_revision, actual_revision, .. } => {
                return Ok((StatusCode(409), json!({"code":"configuration_conflict","expected_revision":expected_revision,"actual_revision":actual_revision})));
            }
            ConfigurationCommitResult::NotFound { .. } => return Ok((StatusCode(404), json!({"code":"conversation_not_found"}))),
        }
    }
    let snapshot: Value = application.execute(Command::ReadPermissionConfiguration {
        conversation_id: input.conversation_id.clone(), source_host_id: Some(host.into()),
        client_id: Some(input.client_id.clone()), unresolved_run_ids: Vec::new(), source_unconfirmed: false, minimum_policy_revision: input.minimum_policy_revision,
    })?;
    Ok((StatusCode(200), json!({"source_host_id":host,"result":snapshot})))
}

#[cfg(test)]
mod paired_permission_configuration_contract_tests {
    use super::*;
    #[test]
    fn paired_configuration_is_not_an_arbitrary_settings_or_application_command_channel() {
        let valid = json!({"operation":"patch","expected_host_id":"host","client_id":"client",
            "conversation_id":"conversation","expected_run_id":"run","assistant_message_id":"assistant",
            "expected_revision":1,"permission_profile":"allow_all"});
        let input = serde_json::from_value(valid.clone()).unwrap();
        validate_paired_permission_configuration(&input, "host").unwrap();
        assert!(validate_paired_permission_configuration(&input, "foreign").is_err());
        for field in ["database", "command", "modelOverrides", "api_key"] {
            let mut invalid = valid.clone(); invalid[field] = json!("forbidden");
            assert!(serde_json::from_value::<PairedPermissionConfigurationRequest>(invalid).is_err());
        }
        let mut obsolete = valid; obsolete["permission_profile"] = json!("safe");
        assert!(serde_json::from_value::<PairedPermissionConfigurationRequest>(obsolete).is_err());
    }
}
