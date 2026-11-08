// SPDX-License-Identifier: Apache-2.0
// Submission transport does not manufacture an admitted Run from a POST or EOF.
use nola_domain::remote_submission::{
    RemoteSubmissionIdentity, RemoteSubmissionPhase, SourceSubmissionReceipt,
};

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum SubmissionReceiptRequest {
    Prepare {
        identity: RemoteSubmissionIdentity,
        request: Value,
    },
    Observe {
        identity: RemoteSubmissionIdentity,
        request_digest: String,
    },
    Cancel {
        identity: RemoteSubmissionIdentity,
        request_digest: String,
    },
}

fn submission_wire_identity(
    request: &Value,
    host: &str,
) -> Result<RemoteSubmissionIdentity, String> {
    let field = |name: &str| -> Result<String, String> {
        let value = request
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing submission field: {name}"))?;
        if value.trim() != value || value.chars().any(char::is_control) {
            return Err(format!("invalid submission field: {name}"));
        }
        Ok(value.to_owned())
    };
    let identity = RemoteSubmissionIdentity {
        source_host_id: field("expected_host_id")?,
        client_id: field("client_id")?,
        conversation_id: field("conversationId")?,
        submission_id: field("submission_id")?,
        user_message_id: field("user_message_id")?,
        assistant_message_id: field("assistant_message_id")?,
    };
    identity.validate()?;
    if request.get("session_id").is_some() {
        return Err("session_id request alias was removed; use conversationId".into());
    }
    if request.get("replace_active_run").is_some() || request.get("replaceActiveRun").is_some() {
        return Err(
            "implicit Run replacement was removed; cancel and observe the exact Run".into(),
        );
    }
    if identity.source_host_id != host {
        return Err("submission source or conversation identity mismatch".into());
    }
    Ok(identity)
}

fn submission_receipt_response(
    request: &mut Request,
    mobile: bool,
) -> Response<Box<dyn Read + Send>> {
    let token = if mobile {
        match mobile_bearer_token(request) {
            Some(token) => Some(token),
            None => return json_response(StatusCode(401), json!({"error":"unauthorized"})),
        }
    } else {
        None
    };
    let result = (|| -> Result<Value, String> {
        // Frozen requests may contain attachments; observations themselves stay small.
        const LIMIT: u64 = 24 * 1024 * 1024;
        if request.body_length().is_some_and(|len| len as u64 > LIMIT) {
            return Err("submission request exceeds budget".into());
        }
        let mut bytes = Vec::new();
        request
            .as_reader()
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("submission request exceeds budget".into());
        }
        if token.is_some()
            && request_header(request, "X-Nola-Mobile-Auth").is_some()
            && !nola_core::mobile_crypto::mobile_body_sha256(&bytes).eq_ignore_ascii_case(
                request_header(request, "X-Nola-Mobile-Body-SHA256").unwrap_or(""),
            )
        {
            return Err("mobile request body authentication failed".into());
        }
        let raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let body = match token.as_deref() {
            Some(token) => nola_core::mobile_crypto::decrypt_mobile_json(
                &serde_json::from_value::<nola_core::mobile_crypto::MobileEncryptedEnvelope>(raw)
                    .map_err(|e| e.to_string())?,
                token,
            )?,
            None => raw,
        };
        let input: SubmissionReceiptRequest =
            serde_json::from_value(body).map_err(|e| e.to_string())?;
        let host = host_identity();
        let (identity, digest, cancel) = match input {
            SubmissionReceiptRequest::Prepare { identity, request } => {
                if identity != submission_wire_identity(&request, host)? {
                    return Err("submission payload owner mismatch".into());
                }
                let digest = nola_domain::remote_submission::submission_request_digest(&request)?;
                // This only canonicalizes. No receipt or execution claim exists until POST /stream.
                return Ok(
                    json!({"source_host_id":host,"result":{"identity":identity,"request_digest":digest}}),
                );
            }
            SubmissionReceiptRequest::Observe {
                identity,
                request_digest,
            } => (identity, request_digest, false),
            SubmissionReceiptRequest::Cancel {
                identity,
                request_digest,
            } => (identity, request_digest, true),
        };
        identity.validate()?;
        nola_domain::remote_submission::validate_request_digest(&digest)?;
        if identity.source_host_id != host {
            return Err("submission source identity mismatch".into());
        }
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        let result = observation_command(
            &mut application,
            json!({
                "type":if cancel {"cancel_source_submission"} else {"read_source_submission_receipt"},
                "identity":identity,"request_digest":digest
            }),
        )?;
        Ok(json!({"source_host_id":host,"result":result}))
    })();
    let (status, payload) = match result {
        Ok(value) => (StatusCode(200), value),
        Err(error) => (
            StatusCode(409),
            json!({"code":"submission_unavailable","error":error}),
        ),
    };
    match token {
        Some(token) => mobile_encrypted_json_response(status, payload, &token),
        None => json_response(status, payload),
    }
}

