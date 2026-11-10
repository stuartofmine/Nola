// SPDX-License-Identifier: Apache-2.0
// Paired maintenance is executed by the source Host against its canonical context.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedContextCompactionRequest {
    expected_host_id: String,
    client_id: String,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
}

fn validate_paired_context_compaction(input: &PairedContextCompactionRequest, host: &str) -> Result<(), String> {
    if input.expected_host_id != host || [&input.expected_host_id, &input.client_id,
        &input.conversation_id, &input.expected_run_id, &input.assistant_message_id].iter().any(|id|
            id.is_empty() || id.trim() != id.as_str() || id.len() > 512 || id.chars().any(char::is_control)) {
        return Err("context maintenance source owner mismatch".into());
    }
    Ok(())
}

fn authorize_paired_context_compaction(
    application: &mut crate::application_port::CliApplicationPort,
    input: &PairedContextCompactionRequest,
    host: &str,
) -> Result<(), String> {
    validate_paired_context_compaction(input, host)?;
    let source: Option<nola_domain::remote_submission::SourceSubmissionReceipt> = application.execute(
        nola_application::ApplicationCommand::ObservedSourceSubmissionReceipt {
            source_host_id: host.into(), client_id: input.client_id.clone(),
            conversation_id: input.conversation_id.clone(), run_id: input.expected_run_id.clone(),
            assistant_message_id: input.assistant_message_id.clone(),
        })?;
    let source = source.ok_or("context maintenance has no admitted source client owner")?;
    source.validate()?;
    if source.phase != nola_domain::remote_submission::RemoteSubmissionPhase::Admitted {
        return Err("context maintenance source is not admitted".into());
    }
    if application.active_run(&input.conversation_id)?.is_some() {
        return Err("context maintenance cannot overlap an active Run".into());
    }
    if application.load_checkpoint(&input.conversation_id)?.is_none() {
        return Err("source conversation checkpoint is unavailable".into());
    }
    Ok(())
}

fn paired_context_compaction_response(request: &mut Request, _config: &Config) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body, Err(response) => return response,
    };
    let input: PairedContextCompactionRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(_) => return mobile_encrypted_json_response(StatusCode(422),
            json!({"code":"invalid_context_compaction_request"}), &token),
    };
    let validated = (|| -> Result<crate::application_port::CliApplicationPort, String> {
        validate_paired_context_compaction(&input, host_identity())?;
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        authorize_paired_context_compaction(&mut application, &input, host_identity())?;
        Ok(application)
    })();
    let mut application = match validated {
        Ok(application) => application,
        Err(error) => return mobile_encrypted_json_response(StatusCode(409),
            json!({"code":"context_compaction_unavailable","error":error}), &token),
    };
    // Reconstruct the exact admitted source invocation through the same Host
    // recovery boundary. No browser override or current default is substituted.
    let result = crate::core_agent_runtime::compact_admitted_core_agent_run(
        &input.conversation_id, &input.expected_run_id, &input.assistant_message_id)
        .and_then(|result| {
            let checkpoint = application.load_checkpoint(&input.conversation_id)?
                .ok_or("committed source checkpoint is unavailable")?;
            Ok(json!({
                "source_host_id":host_identity(), "conversation_id":input.conversation_id,
                "compacted":result.replaced_item_count > 0,
                "checkpoint_revision":checkpoint.revision, "committed_at_ms":checkpoint.updated_at_ms,
                "context": {"used_tokens":result.context_used_tokens,
                    "context_window_tokens":result.context_window_tokens,
                    "usage_percent":result.context_usage_percent,
                    "replaced_item_count":result.replaced_item_count,
                    "remaining_item_count":result.remaining_item_count}
            }))
        });
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200),
            json!({"source_host_id":host_identity(), "result":value}), &token),
        Err(error) => mobile_encrypted_json_response(StatusCode(409),
            json!({"code":"context_compaction_unavailable","error":error,"outcome_unknown":true}), &token),
    }
}

#[cfg(test)]
mod paired_context_compaction_contract_tests {
    use super::*;
    #[test]
    fn source_context_maintenance_rejects_foreign_owner_and_client_context() {
        let valid = json!({"expected_host_id":"host", "client_id":"client", "conversation_id":"conversation",
            "expected_run_id":"run", "assistant_message_id":"assistant"});
        let input = serde_json::from_value(valid.clone()).unwrap();
        validate_paired_context_compaction(&input, "host").unwrap();
        assert!(validate_paired_context_compaction(&input, "other-host").is_err());
        for field in ["messages", "prompt", "modelProfileId", "profile", "api_key", "command", "checkpoint"] {
            let mut invalid = valid.clone(); invalid[field] = json!("forbidden");
            assert!(serde_json::from_value::<PairedContextCompactionRequest>(invalid).is_err());
        }
        let mut invalid = valid; invalid["client_id"] = json!(" client ");
        assert!(validate_paired_context_compaction(&serde_json::from_value(invalid).unwrap(), "host").is_err());
    }
    #[test]
    fn missing_source_admission_cannot_start_maintenance() {
        let input = PairedContextCompactionRequest { expected_host_id:"host".into(), client_id:"client".into(),
            conversation_id:"conversation".into(), expected_run_id:"run".into(), assistant_message_id:"assistant".into() };
        let mut application = crate::application_port::CliApplicationPort::ephemeral().unwrap();
        let error = authorize_paired_context_compaction(&mut application, &input, "host").unwrap_err();
        assert!(error.contains("no admitted source client owner"), "{error}");
        assert!(application.active_run("conversation").unwrap().is_none());
    }

}
