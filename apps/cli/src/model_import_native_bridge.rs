// SPDX-License-Identifier: Apache-2.0
//! Private native service pipe, not a CLI/UI receipt channel. Source parsing,
//! preview and commit are the same Host implementation used by Web and CLI.
use crate::{model_connection_host::*, store::NolaStore};
use nola_domain::{generation::ModelProtocol, model_connection::ModelConnection};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};

const MAX_FRAME: u64 = 4 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Plan {
        request_id: String,
        request: ImportPlanRequest,
    },
    Commit {
        request_id: String,
        request: ImportCommitRequest,
    },
    ProbePrepare {
        request_id: String,
        request: NativeProbeRequest,
    },
    Probe {
        request_id: String,
        request: crate::model_capability_probe::ProbeConfirmation,
    },
    Close,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeProbeRequest {
    connection_id: String,
    expected_revision: i64,
    model_id: String,
    protocol: ModelProtocol,
}

fn read_native_credential(
    store: &NolaStore,
    input: &mut impl BufRead,
    output: &mut impl Write,
    request_id: &str,
    connection: &ModelConnection,
    protocol: ModelProtocol,
) -> Result<(String, Option<String>), String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    write_frame(
        output,
        json!({"type":"credential_read_request", "request_id":request_id, "nonce":nonce,
        "connection_id":connection.id,"account_id":connection.account_id,"endpoint":connection.base_url,"protocol":protocol}),
    )?;
    let reply = read_frame(input)?.ok_or("native_credential_source_closed")?;
    if reply["type"] != "credential_read"
        || reply["request_id"] != request_id
        || reply["nonce"] != nonce
    {
        return Err("native_credential_source_rejected".into());
    }
    let secret = if connection.account_id.is_some() {
        Some(
            reply["secret"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("native_credential_source_missing")?
                .to_string(),
        )
    } else {
        None
    };
    let account: Value = if let Some(id) = &connection.account_id {
        store
            .application()?
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::GetAccount { id: id.clone() },
            })?
    } else {
        Value::Null
    };
    // Native Keychain has no Host vault revision. Bind the exact service-read
    // material plus route/account revision; only the digest leaves this pipe.
    let binding = json!({"owner":"native_keychain","connection":connection,"account":account,"secret":secret});
    Ok((
        format!("{:x}", Sha256::digest(binding.to_string().as_bytes())),
        secret,
    ))
}
fn read_frame(input: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut bytes = vec![];
    input
        .take(MAX_FRAME + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "native_import_pipe_failed")?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_FRAME || !bytes.ends_with(b"\n") {
        return Err("native_import_frame_invalid".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "native_import_frame_invalid".into())
}
fn write_frame(output: &mut impl Write, value: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *output, &value).map_err(|_| "native_import_pipe_failed")?;
    output
        .write_all(b"\n")
        .and_then(|_| output.flush())
        .map_err(|_| "native_import_pipe_failed".into())
}
struct NativeCredentialSink<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
    request_id: &'a str,
}
impl<R: BufRead, W: Write> ImportCredentialSink for NativeCredentialSink<'_, R, W> {
    fn store(
        &mut self,
        _: &NolaStore,
        connection: &ModelConnection,
        account_id: &str,
        secret: &str,
    ) -> Result<String, String> {
        let nonce = uuid::Uuid::new_v4().to_string();
        // Only the native service sees this frame and passes it directly to its
        // injected Keychain sink. Views receive only plan/commit frames.
        write_frame(
            self.output,
            json!({"type":"credential_request","request_id":self.request_id,"nonce":nonce,"connection_id":connection.id,"account_id":account_id,
            "endpoint":connection.base_url,"protocols":connection.protocols,"secret":secret}),
        )?;
        let reply = read_frame(self.input)?.ok_or("native_credential_sink_closed")?;
        if reply["type"] != "credential_saved"
            || reply["nonce"] != nonce
            || reply["request_id"] != self.request_id
        {
            return Err("native_credential_sink_rejected".into());
        }
        let reference = reply["reference"]
            .as_str()
            .filter(|value| value.starts_with("account:") && value.len() <= 256)
            .ok_or("native_credential_reference_invalid")?;
        Ok(reference.into())
    }
}
fn serve(
    store: &NolaStore,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    while let Some(value) = read_frame(input)? {
        let request: Request =
            serde_json::from_value(value).map_err(|_| "native_import_request_invalid")?;
        let request_id = match &request {
            Request::Plan { request_id, .. }
            | Request::Commit { request_id, .. }
            | Request::ProbePrepare { request_id, .. }
            | Request::Probe { request_id, .. } => request_id.clone(),
            Request::Close => break,
        };
        if uuid::Uuid::parse_str(&request_id).is_err() {
            return Err("native_import_request_id_invalid".into());
        }
        let result = match request {
            Request::Plan { request, .. } => plan_import(store, request, &[])
                .and_then(|plan| {
                    serde_json::to_value(plan).map_err(|_| "native_import_response_invalid".into())
                })
                .map(|plan| json!({"type":"plan","plan":plan})),
            Request::Commit { request, .. } => commit_import_with_sink(
                store,
                request,
                &[],
                &mut NativeCredentialSink {
                    input,
                    output,
                    request_id: &request_id,
                },
            )
            .map(|results| json!({"type":"commit","results":results})),
            Request::ProbePrepare { request, .. } => {
                crate::model_capability_probe::prepare_with_credentials(
                    store,
                    &request.connection_id,
                    request.expected_revision,
                    &request.model_id,
                    request.protocol,
                    &mut |connection, protocol| {
                        read_native_credential(
                            store,
                            input,
                            output,
                            &request_id,
                            connection,
                            protocol,
                        )
                    },
                )
                .map(|confirmation| json!({"type":"probe_prepare","confirmation":confirmation}))
            }
            Request::Probe { request, .. } => {
                crate::model_capability_probe::probe_with_credentials(
                    store,
                    request,
                    &[],
                    &mut |connection, protocol| {
                        read_native_credential(
                            store,
                            input,
                            output,
                            &request_id,
                            connection,
                            protocol,
                        )
                    },
                )
                .map(|snapshot| json!({"type":"probe","snapshot":snapshot}))
            }
            Request::Close => break,
        };
        let mut response = result.unwrap_or_else(|error| json!({"type":"error","error":error}));
        response["request_id"] = json!(request_id);
        write_frame(output, response)?;
    }
    Ok(())
}
pub(crate) fn run_if_requested() -> Option<Result<(), String>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) != Some("model-import-bridge") {
        return None;
    }
    Some((|| {
        if arguments.len() != 3
            || arguments[1] != "--database"
            || !std::path::Path::new(&arguments[2]).is_absolute()
        {
            return Err("native_import_database_required".into());
        }
        let store = NolaStore::open(std::path::Path::new(&arguments[2]))?;
        serve(
            &store,
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
        )
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn native_probe_uses_same_owner_and_private_credential_source() {
        use std::io::BufReader;
        use std::os::unix::net::UnixStream;
        let (_directory, store, server, _) =
            crate::model_capability_probe::tests::setup(ModelProtocol::Responses);
        let path = store.path().to_owned();
        let (mut client, mut child) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            let mut reader = BufReader::new(child.try_clone().unwrap());
            serve(&NolaStore::open(&path).unwrap(), &mut reader, &mut child).unwrap();
        });
        let upstream = std::thread::spawn(move || {
            let request = server
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            assert_eq!(request.url(), "/v1/responses");
            request
                .respond(tiny_http::Response::from_string(
                    json!({"output":[{"content":[{"type":"output_text","text":"OK"}]}]})
                        .to_string(),
                ))
                .unwrap();
        });
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut exchange = |kind: &str, request: Value| {
            let request_id = uuid::Uuid::new_v4().to_string();
            write_frame(
                &mut client,
                json!({"type":kind,"request_id":request_id,"request":request}),
            )
            .unwrap();
            loop {
                let frame = read_frame(&mut reader).unwrap().unwrap();
                assert_eq!(frame["request_id"], request_id);
                if frame["type"] == "credential_read_request" {
                    assert_eq!(frame["connection_id"], "fixture");
                    assert_eq!(frame["protocol"], "responses");
                    assert!(frame["account_id"].is_null());
                    write_frame(
                        &mut client,
                        json!({"type":"credential_read","request_id":request_id,
                        "nonce":frame["nonce"],"secret":""}),
                    )
                    .unwrap();
                } else {
                    break frame;
                }
            }
        };
        let prepared = exchange(
            "probe_prepare",
            json!({"connection_id":"fixture","expected_revision":2,
            "model_id":"fixture-model","protocol":"responses"}),
        );
        assert_eq!(prepared["type"], "probe_prepare");
        let mut confirmation = prepared["confirmation"].clone();
        assert_eq!(confirmation["acknowledge_possible_charges"], false);
        confirmation["acknowledge_possible_charges"] = json!(true);
        let completed = exchange("probe", confirmation);
        assert_eq!(completed["type"], "probe");
        assert_eq!(
            completed["snapshot"]["document"]["capability_assessments"][0]["evidence"]
                ["text_generation"],
            true
        );
        drop(exchange);
        write_frame(&mut client, json!({"type":"close"})).unwrap();
        worker.join().unwrap();
        upstream.join().unwrap();
        assert!(store.model_profiles().unwrap().is_empty());
    }

    #[test]
    fn native_credential_source_rejects_wrong_request_and_nonce() {
        let (_directory, store, _server, _) =
            crate::model_capability_probe::tests::setup(ModelProtocol::Responses);
        let snapshot: nola_domain::model_connection::ModelConnectionSnapshot = store
            .application()
            .unwrap()
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::GetConnection {
                    id: "fixture".into(),
                },
            })
            .unwrap();
        let mut input = std::io::Cursor::new(b"{\"type\":\"credential_read\",\"request_id\":\"wrong\",\"nonce\":\"wrong\",\"secret\":\"fixture\"}\n".to_vec());
        let mut output = vec![];
        assert!(read_native_credential(
            &store,
            &mut input,
            &mut output,
            "approved",
            &snapshot.document.connection,
            ModelProtocol::Responses
        )
        .is_err());
        assert!(!String::from_utf8(output).unwrap().contains("\"secret\""));
    }
    #[test]
    fn credential_sink_rejects_unbound_reply_without_persisting_account() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let connection: ModelConnection = serde_json::from_value(json!({
            "id":"fixture", "display_name":"Fixture", "base_url":"https://fixture.invalid/v1",
            "protocols":["responses"], "account_id":null, "source":"codex",
            "source_id":"fixture", "enabled":true
        }))
        .unwrap();
        let mut input = std::io::Cursor::new(b"{\"type\":\"credential_saved\",\"request_id\":\"wrong\",\"nonce\":\"wrong\",\"reference\":\"account:wrong\"}\n".to_vec());
        let mut output = vec![];
        let result = NativeCredentialSink {
            input: &mut input,
            output: &mut output,
            request_id: "approved-request",
        }
        .store(&store, &connection, "approved-account", "fixture-secret");
        assert_eq!(result.unwrap_err(), "native_credential_sink_rejected");
        let frame: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(frame["connection_id"], "fixture");
        assert_eq!(frame["account_id"], "approved-account");
        assert_eq!(frame["endpoint"], "https://fixture.invalid/v1");
        assert_eq!(frame["request_id"], "approved-request");
        assert!(store.model_profiles().unwrap().is_empty());
    }

    #[test]
    fn native_preview_uses_same_source_parser_and_never_emits_credentials() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.jsonc");
        std::fs::write(&path,br#"{"provider":{"fixture":{"npm":"@ai-sdk/openai","options":{"baseURL":"https://fixture.invalid/v1","apiKey":"private-fixture"},"models":{"model":{}}}}}"#).unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let request = json!({"type":"plan","request_id":uuid::Uuid::new_v4().to_string(),"request":{"source":"open_code","path":path,"credential_path":null,"include_credentials":true,"allow_environment":false}});
        let mut input =
            std::io::Cursor::new(format!("{request}\n{{\"type\":\"close\"}}\n").into_bytes());
        let mut output = vec![];
        serve(&store, &mut input, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"type\":\"plan\""));
        assert!(!output.contains("private-fixture"));
        assert!(store.model_profiles().unwrap().is_empty());
    }
}