/// A source registration owns one attempt to dispatch. A duplicate observes it;
/// even a crash before admission does not confer permission to execute twice.
struct SourceSubmissionDispatch {
    identity: RemoteSubmissionIdentity,
    digest: String,
    owns_dispatch: bool,
    _preflight: Option<SourceAdmissionFence>,
}
impl SourceSubmissionDispatch {
    fn settle(&self) -> Result<SourceSubmissionReceipt, String> {
        if let Some(fence)=&self._preflight {
            if !fence.owns()? {return Err("source admission preflight owner changed".into());}
        }
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        serde_json::from_value(observation_command(
            &mut application,
            json!({
                "type":"reject_source_submission","identity":self.identity,
                "request_digest":self.digest,"rejection_code":"dispatch_not_admitted"
            }),
        )?)
        .map_err(|e| e.to_string())
    }

    fn finish_stream(&self, tx: &mpsc::Sender<Vec<u8>>, token: Option<&str>) {
        let event = match self.settle() {
            Ok(receipt) => json!({"type":"submission_receipt","receipt":receipt}),
            Err(_) => json!({"type":"error","code":"submission_recovery_required",
                "message":"Source receipt is unavailable; observe the exact submission before retrying."}),
        };
        let _ = tx.send(mobile_sse_event(event, token));
        let _ = tx.send(b"data: [DONE]\n\n".to_vec());
    }

    fn read(&self) -> Result<SourceSubmissionReceipt, String> {
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        serde_json::from_value(observation_command(&mut application, json!({
            "type":"read_source_submission_receipt","identity":self.identity,"request_digest":self.digest
        }))?).map_err(|e| e.to_string())
    }
}
impl Drop for SourceSubmissionDispatch {
    fn drop(&mut self) {
        if !self.owns_dispatch {
            return;
        }
        // Atomic Store transition rejects only pending receipts; admitted Runs
        // retain their real lifecycle even on transport failure or unwinding.
        let _ = self.settle();
    }
}

fn register_stream_submission(
    body: &mut Value,
) -> Result<(SourceSubmissionDispatch, SourceSubmissionReceipt, bool), String> {
    let (identity, digest) = validate_stream_submission(body, host_identity())?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let registered: nola_domain::remote_submission::RegisteredSourceSubmission =
        serde_json::from_value(observation_command(
            &mut application,
            json!({"type":"register_source_submission","identity":identity,"request":body}),
        )?)
        .map_err(|e| e.to_string())?;
    Ok((
        SourceSubmissionDispatch {
            identity,
            digest,
            owns_dispatch: registered.created,
            _preflight: None,
        },
        registered.receipt,
        registered.created,
    ))
}

/// Observation precedes ephemeral preparation checks only for a previously
/// admitted exact request. It never creates a receipt or dispatches a Run.
fn admitted_stream_submission_receipt_with(
    body: &Value,
    host: &str,
    read: impl FnOnce(&RemoteSubmissionIdentity, &str) -> Result<Option<SourceSubmissionReceipt>, String>,
) -> Result<Option<SourceSubmissionReceipt>, String> {
    let mut semantic = body.clone();
    let (identity, digest) = validate_stream_submission(&mut semantic, host)?;
    let Some(receipt) = read(&identity, &digest)? else { return Ok(None); };
    receipt.validate()?;
    if receipt.identity != identity || receipt.request_digest != digest {
        return Err("submission receipt identity or digest mismatch".into());
    }
    Ok((receipt.phase == RemoteSubmissionPhase::Admitted).then_some(receipt))
}

fn admitted_stream_submission_receipt(body: &Value) -> Result<Option<SourceSubmissionReceipt>, String> {
    admitted_stream_submission_receipt_with(body, host_identity(), |identity,digest| {
        let mut application=crate::application_port::CliApplicationPort::persistent_default()?;
        application.execute(nola_application::ApplicationCommand::ReadSourceSubmissionReceipt {
            identity:identity.clone(),request_digest:digest.to_owned(),
        })
    })
}

