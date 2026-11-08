use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Inspect the model-facing protocol, not internal ToolOutcome.text. This
/// crosses CLI -> Host -> Core context -> provider and persistent Parts.
#[test]
fn cli_reads_saved_output_without_recursive_truncation_or_losing_the_cursor() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("evidence.txt");
    std::fs::write(&source, "中\"\\\u{0001}😀\n".repeat(1600)).unwrap();
    let database = workspace.path().join("fixture.db");
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", server.server_addr());
    let stop = Arc::new(AtomicBool::new(false));
    let received_pages = Arc::new(Mutex::new(Vec::<Value>::new()));
    let worker_stop = stop.clone();
    let worker_pages = received_pages.clone();
    let worker = std::thread::spawn(move || {
        let mut sequence = 0;
        while !worker_stop.load(Ordering::SeqCst) {
            let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                continue;
            };
            if request.url().contains("/models") {
                request
                    .respond(tiny_http::Response::from_string(
                        r#"{"data":[{"id":"fixture-model","context_window":128000}]}"#,
                    ))
                    .unwrap();
                continue;
            }
            sequence += 1;
            assert!(sequence < 30, "output navigation did not converge");
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let body: Value = serde_json::from_str(&body).unwrap();
            let messages = body["messages"].as_array().unwrap();
            let audit_request = messages
                .iter()
                .filter(|message| message["role"] == "system")
                .filter_map(|message| message["content"].as_str())
                .any(|text| text.contains("This is an independent constraint-audit boundary."));
            let latest = messages
                .iter()
                .rev()
                .find(|message| message["role"] == "tool")
                .and_then(|message| message["content"].as_str())
                .and_then(|text| serde_json::from_str::<Value>(text).ok());
            let call = if let Some(output) = latest {
                if output["type"] == "tool_output_slice" {
                    assert!(
                        output.get("truncated").is_none(),
                        "Core must retain the typed page: {output}"
                    );
                    assert_eq!(
                        output["bytes_returned"].as_u64().unwrap() as usize,
                        output["content"].as_str().unwrap().len()
                    );
                    let mut pages = worker_pages.lock().unwrap();
                    if pages
                        .last()
                        .is_none_or(|prior| prior["offset"] != output["offset"])
                    {
                        pages.push(output.clone());
                    }
                    output["has_more"].as_bool().unwrap().then(|| ("tool_output_read", json!({
                        "path": output["path"], "offset": output["next_offset"], "max_bytes": 12000
                    })))
                } else if output["error"] == "saved_result_requires_output_reader" {
                    assert_eq!(output["tool_execution_skipped"], true);
                    Some(("tool_output_read", output["recovery"]["arguments"].clone()))
                } else {
                    let reference = output["full_output_ref"]
                        .as_str()
                        .unwrap_or_else(|| panic!("Missing saved output reference: {output}"));
                    // Reproduce the Mac failure's wrong-reader call. Recovery
                    // must not turn a saved reference into a workspace grant.
                    Some(("read_file", json!({"path": reference})))
                }
            } else {
                Some((
                    // Native file reads deliberately page the original file
                    // and grant only returned line coverage. Generate an actual
                    // oversized process result for this saved-output contract.
                    "bash",
                    json!({"command": "cat evidence.txt", "operation_kind": "inspect"}),
                ))
            };
            let chunk = if audit_request {
                json!({"choices":[{"delta":{"content": json!({
                    "decision":"accept", "outcome":"answered", "constraints":["complete saved output read"],
                    "items":[{"label":"OUTPUT_ROUNDTRIP_OK", "eligibility":"eligible", "evidence":"All saved-result pages were consumed with contiguous offsets"}],
                    "violations":[]
                }).to_string()},"finish_reason":"stop"}]})
            } else if let Some((name, args)) = call {
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":format!("call-{sequence}"),
                    "type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]})
            } else {
                json!({"choices":[{"delta":{"content":"OUTPUT_ROUNDTRIP_OK"},"finish_reason":"stop"}]})
            };
            let response =
                tiny_http::Response::from_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    );
            request.respond(response).unwrap();
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_nola"))
        .args([
            "Run cat evidence.txt and inspect the complete saved command output. Reply OUTPUT_ROUNDTRIP_OK.",
            "--endpoint",
            &endpoint,
            "--api-key",
            "fixture",
            "--model",
            "fixture-model",
            "--project",
            workspace.path().to_str().unwrap(),
            "--session",
            "output-roundtrip",
            "--max-iterations",
            "25",
            "--json",
        ])
        .env("NOLA_DB_PATH", &database)
        .env("NOLA_HOME", workspace.path().join("home"))
        .env("NOLA_LONG_TERM_MEMORY_ENABLED", "0")
        .env("NOLA_PERMISSION_PROFILE", "allow_all")
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .env("NOLA_CLI_FORCE_TOOL_SET", "all")
        .stdout(Stdio::from(
            std::fs::File::create(workspace.path().join("stdout.log")).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(workspace.path().join("stderr.log")).unwrap(),
        ))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    stop.store(true, Ordering::SeqCst);
    let worker_result = worker.join();
    if worker_result.is_err() || !status.is_some_and(|status| status.success()) {
        let evidence = workspace.keep();
        panic!(
            "CLI did not complete (status={status:?}, provider_panicked={}); diagnostics preserved at {}",
            worker_result.is_err(), evidence.display()
        );
    }
    let pages = received_pages.lock().unwrap();
    assert!(pages.len() > 1);
    let saved = pages[0]["path"].as_str().unwrap();
    assert!(
        std::path::Path::new(saved).starts_with(
            std::fs::canonicalize(workspace.path().join("fixture.db.outputs")).unwrap()
        ),
        "saved evidence must belong to the isolated database, not the global cache: {saved}"
    );
    let restored: String = pages
        .iter()
        .map(|page| page["content"].as_str().unwrap())
        .collect();
    assert_eq!(restored, std::fs::read_to_string(saved).unwrap());
    let saved_result: Value = serde_json::from_str(&restored).unwrap();
    assert_eq!(saved_result["type"], "shell_execution");
    assert_eq!(saved_result["exit_code"], 0);
    let original_stdout = std::fs::read_to_string(&source).unwrap();
    let restored_stdout = saved_result["stdout"].as_str().expect("typed shell stdout");
    // Exact comparison stays mandatory; avoid dumping the entire large payload
    // on failure and hiding the other test diagnostics behind megabytes of text.
    assert!(restored_stdout == original_stdout,
        "navigation must recover exact UTF-8/JSON process output: restored_bytes={}, original_bytes={}, first_difference={:?}",
        restored_stdout.len(), original_stdout.len(),
        restored_stdout.bytes().zip(original_stdout.bytes()).position(|(left,right)| left != right));
    assert_eq!(saved_result["output_compacted"], false,
        "the saved payload must be complete even when its model preview is compacted");
    for pair in pages.windows(2) {
        assert_eq!(pair[0]["next_offset"], pair[1]["offset"]);
    }
    let db = rusqlite::Connection::open(&database).unwrap();
    let nonterminal: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runs r LEFT JOIN run_results rr ON rr.run_id=r.id
             WHERE r.status != 'sealed' OR rr.run_id IS NULL
                OR json_extract(rr.result_json,'$.value.kind') != 'answered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(nonterminal, 0);
    let canonical = nola_store::NolaStore::open(&database).unwrap();
    let mut parts = db
        .prepare(
            "SELECT m.conversation_id,m.id,p.id FROM message_parts p
        JOIN messages m ON m.id=p.message_id WHERE m.role='assistant' AND p.kind='text'
        ORDER BY m.ordinal,p.ordinal",
        )
        .unwrap();
    let identities = parts
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let visible_answer = identities
        .iter()
        .map(|(conversation, message, part)| {
            canonical
                .read_conversation_text_range(conversation, message, part, 0, 32768)
                .unwrap()
                .text
        })
        .collect::<String>();
    assert_eq!(
        visible_answer, "OUTPUT_ROUNDTRIP_OK",
        "private evidence-audit JSON and the candidate must not leak into visible assistant Parts"
    );
}