fn validate_stream_submission(
    body: &mut Value,
    host: &str,
) -> Result<(RemoteSubmissionIdentity, String), String> {
    let identity = submission_wire_identity(body, host)?;
    let digest = body
        .as_object_mut()
        .and_then(|map| map.remove("request_digest"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("missing submission request_digest")?;
    if nola_domain::remote_submission::submission_request_digest(body)? != digest {
        return Err("submission request digest mismatch".into());
    }
    Ok((identity, digest))
}

#[cfg(test)]
mod submission_receipt_tests {
    use super::*;

    fn admitted_receipt(body: &Value) -> SourceSubmissionReceipt {
        let mut semantic=body.clone();
        let (identity,digest)=validate_stream_submission(&mut semantic,"host").unwrap();
        SourceSubmissionReceipt { protocol_version:1,request_digest:digest,
            phase:RemoteSubmissionPhase::Admitted,rejection_code:None,
            admission:Some(nola_domain::run_message_admission::RunMessageAdmissionCursor {
                identity:nola_domain::assistant_journal::AssistantJournalIdentity {
                    source_store_epoch:"fixture-epoch".into(),conversation_id:identity.conversation_id.clone(),
                    run_id:identity.submission_id.clone(),assistant_message_id:identity.assistant_message_id.clone(),
                },sequence:nola_domain::assistant_journal_operations::JournalNumber(1),
                record_digest:nola_domain::run_message_admission::admission_digest(b"fixture"),
            }),identity }
    }

    #[test]
    fn admitted_retry_is_observation_without_reading_preparation() {
        let id=uuid::Uuid::new_v4().to_string();
        let mut original=payload();
        original["tool_plan_ref"]=json!({"manifest_id":id,"host_id":"host","client_id":"client",
            "session_id":"conversation","catalog_revision":"a".repeat(64)});
        let wire=bind(original);
        let receipt=admitted_receipt(&wire);
        // This path depends only on a validated durable source receipt. It
        // cannot consult preparation storage, whether expired or reclaimed.
        let observed=admitted_stream_submission_receipt_with(&wire,"host",|identity,digest| {
            assert_eq!(identity,&receipt.identity);assert_eq!(digest,receipt.request_digest);
            Ok(Some(receipt.clone()))
        }).unwrap().unwrap();
        assert_eq!(observed,receipt);
    }

    #[test]
    fn retry_rejects_tampered_digest_or_receipt_and_does_not_admit_prepared_intent() {
        let wire=bind(payload());let receipt=admitted_receipt(&wire);
        let mut changed=wire.clone();changed["messages"][0]["content"]="changed".into();
        assert!(admitted_stream_submission_receipt_with(&changed,"host",|_,_|panic!("invalid digest must not reach Store")).is_err());
        let mut changed_semantic=changed.clone();changed_semantic.as_object_mut().unwrap().remove("request_digest");
        let changed_wire=bind(changed_semantic);
        assert!(admitted_stream_submission_receipt_with(&changed_wire,"host",|_,_|Ok(Some(receipt.clone()))).is_err());
        let mut prepared=receipt;prepared.phase=RemoteSubmissionPhase::Prepared;prepared.admission=None;
        assert!(admitted_stream_submission_receipt_with(&wire,"host",|_,_|Ok(Some(prepared))).unwrap().is_none());
        assert!(admitted_stream_submission_receipt_with(&wire,"host",|_,_|Ok(None)).unwrap().is_none());
    }

    #[test]
    fn durable_host_identity_never_silently_replaces_invalid_pairing_state() {
        let directory = tempfile::tempdir().unwrap();
        let identity = load_or_create_host_identity_at(directory.path()).unwrap();
        assert_eq!(
            load_or_create_host_identity_at(directory.path()).unwrap(),
            identity
        );
        let path = directory.path().join("host-identity");
        fs::write(&path, "invalid identity\n").unwrap();
        assert!(load_or_create_host_identity_at(directory.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "invalid identity\n");
    }

    fn payload() -> Value {
        json!({"expected_host_id":"host","client_id":"client","conversationId":"conversation",
            "submission_id":"submission","user_message_id":"user","assistant_message_id":"assistant",
            "messages":[{"role":"user","content":"当前 search / rechercher"}],
            "model_intent":{"model_id":"model","temperature":0.25}})
    }

    fn bind(mut value: Value) -> Value {
        value["request_digest"] = nola_domain::remote_submission::submission_request_digest(&value)
            .unwrap()
            .into();
        value
    }

    #[test]
    fn submission_digest_binds_original_request_and_owner() {
        let original = payload();
        let mut wire = bind(original.clone());
        let (identity, digest) = validate_stream_submission(&mut wire, "host").unwrap();
        assert_eq!(wire, original);
        assert_eq!(identity.submission_id, "submission");
        assert_eq!(
            digest,
            nola_domain::remote_submission::submission_request_digest(&original).unwrap()
        );
        for key in [
            "client_id",
            "submission_id",
            "conversationId",
            "user_message_id",
            "assistant_message_id",
        ] {
            let mut changed = bind(original.clone());
            changed[key] = "another-owner".into();
            assert!(
                validate_stream_submission(&mut changed, "host").is_err(),
                "{key}"
            );
        }
        let mut changed = bind(original);
        changed["model_intent"]["temperature"] = json!(0.75);
        assert!(validate_stream_submission(&mut changed, "host").is_err());
    }

    #[test]
    fn submission_rejects_unbound_wrong_host_and_old_identity_alias() {
        assert!(validate_stream_submission(&mut payload(), "host").is_err());
        assert!(validate_stream_submission(&mut bind(payload()), "another-host").is_err());
        let mut alias = payload();
        alias["session_id"] = "conversation".into();
        assert!(validate_stream_submission(&mut bind(alias), "host").is_err());
        let mut collision = payload();
        collision["assistant_message_id"] = "user".into();
        assert!(validate_stream_submission(&mut bind(collision), "host").is_err());
    }

    #[test]
    fn submission_observation_never_accepts_an_execution_payload() {
        let identity = submission_wire_identity(&payload(), "host").unwrap();
        let digest = nola_domain::remote_submission::submission_request_digest(&payload()).unwrap();
        for operation in ["observe", "cancel"] {
            let valid = json!({"operation":operation,"identity":identity,"request_digest":digest});
            assert!(serde_json::from_value::<SubmissionReceiptRequest>(valid.clone()).is_ok());
            let mut invalid = valid;
            invalid["request"] = payload();
            assert!(serde_json::from_value::<SubmissionReceiptRequest>(invalid).is_err());
        }
    }
}

/// Shared by event and permission delivery so admission always precedes the
/// first journal or permission envelope, including synchronous tool callbacks.
struct SubmissionReceiptFence {
    dispatch: Arc<SourceSubmissionDispatch>,
    sent_admission: Mutex<bool>,
}
impl SubmissionReceiptFence {
    fn deliver(&self, tx: &mpsc::Sender<Vec<u8>>, token: Option<&str>) -> Result<(), String> {
        let mut sent = self
            .sent_admission
            .lock()
            .map_err(|_| "submission fence unavailable")?;
        if !*sent {
            let receipt = self.dispatch.read()?;
            receipt.validate()?;
            if receipt.phase != RemoteSubmissionPhase::Admitted {
                return Err("submission not admitted".into());
            }
            tx.send(mobile_sse_event(
                json!({"type":"submission_receipt","receipt":receipt}),
                token,
            ))
            .map_err(|_| "submission transport closed")?;
            *sent = true;
        }
        Ok(())
    }
}

/// Local pre-admission exclusion, using the existing owner-qualified lease table.
/// It is independent of the actual conversation/Run lease and grants no effect.
struct SourceAdmissionFence {store:NolaStore,key:String,owner:String}
impl SourceAdmissionFence {
    fn owns(&self)->Result<bool,String> {
        Ok(self.store.session_run_lock(&self.key)?.is_some_and(|lease|lease.owner==self.owner))
    }
    fn acquire(identity:&RemoteSubmissionIdentity,digest:&str)->Result<Option<Self>,String> {
        Self::acquire_in_store(NolaStore::open_default()?,identity,digest)
    }
    fn acquire_in_store(store:NolaStore,identity:&RemoteSubmissionIdentity,digest:&str)->Result<Option<Self>,String> {
        let key=source_admission_key(identity,digest)?;
        let process=crate::process_observation::current_worker_identity()?;
        let owner=format!("source-admission:{}",process.nonce);
        let lease=crate::store::DbSessionRunLock {session_id:key.clone(),owner:owner.clone(),pid:process.pid,
            process_start:serde_json::to_string(&process).map_err(|e|e.to_string())?,acquired_at_ms:now_unix_millis()};
        for _ in 0..3 {
            if store.try_acquire_session_run_lock(&lease)? {return Ok(Some(Self{store,key,owner}));}
            let Some(old)=store.session_run_lock(&key)? else {continue;};
            let Ok(expected)=serde_json::from_str::<nola_host_protocol::HostWorkerProcessIdentity>(&old.process_start) else {return Ok(None);};
            if expected.pid!=old.pid || expected.validate().is_err() {return Ok(None);}
            let gone=expected.is_terminated(&crate::process_observation::observe_process(old.pid)).unwrap_or(false);
            if !gone {return Ok(None);}
            if !store.release_session_run_lock(&key,&old.owner)? {continue;}
        }
        Ok(None)
    }
}
impl Drop for SourceAdmissionFence {
    fn drop(&mut self){let _=self.store.release_session_run_lock(&self.key,&self.owner);}
}

/// Constructed only after the Room Core claim validated the exact frozen intent.
struct RoomSourceAdmissionRetry {
    identity:RemoteSubmissionIdentity,
    digest:String,
    // Trusted, immutable Core manifest data; never accepted from the HTTP body.
    prompt_definitions:Option<Vec<nola_domain::expert_message::ExpertPromptDefinition>>,
}

fn source_admission_key(identity:&RemoteSubmissionIdentity,digest:&str)->Result<String,String> {
    use sha2::{Digest,Sha256};
    Ok(format!("source-admission/{:x}",Sha256::digest(serde_json::to_vec(&(identity,digest)).map_err(|e|e.to_string())?)))
}

#[cfg(all(test,any(target_os="macos",target_os="linux")))]
mod source_admission_fence_tests {
    use super::*;
    fn identity()->RemoteSubmissionIdentity {
        RemoteSubmissionIdentity {source_host_id:"host".into(),client_id:"room:room".into(),conversation_id:"child".into(),
            submission_id:"submission".into(),user_message_id:"submission:user".into(),assistant_message_id:"submission:assistant".into()}
    }
    #[test]
    fn live_preflight_excludes_retry_and_stale_drop_cannot_release_new_owner()->Result<(),String> {
        let directory=tempfile::tempdir().map_err(|e|e.to_string())?;
        let path=directory.path().join("nola.db");let id=identity();let digest="a".repeat(64);
        let first=SourceAdmissionFence::acquire_in_store(NolaStore::open(&path)?,&id,&digest)?.ok_or("first fence unavailable")?;
        assert!(first.owns()?);
        assert!(SourceAdmissionFence::acquire_in_store(NolaStore::open(&path)?,&id,&digest)?.is_none());
        let store=NolaStore::open(&path)?;
        assert!(store.release_session_run_lock(&first.key,&first.owner)?);
        let second=SourceAdmissionFence::acquire_in_store(NolaStore::open(&path)?,&id,&digest)?.ok_or("new fence unavailable")?;
        assert!(!first.owns()?);drop(first);assert!(second.owns()?);
        let key=second.key.clone();drop(second);assert!(store.session_run_lock(&key)?.is_none());
        Ok(())
    }
    #[test]
    fn proven_process_identity_change_can_recover_but_unknown_identity_is_preserved()->Result<(),String> {
        let directory=tempfile::tempdir().map_err(|e|e.to_string())?;
        let path=directory.path().join("nola.db");let id=identity();let digest="b".repeat(64);
        let store=NolaStore::open(&path)?;let key=source_admission_key(&id,&digest)?;
        let mut previous=crate::process_observation::current_worker_identity()?;
        previous.start_id.push_str("-different-process-instance");
        let old=crate::store::DbSessionRunLock {session_id:key.clone(),owner:"old-preflight".into(),pid:previous.pid,
            process_start:serde_json::to_string(&previous).unwrap(),acquired_at_ms:1};
        assert!(store.try_acquire_session_run_lock(&old)?);
        let recovered=SourceAdmissionFence::acquire_in_store(NolaStore::open(&path)?,&id,&digest)?.ok_or("proven stale fence not reclaimed")?;
        assert!(recovered.owns()?);drop(recovered);
        let unknown=crate::store::DbSessionRunLock {process_start:"unavailable".into(),..old};
        assert!(store.try_acquire_session_run_lock(&unknown)?);
        assert!(SourceAdmissionFence::acquire_in_store(NolaStore::open(&path)?,&id,&digest)?.is_none());
        assert_eq!(store.session_run_lock(&key)?.unwrap().owner,"old-preflight");
        Ok(())
    }
}
