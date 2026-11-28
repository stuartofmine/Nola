// SPDX-License-Identifier: Apache-2.0
//! Full-stack real-subprocess CLI & real-PTY TUI end-to-end tests for the compiled `nola` binary.
//!
//! Every test runs against isolated temporary directories (`HOME`, `NOLA_HOME`, `NOLA_DB_PATH`,
//! `--db`, `--project`) and NEVER touches `~/.nola` or `~/Library/Application Support/Nola`.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

const NOLA_BIN: &str = env!("CARGO_BIN_EXE_nola");

// ============================================================================
// Isolated Subprocess Harness
// ============================================================================

struct IsolatedEnv {
    _root: TempDir,
    home: PathBuf,
    workspace: PathBuf,
    db_path: PathBuf,
}

impl IsolatedEnv {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create tempdir");
        let home = root.path().join("home");
        let workspace = root.path().join("workspace");
        let db_path = root.path().join("state").join("nola-test.db");
        fs::create_dir_all(&home).expect("create home");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(db_path.parent().unwrap()).expect("create state dir");
        Self {
            _root: root,
            home,
            workspace,
            db_path,
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(NOLA_BIN);
        c.current_dir(&self.workspace)
            .env("HOME", &self.home)
            .env("NOLA_HOME", &self.home)
            .env("NOLA_DB_PATH", &self.db_path)
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8")
            .env_remove("NOLA_MAC_APP_SUPPORT")
            .env_remove("NOLA_VAULT_PATH")
            .env_remove("NOLA_LLM_ENDPOINT")
            .env_remove("NOLA_LLM_MODEL")
            .env_remove("NOLA_LLM_PROTOCOL")
            .env_remove("NOLA_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("NOLA_PERMISSION_PROFILE")
            .env_remove("NOLA_BENCHMARK_TOKEN")
            .env_remove("NOLA_LANG")
            .stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut c = self.cmd();
        c.arg("--db")
            .arg(&self.db_path)
            .arg("--project")
            .arg(&self.workspace)
            .args(args);
        c.output().expect("spawn nola subprocess")
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "nola {:?} failed (status={:?})\nstdout:\n{}\nstderr:\n{}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn run_json(&self, args: &[&str]) -> Value {
        let stdout = self.run_ok(args);
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
            panic!("failed to parse JSON from `nola {:?}`: {e}\nraw stdout:\n{stdout}", args)
        })
    }

    fn write_and_apply_mock_config(&self, endpoint_v1: &str) -> PathBuf {
        let cfg_path = self.workspace.join("nola-e2e-config.jsonc");
        let cfg_text = format!(
            r#"{{
  // E2E mock model configuration
  "llm": {{
    "default_model": {{
      "connection": "mock_local",
      "model": "mock-reasoner-7b",
    }},
    "connections": [
      {{
        "id": "mock_local",
        "endpoint": "{endpoint_v1}",
        "protocol": "chat_completions",
        "api_key": "sk-local-test-key",
        "models": [
          {{
            "id": "mock-reasoner-7b",
            "context_window": 131072,
            "max_output_tokens": 4096,
            "capabilities": {{
              "tool_calls": true,
              "reasoning": true,
              "sampling_parameters": true
            }}
          }}
        ]
      }}
    ]
  }}
}}"#
        );
        fs::write(&cfg_path, cfg_text).expect("write mock config");
        let applied = self.run_json(&["config", "apply", cfg_path.to_str().unwrap()]);
        assert_eq!(applied["ok"], true);
        assert_eq!(applied["applied"], true);
        cfg_path
    }
}

// ============================================================================
// Local Mock OpenAI + llama.cpp `/v1/models` + `/props` + SSE Server
// ============================================================================

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    url: String,
    body: Option<Value>,
}

#[derive(Debug, Clone)]
enum MockSseTurn {
    Text {
        reasoning: Option<String>,
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
    },
    HttpError {
        status: u16,
        body: String,
    },
}

struct MockLlmServer {
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    turns: Arc<Mutex<VecDeque<MockSseTurn>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockLlmServer {
    fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock http server");
        let addr = server.server_addr().to_ip().expect("tcp addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let captured = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
        let turns = Arc::new(Mutex::new(VecDeque::<MockSseTurn>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let cap_bg = Arc::clone(&captured);
        let turns_bg = Arc::clone(&turns);
        let shut_bg = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            while !shut_bg.load(Ordering::Relaxed) {
                let Ok(Some(mut req)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                let method = req.method().as_str().to_string();
                let url = req.url().to_string();
                let mut raw_body = String::new();
                let _ = req.as_reader().read_to_string(&mut raw_body);
                let body_json = if raw_body.trim().is_empty() {
                    None
                } else {
                    serde_json::from_str::<Value>(&raw_body).ok()
                };
                cap_bg.lock().unwrap().push(CapturedRequest {
                    method: method.clone(),
                    url: url.clone(),
                    body: body_json,
                });

                match (method.as_str(), url.as_str()) {
                    ("GET", "/v1/models") => {
                        let payload = json!({
                            "object": "list",
                            "data": [
                                {
                                    "id": "mock-reasoner-7b",
                                    "object": "model",
                                    "display_name": "Mock Reasoner 7B",
                                    "context_length": 131072
                                },
                                {
                                    "id": "mock-coder-14b",
                                    "object": "model",
                                    "display_name": "Mock Coder 14B",
                                    "context_length": 262144
                                }
                            ]
                        });
                        let resp = tiny_http::Response::from_string(payload.to_string())
                            .with_status_code(200)
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    &b"Content-Type"[..],
                                    &b"application/json"[..],
                                )
                                .unwrap(),
                            );
                        let _ = req.respond(resp);
                    }
                    ("GET", "/props") => {
                        let payload = json!({
                            "model_alias": "mock-reasoner-7b",
                            "default_generation_settings": {
                                "n_ctx": 131072
                            },
                            "chat_template_caps": {
                                "supports_tools": true,
                                "supports_reasoning": true
                            }
                        });
                        let resp = tiny_http::Response::from_string(payload.to_string())
                            .with_status_code(200)
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    &b"Content-Type"[..],
                                    &b"application/json"[..],
                                )
                                .unwrap(),
                            );
                        let _ = req.respond(resp);
                    }
                    ("POST", "/v1/chat/completions") => {
                        let next_turn = turns_bg.lock().unwrap().pop_front().unwrap_or(
                            MockSseTurn::Text {
                                reasoning: None,
                                text: "Default mock reply.".into(),
                            },
                        );
                        if let MockSseTurn::HttpError { status, body } = next_turn {
                            let resp = tiny_http::Response::from_string(body)
                                .with_status_code(status)
                                .with_header(
                                    tiny_http::Header::from_bytes(
                                        &b"Content-Type"[..],
                                        &b"application/json"[..],
                                    )
                                    .unwrap(),
                                );
                            let _ = req.respond(resp);
                        } else {
                            let sse = render_openai_sse_turn(&next_turn);
                            let resp = tiny_http::Response::from_string(sse)
                                .with_status_code(200)
                                .with_header(
                                    tiny_http::Header::from_bytes(
                                        &b"Content-Type"[..],
                                        &b"text/event-stream"[..],
                                    )
                                    .unwrap(),
                                );
                            let _ = req.respond(resp);
                        }
                    }
                    _ => {
                        let resp = tiny_http::Response::from_string(
                            json!({"error": "not_found", "path": url}).to_string(),
                        )
                        .with_status_code(404);
                        let _ = req.respond(resp);
                    }
                }
            }
        });

        Self {
            base_url,
            captured,
            turns,
            shutdown,
            handle: Some(handle),
        }
    }

    fn v1_endpoint(&self) -> String {
        format!("{}/v1", self.base_url)
    }

    fn push_turn(&self, turn: MockSseTurn) {
        self.turns.lock().unwrap().push_back(turn);
    }

    fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.captured.lock().unwrap().clone()
    }

    fn chat_requests(&self) -> Vec<CapturedRequest> {
        self.captured
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" && r.url == "/v1/chat/completions")
            .cloned()
            .collect()
    }
}

impl Drop for MockLlmServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn render_openai_sse_turn(turn: &MockSseTurn) -> String {
    let mut out = String::new();
    match turn {
        MockSseTurn::Text { reasoning, text } => {
            if let Some(r) = reasoning {
                let chunk = json!({
                    "id": "chatcmpl-mock-1",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "reasoning_content": r },
                        "finish_reason": Value::Null
                    }]
                });
                out.push_str(&format!("data: {chunk}\n\n"));
            }
            let chunk = json!({
                "id": "chatcmpl-mock-1",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": text },
                    "finish_reason": Value::Null
                }]
            });
            out.push_str(&format!("data: {chunk}\n\n"));
            let end_chunk = json!({
                "id": "chatcmpl-mock-1",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 120,
                    "completion_tokens": 24,
                    "total_tokens": 144
                }
            });
            out.push_str(&format!("data: {end_chunk}\n\n"));
        }
        MockSseTurn::ToolCall {
            call_id,
            name,
            arguments,
        } => {
            let args_str = serde_json::to_string(arguments).unwrap();
            let chunk = json!({
                "id": "chatcmpl-mock-tc",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "tool_calls": [{
                            "index": 0,
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args_str
                            }
                        }]
                    },
                    "finish_reason": Value::Null
                }]
            });
            out.push_str(&format!("data: {chunk}\n\n"));
            let end_chunk = json!({
                "id": "chatcmpl-mock-tc",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 140,
                    "completion_tokens": 30,
                    "total_tokens": 170
                }
            });
            out.push_str(&format!("data: {end_chunk}\n\n"));
        }
        MockSseTurn::HttpError { .. } => {}
    }
    out.push_str("data: [DONE]\n\n");
    out
}

// ============================================================================
// 1. CLI Version, Config, and Model Discovery E2E
// ============================================================================

#[test]
fn cli_version_config_and_model_discovery_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();

    // `nola version` (plain text) & `nola version --json` (DB-free)
    let ver_text = env.run_ok(&["version"]);
    assert!(
        ver_text.trim().starts_with("nola "),
        "unexpected `nola version` output: {ver_text}"
    );
    let ver_json = env.run_json(&["version", "--json"]);
    assert_eq!(ver_json["name"], "nola");
    assert_eq!(ver_json["version"], env!("CARGO_PKG_VERSION"));
    assert!(ver_json["protocol_version"].as_u64().unwrap_or(0) >= 1);

    let unconfigured = env.run(&["run", "--no-tools", "Hello"]);
    assert!(!unconfigured.status.success());
    assert!(String::from_utf8_lossy(&unconfigured.stderr).contains("nola config edit"));

    // `nola config example` -> valid JSONC template
    let example = env.run_ok(&["config", "example"]);
    assert!(
        example.contains("default_model") && example.contains("connections"),
        "config example missing expected keys: {example}"
    );

    // Write example-derived JSONC config pointing to our mock HTTP server and validate with `nola config check`
    let cfg_path = env.workspace.join("config.jsonc");
    let cfg_body = format!(
        r#"{{
  // JSONC comment should be stripped cleanly
  "llm": {{
    "default_model": {{
      "connection": "local_mock",
      "model": "mock-reasoner-7b",
    }},
    "connections": [
      {{
        "id": "local_mock",
        "endpoint": "{}/v1",
        "protocol": "chat_completions",
        "api_key": "sk-secret-value-should-redact",
        "models": [
          {{ "id": "mock-reasoner-7b", "context_window": 131072 }}
        ],
      }},
    ],
  }},
}}"#,
        mock.base_url
    );
    fs::write(&cfg_path, cfg_body).expect("write config.jsonc");

    let check_res = env.run_json(&["config", "check", cfg_path.to_str().unwrap()]);
    assert_eq!(check_res["ok"], true);
    assert_eq!(check_res["valid"], true);
    assert_eq!(check_res["connections"], 1);

    // Dry-run and real apply
    let dry_res = env.run_json(&["config", "apply", "--dry-run", cfg_path.to_str().unwrap()]);
    assert_eq!(dry_res["ok"], true);
    assert_eq!(dry_res["dry_run"], true);
    assert_eq!(dry_res["applied"], false);

    let apply_res = env.run_json(&["config", "apply", cfg_path.to_str().unwrap()]);
    assert_eq!(apply_res["ok"], true);
    assert_eq!(apply_res["applied"], true);
    assert_eq!(apply_res["connection"], "local_mock");
    assert_eq!(apply_res["model"], "mock-reasoner-7b");

    // Separate subprocess invocations verify SQLite persistence: `nola model connections` & `nola model list`
    let conns = env.run_json(&["model", "connections"]);
    let conn_arr = conns["connections"].as_array().expect("connections array");
    assert_eq!(conn_arr.len(), 1);
    assert_eq!(conn_arr[0]["id"], "local_mock");
    assert!(
        !conns.to_string().contains("sk-secret-value-should-redact"),
        "model connections leaked raw API key!"
    );

    let listed = env.run_json(&["model", "list"]);
    assert_eq!(listed["default_model"]["connection"], "local_mock");
    assert_eq!(listed["default_model"]["model"], "mock-reasoner-7b");

    // Verify mock `/props` is healthy (llama.cpp capability probe endpoint)
    let props_resp: Value = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{}/props", mock.base_url))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(props_resp["default_generation_settings"]["n_ctx"], 131072);

    // `nola model discover` against mock `/v1/models` + `--save` persistence across invocations
    let disc = env.run_json(&["model", "discover", "local_mock"]);
    assert_eq!(disc["ok"], true);
    assert_eq!(disc["saved"], false);
    assert_eq!(disc["count"], 2);
    let ids: Vec<&str> = disc["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(ids.contains(&"mock-reasoner-7b"));
    assert!(ids.contains(&"mock-coder-14b"));

    let disc_save = env.run_json(&["model", "discover", "local_mock", "--save"]);
    assert_eq!(disc_save["ok"], true);
    assert_eq!(disc_save["saved"], true);

    // Fresh invocation of `nola model list` must show `mock-coder-14b` persisted in SQLite!
    let listed_after = env.run_json(&["model", "list"]);
    let persisted_ids: Vec<&str> = listed_after["connections"][0]["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(
        persisted_ids.contains(&"mock-coder-14b"),
        "discovered model did not persist to SQLite: {persisted_ids:?}"
    );
    assert!(mock
        .captured_requests()
        .iter()
        .any(|r| r.method == "GET" && r.url == "/v1/models"));
}

// ============================================================================
// 2. CLI Memory, Skills, MCP, Goal, Room, Scheduler Persistence E2E
// ============================================================================

#[test]
fn cli_memory_skills_mcp_goal_room_scheduler_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();

    // --- Memory CRUD across separate subprocess invocations ---
    let created = env.run_json(&[
        "memory",
        "create",
        "--id",
        "mem-e2e-1",
        "--title",
        "Storage Baseline",
        "--content",
        "Nola v2 uses SQLite WAL user_version=2",
        "--kind",
        "decision",
        "--scope",
        "workspace",
        "--tags",
        "sqlite,architecture",
    ]);
    assert_eq!(created["record"]["id"], "mem-e2e-1");

    let list1 = env.run_json(&["memory", "list"]);
    let recs1 = list1["records"].as_array().expect("memory records array");
    assert_eq!(recs1.len(), 1);
    assert_eq!(recs1[0]["id"], "mem-e2e-1");
    assert!(recs1[0]["content"]
        .as_str()
        .unwrap_or("")
        .contains("user_version=2"));

    let revised = env.run_json(&[
        "memory",
        "revise",
        "mem-e2e-1",
        "--title",
        "Storage Baseline v2",
        "--content",
        "Nola v2 uses SQLite WAL user_version=2 + FTS5",
    ]);
    assert_eq!(revised["record"]["id"], "mem-e2e-1");

    let list2 = env.run_json(&["memory", "list"]);
    let recs2 = list2["records"].as_array().expect("memory records array");
    assert_eq!(recs2.len(), 1);
    assert!(recs2[0]["content"]
        .as_str()
        .unwrap_or("")
        .contains("FTS5"));

    let deleted = env.run_json(&["memory", "delete", "mem-e2e-1"]);
    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["deleted"], "mem-e2e-1");

    let list3 = env.run_json(&["memory", "list"]);
    assert_eq!(list3["records"].as_array().unwrap().len(), 0);

    // --- Skills discovery ---
    let skill_dir = env.workspace.join(".nola/skills/e2e-audit-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: e2e-audit-skill\ndescription: Temporary skill for CLI E2E\n---\nAlways check SQLite WAL.\n",
    )
    .unwrap();
    let skills = env.run_json(&["skills", "list"]);
    let skill_arr = skills["skills"].as_array().expect("skills array");
    assert!(
        skill_arr
            .iter()
            .any(|s| s["name"].as_str() == Some("e2e-audit-skill")),
        "expected workspace skill in `nola skills list`: {skills}"
    );

    // --- MCP add (stdio & http), list (with secret redaction), remove ---
    let mcp_stdio = env.run_json(&[
        "mcp",
        "add",
        "stdio-demo",
        "--transport",
        "stdio",
        "--command",
        "/bin/echo",
        "--arg",
        "hello-mcp",
        "--env",
        "API_TOKEN=super-secret-token",
    ]);
    assert_eq!(mcp_stdio["ok"], true);
    assert_eq!(mcp_stdio["server"]["id"], "stdio-demo");
    assert!(!mcp_stdio.to_string().contains("super-secret-token"));

    let mcp_http = env.run_json(&[
        "mcp",
        "add",
        "http-demo",
        "--transport",
        "http",
        "--url",
        "http://127.0.0.1:18088/mcp",
    ]);
    assert_eq!(mcp_http["ok"], true);
    assert_eq!(mcp_http["server"]["id"], "http-demo");

    let mcp_list = env.run_json(&["mcp", "list"]);
    let servers = mcp_list.as_array().expect("mcp list array");
    assert_eq!(servers.len(), 2);
    assert!(!mcp_list.to_string().contains("super-secret-token"));
    assert!(servers.iter().any(|s| s["id"] == "stdio-demo"));
    assert!(servers.iter().any(|s| s["id"] == "http-demo"));

    let rm1 = env.run_json(&["mcp", "remove", "stdio-demo"]);
    assert_eq!(rm1["ok"], true);
    assert_eq!(rm1["deleted"], true);
    let rm2 = env.run_json(&["mcp", "remove", "http-demo"]);
    assert_eq!(rm2["ok"], true);
    assert_eq!(rm2["deleted"], true);

    let mcp_after = env.run_json(&["mcp", "list"]);
    assert_eq!(mcp_after.as_array().unwrap().len(), 0);

    // --- Goal create, list, status, cancel ---
    let goal = env.run_json(&[
        "goal",
        "create",
        "Ship CLI E2E Coverage",
        "--id",
        "goal-e2e-1",
        "--objective",
        "Exercise all CLI subcommands against real SQLite",
        "--criteria",
        "All assertions pass",
    ]);
    assert_eq!(goal["id"], "goal-e2e-1");
    assert_eq!(goal["status"], "pending");

    let goals = env.run_json(&["goal", "list"]);
    assert!(goals
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["id"] == "goal-e2e-1"));

    let status1 = env.run_json(&["goal", "status", "goal-e2e-1"]);
    assert_eq!(status1["status"], "pending");

    let cancelled = env.run_json(&[
        "goal",
        "cancel",
        "goal-e2e-1",
        "--note",
        "Verified via E2E",
    ]);
    assert_eq!(cancelled["status"], "failed");

    let status2 = env.run_json(&["goal", "status", "goal-e2e-1"]);
    assert_eq!(status2["status"], "failed");
    assert!(status2["progress_notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap_or("").contains("Verified via E2E")));

    // --- Room list & Scheduler list / tick ---
    let rooms = env.run_json(&["room", "list"]);
    assert!(rooms.is_array());

    let sched_list = env.run_json(&["scheduler", "list"]);
    assert!(sched_list.is_array());

    env.write_and_apply_mock_config(&mock.v1_endpoint());
    let tick_res = env.run_json(&["scheduler", "tick"]);
    assert_eq!(tick_res, json!([]));
}

// ============================================================================
// 3. CLI Wiki Indexing on Real Multi-Language Repo (Rust + Swift + TypeScript)
// ============================================================================

#[test]
fn cli_wiki_indexes_rust_swift_and_typescript_repo_e2e() {
    let env = IsolatedEnv::new();
    let repo = env.workspace.join("sample_polyglot_repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("ios")).unwrap();
    fs::create_dir_all(repo.join("web")).unwrap();

    fs::write(
        repo.join("src/scoring.rs"),
        r#"pub struct ScoreCard {
    pub points: i32,
}

pub fn compute_score(base: i32) -> ScoreCard {
    ScoreCard { points: base * 2 }
}
"#,
    )
    .unwrap();

    fs::write(
        repo.join("src/engine.rs"),
        r#"use crate::scoring::{compute_score, ScoreCard};

pub struct ExecutionEngine;

impl ExecutionEngine {
    pub fn evaluate(&self, raw: i32) -> ScoreCard {
        compute_score(raw)
    }
}
"#,
    )
    .unwrap();

    fs::write(
        repo.join("ios/ChatClient.swift"),
        r#"import Foundation

public struct ChatMessage {
    public let id: String
    public let body: String
}

public final class ChatClient {
    public func formatMessage(_ msg: ChatMessage) -> String {
        return "\(msg.id): \(msg.body)"
    }
}
"#,
    )
    .unwrap();

    fs::write(
        repo.join("web/session.ts"),
        r#"export interface SessionSummary {
    id: string;
    title: string;
}

export function formatSessionTitle(summary: SessionSummary): string {
    return `${summary.id} - ${summary.title}`;
}

export class SessionStore {
    describe(item: SessionSummary): string {
        return formatSessionTitle(item);
    }
}
"#,
    )
    .unwrap();

    let arch = env.run_json(&["wiki", repo.to_str().unwrap(), "--json"]);
    let total_files = arch["total_files"].as_u64().unwrap_or(0);
    let total_symbols = arch["total_symbols"].as_u64().unwrap_or(0);
    let total_edges = arch["total_edges"].as_u64().unwrap_or(0);
    assert!(
        total_files >= 4,
        "expected >= 4 indexed files across Rust/Swift/TS, got {arch}"
    );
    assert!(
        total_symbols >= 6,
        "expected >= 6 indexed symbols, got {arch}"
    );
    assert!(
        total_edges > 0,
        "expected > 0 indexed cross-symbol/file edges, got {arch}"
    );

    let modules = arch["modules"].as_array().expect("modules array");
    let mod_names: Vec<&str> = modules
        .iter()
        .filter_map(|m| m["module"].as_str())
        .collect();
    assert!(mod_names.contains(&"src"), "missing `src` module: {mod_names:?}");
    assert!(mod_names.contains(&"ios"), "missing `ios` module: {mod_names:?}");
    assert!(mod_names.contains(&"web"), "missing `web` module: {mod_names:?}");

    let all_langs: Vec<String> = modules
        .iter()
        .flat_map(|m| {
            m["languages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(|s| s.to_ascii_lowercase()))
        })
        .collect();
    assert!(
        all_langs.iter().any(|l| l.contains("rust")),
        "missing Rust language: {all_langs:?}"
    );
    assert!(
        all_langs.iter().any(|l| l.contains("swift")),
        "missing Swift language: {all_langs:?}"
    );
    assert!(
        all_langs.iter().any(|l| l.contains("typescript")),
        "missing TypeScript language: {all_langs:?}"
    );

    // Markdown + `--output` file export
    let out_md = env.workspace.join("wiki-out.md");
    let stdout_md = env.run_ok(&[
        "wiki",
        repo.to_str().unwrap(),
        "--output",
        out_md.to_str().unwrap(),
    ]);
    assert!(stdout_md.contains("# Codebase Architecture Wiki"));
    let written = fs::read_to_string(&out_md).expect("read written wiki markdown");
    assert!(written.contains("Indexed Files"));
    assert!(written.contains("`src`"));
    assert!(written.contains("`ios`"));
    assert!(written.contains("`web`"));
}

// ============================================================================
// 4. CLI `nola run --json` SSE Tool Execution, Reasoning Effort, Profiles,
//    and `nola sessions` Lifecycle E2E
// ============================================================================

#[test]
fn cli_run_json_tools_reasoning_profiles_and_sessions_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();
    env.write_and_apply_mock_config(&mock.v1_endpoint());

    fs::write(
        env.workspace.join("notes.txt"),
        "alpha-beta-gamma-workspace-content\n",
    )
    .unwrap();

    // --- 4a. `nola run --json --reasoning-effort none --profile cautious` with read_file tool call ---
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_read_1".into(),
        name: "read_file".into(),
        arguments: json!({"path": "notes.txt"}),
    });
    mock.push_turn(MockSseTurn::Text {
        reasoning: None,
        text: "Read notes.txt successfully.".into(),
    });

    let run1_out = env.run_ok(&[
        "run",
        "--json",
        "--reasoning-effort",
        "none",
        "--profile",
        "cautious",
        "Read notes.txt and confirm",
    ]);
    let run1_events: Vec<Value> = run1_out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad NDJSON line `{l}`: {e}")))
        .collect();

    let sid1 = run1_events
        .iter()
        .find_map(|ev| ev.get("session_id").and_then(|v| v.as_str()))
        .expect("session_id in run1 events")
        .to_string();
    assert!(
        run1_events.iter().any(|ev| {
            ev["type"] == "tool_finished"
                && ev["name"] == "read_file"
                && ev["is_error"] == false
        }),
        "expected successful read_file tool_finished event: {run1_events:?}"
    );
    assert!(
        run1_events.iter().any(|ev| {
            ev["type"] == "text_delta"
                && ev["delta"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Read notes.txt successfully.")
        }),
        "expected final text_delta event: {run1_events:?}"
    );

    let chat_reqs_1 = mock.chat_requests();
    assert_eq!(chat_reqs_1.len(), 2, "expected 2 LLM calls (tool + final)");
    assert_eq!(
        chat_reqs_1[0].body.as_ref().unwrap()["reasoning_effort"],
        "none"
    );
    let second_msgs = chat_reqs_1[1].body.as_ref().unwrap()["messages"]
        .as_array()
        .unwrap();
    assert!(
        second_msgs.iter().any(|m| {
            m["role"] == "tool"
                && m["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("alpha-beta-gamma-workspace-content")
        }),
        "tool result from read_file was not sent back to LLM: {second_msgs:?}"
    );

    // --- 4b. `--reasoning-effort high --profile cautious` blocks mutating `write_file` ---
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_write_blocked".into(),
        name: "write_file".into(),
        arguments: json!({"path": "blocked.txt", "content": "should not be written"}),
    });
    mock.push_turn(MockSseTurn::Text {
        reasoning: Some("Permission denied as expected in cautious mode.".into()),
        text: "Blocked under cautious profile.".into(),
    });

    let run2_out = env.run_ok(&[
        "run",
        "--json",
        "--reasoning-effort",
        "high",
        "--profile",
        "cautious",
        "Try writing blocked.txt",
    ]);
    let run2_events: Vec<Value> = run2_out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        !env.workspace.join("blocked.txt").exists(),
        "cautious profile MUST NOT write blocked.txt without approval!"
    );
    assert!(
        run2_events.iter().any(|ev| {
            ev["type"] == "tool_finished"
                && ev["name"] == "write_file"
                && ev["is_error"] == true
        }),
        "expected write_file denial under cautious profile: {run2_events:?}"
    );

    let chat_reqs_2 = mock.chat_requests();
    assert_eq!(chat_reqs_2.len(), 4);
    assert_eq!(
        chat_reqs_2[2].body.as_ref().unwrap()["reasoning_effort"],
        "high"
    );

    // --- 4c. `--reasoning-effort high --profile allow_all` allows `write_file` ---
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_write_allowed".into(),
        name: "write_file".into(),
        arguments: json!({"path": "allowed.txt", "content": "written-under-allow-all"}),
    });
    mock.push_turn(MockSseTurn::Text {
        reasoning: Some("Executed write under allow_all.".into()),
        text: "Wrote allowed.txt.".into(),
    });

    let run3_out = env.run_ok(&[
        "run",
        "--json",
        "--reasoning-effort",
        "high",
        "--profile",
        "allow_all",
        "Write allowed.txt",
    ]);
    let run3_events: Vec<Value> = run3_out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        run3_events.iter().any(|ev| {
            ev["type"] == "tool_finished"
                && ev["name"] == "write_file"
                && ev["is_error"] == false
        }),
        "expected write_file to succeed under allow_all: {run3_events:?}"
    );
    assert_eq!(
        fs::read_to_string(env.workspace.join("allowed.txt")).unwrap(),
        "written-under-allow-all"
    );

    // --- 4d. Reject `--profile accept_edits` AND `--permission-profile accept_edits` ---
    let req_count_before_reject = mock.chat_requests().len();
    for flag in ["--profile", "--permission-profile"] {
        let out = env.run(&["run", "--json", flag, "accept_edits", "Should be rejected"]);
        assert!(
            !out.status.success(),
            "`nola run {flag} accept_edits` MUST fail with non-zero exit code!"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("accept_edits"),
            "expected error mentioning 'accept_edits' for `{flag}`, got: {stderr}"
        );
    }
    assert_eq!(
        mock.chat_requests().len(),
        req_count_before_reject,
        "rejected accept_edits runs must never reach the LLM endpoint"
    );

    // --- 4e. Sessions list, show, backup, delete ---
    let sessions_list = env.run_json(&["sessions", "list", "--json"]);
    let sess_arr = sessions_list["sessions"].as_array().expect("sessions array");
    assert!(
        sess_arr.len() >= 3,
        "expected at least 3 persisted sessions, got {sess_arr:?}"
    );
    assert!(sess_arr.iter().any(|s| s["id"] == sid1));

    let shown = env.run_json(&["sessions", "show", &sid1]);
    assert_eq!(shown["session"]["id"], sid1);
    assert!(shown["event_count"].as_u64().unwrap_or(0) > 0);
    assert!(!shown["messages"].as_array().unwrap().is_empty());

    mock.push_turn(MockSseTurn::Text {
        reasoning: None,
        text: "alpha-beta-gamma-workspace-content".into(),
    });
    env.run_ok(&["run", "--json", "--no-tools", "--session", &sid1, "Repeat the file content from our conversation"]);
    let resumed_requests = mock.chat_requests();
    let body = resumed_requests.last().unwrap().body.as_ref().unwrap();
    assert!(body["tools"].is_null());
    let resumed_messages = body["messages"].as_array().unwrap();
    assert!(resumed_messages.iter().any(|message| message["role"] == "system"
        && message["content"].as_str().is_some_and(|text| text.contains("Tools are disabled"))));
    assert!(resumed_messages.iter().any(|message| message["role"] == "assistant"
        && message["content"] == "Read notes.txt successfully."), "Explicit session ID must restore earlier turns");
    let reopened = env.run_json(&["sessions", "show", &sid1]);
    assert_eq!(reopened["session"]["title"], shown["session"]["title"]);
    let roles: Vec<_> = reopened["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant"]);

    let backup_db = env.workspace.join("sessions-backup.db");
    let bkp = env.run_json(&["sessions", "backup", backup_db.to_str().unwrap()]);
    assert_eq!(bkp["backup"], backup_db.to_str().unwrap());
    assert!(backup_db.is_file(), "backup SQLite file must exist");

    // Verify backup DB is readable and retains `sid1`
    let bkp_show_out = env
        .cmd()
        .args([
            "--db",
            backup_db.to_str().unwrap(),
            "sessions",
            "show",
            &sid1,
        ])
        .output()
        .expect("run sessions show against backup");
    assert!(
        bkp_show_out.status.success(),
        "sessions show on backup DB failed: {}",
        String::from_utf8_lossy(&bkp_show_out.stderr)
    );
    let bkp_shown: Value = serde_json::from_slice(&bkp_show_out.stdout).unwrap();
    assert_eq!(bkp_shown["session"]["id"], sid1);

    // Delete `sid1` from primary DB and verify it is gone
    let del = env.run_json(&["sessions", "delete", &sid1]);
    assert_eq!(del["ok"], true);
    assert_eq!(del["deleted"], true);

    let after_del = env.run_json(&["sessions", "list", "--json"]);
    assert!(
        !after_del["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == sid1),
        "deleted session {sid1} still appears in `nola sessions list`"
    );
    let show_deleted = env.run(&["sessions", "show", &sid1]);
    assert!(
        !show_deleted.status.success(),
        "showing deleted session must fail"
    );
}

// ============================================================================
// 5. Real Interactive PTY TUI E2E Test (`openpty` + `nola tui`)
// ============================================================================

#[cfg(unix)]
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() && bytes[i] != 0x07 {
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            } else {
                i += 2;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermColor {
    Default,
    Ansi16(u8),
    Palette256(u8),
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct TermCell {
    ch: char,
    fg: TermColor,
    bg: TermColor,
    bold: bool,
    dim: bool,
    continuation: bool,
}

#[cfg(unix)]
impl Default for TermCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: TermColor::Default,
            bg: TermColor::Default,
            bold: false,
            dim: false,
            continuation: false,
        }
    }
}

#[cfg(unix)]
fn vt_char_width(c: char) -> usize {
    if c.is_control() {
        return 0;
    }
    let cp = c as u32;
    if (0x0300..=0x036F).contains(&cp)
        || (0x1AB0..=0x1AFF).contains(&cp)
        || (0x1DC0..=0x1DFF).contains(&cp)
        || (0x200B..=0x200F).contains(&cp)
        || cp == 0x2060
        || (0x20D0..=0x20FF).contains(&cp)
        || (0xFE00..=0xFE0F).contains(&cp)
        || (0xFE20..=0xFE2F).contains(&cp)
        || cp == 0xFEFF
        || (0xE0100..=0xE01EF).contains(&cp)
    {
        return 0;
    }
    if (0x1100..=0x115F).contains(&cp)
        || (0x231A..=0x231B).contains(&cp)
        || (0x23E9..=0x23F3).contains(&cp)
        || (0x23F8..=0x23FA).contains(&cp)
        || (0x2600..=0x26FF).contains(&cp)
        || (0x2700..=0x27BF).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE10..=0xFE6F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x1F300..=0x1FAFF).contains(&cp)
        || (0x20000..=0x2FFFD).contains(&cp)
    {
        2
    } else {
        1
    }
}

/// Stateful 2D VT100/ANSI terminal emulator that verifies actual rendered cell geometry,
/// box-border closure & right-edge alignment, background tints, and colors — not just flat text.
#[cfg(unix)]
struct VirtualTerminal {
    cols: usize,
    rows: usize,
    grid: Vec<Vec<TermCell>>,
    cx: usize,
    cy: usize,
    fg: TermColor,
    bg: TermColor,
    bold: bool,
    dim: bool,
    pending_utf8: Vec<u8>,
    pending_esc: String,
}

#[cfg(unix)]
impl VirtualTerminal {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            grid: vec![vec![TermCell::default(); cols]; rows],
            cx: 0,
            cy: 0,
            fg: TermColor::Default,
            bg: TermColor::Default,
            bold: false,
            dim: false,
            pending_utf8: Vec::new(),
            pending_esc: String::new(),
        }
    }

    fn reset(&mut self) {
        self.grid = vec![vec![TermCell::default(); self.cols]; self.rows];
        self.cx = 0;
        self.cy = 0;
        self.fg = TermColor::Default;
        self.bg = TermColor::Default;
        self.bold = false;
        self.dim = false;
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        self.pending_utf8.extend_from_slice(bytes);
        let mut valid_len = self.pending_utf8.len();
        while valid_len > 0 {
            match std::str::from_utf8(&self.pending_utf8[..valid_len]) {
                Ok(s) => {
                    let owned = s.to_string();
                    self.pending_utf8.drain(..valid_len);
                    self.feed_str(&owned);
                    return;
                }
                Err(e) => {
                    let up_to = e.valid_up_to();
                    if up_to > 0 {
                        let owned =
                            std::str::from_utf8(&self.pending_utf8[..up_to]).unwrap().to_string();
                        self.pending_utf8.drain(..up_to);
                        self.feed_str(&owned);
                        return;
                    }
                    if e.error_len().is_none() {
                        // Incomplete trailing UTF-8 sequence — wait for next read.
                        return;
                    }
                    // Skip invalid byte
                    self.pending_utf8.remove(0);
                    valid_len = self.pending_utf8.len();
                }
            }
        }
    }

    fn feed_str(&mut self, input: &str) {
        let data = if self.pending_esc.is_empty() {
            input.to_string()
        } else {
            let mut s = std::mem::take(&mut self.pending_esc);
            s.push_str(input);
            s
        };
        let chars: Vec<char> = data.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\x1b' {
                if i + 1 >= chars.len() {
                    self.pending_esc = chars[i..].iter().collect();
                    return;
                }
                if chars[i + 1] == '[' {
                    let mut j = i + 2;
                    while j < chars.len() && !('@'..='~').contains(&chars[j]) {
                        j += 1;
                    }
                    if j >= chars.len() {
                        self.pending_esc = chars[i..].iter().collect();
                        return;
                    }
                    let seq: String = chars[i + 2..j].iter().collect();
                    let final_ch = chars[j];
                    self.handle_csi(&seq, final_ch);
                    i = j + 1;
                    continue;
                }
                i += 2;
                continue;
            }
            match ch {
                '\r' => self.cx = 0,
                '\n' => self.cy = (self.cy + 1).min(self.rows.saturating_sub(1)),
                c if !c.is_control() => {
                    let w = vt_char_width(c);
                    if w > 0 && self.cy < self.rows && self.cx < self.cols {
                        self.grid[self.cy][self.cx] = TermCell {
                            ch: c,
                            fg: self.fg,
                            bg: self.bg,
                            bold: self.bold,
                            dim: self.dim,
                            continuation: false,
                        };
                        if w == 2 && self.cx + 1 < self.cols {
                            self.grid[self.cy][self.cx + 1] = TermCell {
                                ch: '\0',
                                fg: self.fg,
                                bg: self.bg,
                                bold: self.bold,
                                dim: self.dim,
                                continuation: true,
                            };
                        }
                        self.cx = (self.cx + w).min(self.cols);
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn handle_csi(&mut self, seq: &str, final_ch: char) {
        if seq.starts_with('?') {
            return;
        }
        let parts: Vec<usize> = if seq.is_empty() {
            Vec::new()
        } else {
            seq.split(';')
                .map(|p| p.parse::<usize>().unwrap_or(0))
                .collect()
        };
        match final_ch {
            'H' | 'f' => {
                let row = parts.first().copied().unwrap_or(1).saturating_sub(1);
                let col = parts.get(1).copied().unwrap_or(1).saturating_sub(1);
                self.cy = row.min(self.rows.saturating_sub(1));
                self.cx = col.min(self.cols.saturating_sub(1));
            }
            'K' => {
                let mode = parts.first().copied().unwrap_or(0);
                let range = match mode {
                    0 => self.cx..self.cols,
                    1 => 0..(self.cx.saturating_add(1).min(self.cols)),
                    _ => 0..self.cols,
                };
                for x in range {
                    if x < self.cols {
                        self.grid[self.cy][x] = TermCell::default();
                    }
                }
            }
            'J' => self.reset(),
            'm' => {
                let codes = if parts.is_empty() { vec![0] } else { parts };
                let mut k = 0;
                while k < codes.len() {
                    match codes[k] {
                        0 => {
                            self.fg = TermColor::Default;
                            self.bg = TermColor::Default;
                            self.bold = false;
                            self.dim = false;
                        }
                        1 => self.bold = true,
                        2 => self.dim = true,
                        22 => {
                            self.bold = false;
                            self.dim = false;
                        }
                        p @ 30..=37 => self.fg = TermColor::Ansi16((p - 30) as u8),
                        39 => self.fg = TermColor::Default,
                        p @ 40..=47 => self.bg = TermColor::Ansi16((p - 40) as u8),
                        49 => self.bg = TermColor::Default,
                        p @ 90..=97 => self.fg = TermColor::Ansi16((p - 90 + 8) as u8),
                        38 if k + 2 < codes.len() && codes[k + 1] == 5 => {
                            self.fg = TermColor::Palette256(codes[k + 2] as u8);
                            k += 2;
                        }
                        48 if k + 2 < codes.len() && codes[k + 1] == 5 => {
                            self.bg = TermColor::Palette256(codes[k + 2] as u8);
                            k += 2;
                        }
                        _ => {}
                    }
                    k += 1;
                }
            }
            _ => {}
        }
    }

    fn row_text(&self, row: usize) -> String {
        let mut s = String::new();
        for cell in &self.grid[row] {
            if !cell.continuation {
                s.push(cell.ch);
            }
        }
        s.trim_end().to_string()
    }

    fn screen_text(&self) -> String {
        (0..self.rows)
            .map(|r| self.row_text(r))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Enforces that EVERY rounded box on the 2D screen is completely enclosed on all 4 sides
    /// with exact left/right column alignment and zero legacy half-open `┌`/`└` borders.
    fn assert_all_boxes_closed_and_aligned(&self) {
        let dump = self.screen_text();
        let mut active_box: Option<(usize, usize, usize)> = None; // (start_row, left_col, right_col)
        let mut boxes_found = 0usize;

        for r in 0..self.rows {
            let row = &self.grid[r];
            for c in 0..self.cols {
                assert!(
                    row[c].ch != '┌' && row[c].ch != '└',
                    "Found legacy half-open border '{}' at ({r},{c})!\nScreen:\n{dump}",
                    row[c].ch
                );
            }
            let top_left = row.iter().position(|cell| cell.ch == '╭');
            let top_right = row.iter().rposition(|cell| cell.ch == '╮');
            let bot_left = row.iter().position(|cell| cell.ch == '╰');
            let bot_right = row.iter().rposition(|cell| cell.ch == '╯');

            if let Some(lc) = top_left {
                let rc = top_right.unwrap_or_else(|| {
                    panic!("Unclosed top border `╭` without `╮` at row {r}!\nScreen:\n{dump}")
                });
                assert!(
                    rc > lc + 4,
                    "Degenerate box width at row {r}: lc={lc}, rc={rc}\nScreen:\n{dump}"
                );
                assert!(
                    active_box.is_none(),
                    "Nested or unclosed previous box before row {r}\nScreen:\n{dump}"
                );
                active_box = Some((r, lc, rc));
                boxes_found += 1;
            } else if let Some(lc) = bot_left {
                let rc = bot_right.unwrap_or_else(|| {
                    panic!("Unclosed bottom border `╰` without `╯` at row {r}!\nScreen:\n{dump}")
                });
                let (sr, exp_lc, exp_rc) = active_box.take().unwrap_or_else(|| {
                    panic!("Orphan bottom border `╰...╯` at row {r}\nScreen:\n{dump}")
                });
                assert_eq!(
                    (lc, rc),
                    (exp_lc, exp_rc),
                    "Box bottom corners ({lc},{rc}) at row {r} misaligned with top ({exp_lc},{exp_rc}) from row {sr}!\nScreen:\n{dump}"
                );
            } else if let Some((sr, exp_lc, exp_rc)) = active_box {
                assert_eq!(
                    row[exp_lc].ch, '│',
                    "Box left border missing/misaligned at row {r} col {exp_lc} (box started at row {sr})\nScreen:\n{dump}"
                );
                assert_eq!(
                    row[exp_rc].ch, '│',
                    "Box right border missing/misaligned at row {r} col {exp_rc} (box started at row {sr}, got '{}')\nScreen:\n{dump}",
                    row[exp_rc].ch
                );
            }
        }
        assert!(
            active_box.is_none(),
            "Box opened at {:?} was never closed with `╰...╯`!\nScreen:\n{dump}",
            active_box
        );
        assert!(
            boxes_found > 0,
            "Expected at least one full-enclosed rounded box on screen!\nScreen:\n{dump}"
        );
    }

    /// Verifies minimalist top chrome (no harsh DarkBlue banner) and clean bottom status bar.
    fn assert_minimalist_chrome(&self) {
        let dump = self.screen_text();
        for c in 0..self.cols {
            assert_ne!(
                self.grid[0][c].bg,
                TermColor::Ansi16(4),
                "Row 0 must not use harsh solid DarkBlue banner!\nScreen:\n{dump}"
            );
        }
        let row0 = self.row_text(0);
        assert!(
            row0.contains("Nola"),
            "Top bar should display `Nola`, got: `{row0}`\nScreen:\n{dump}"
        );
        let last_row = self.row_text(self.rows - 1);
        assert!(
            last_row.contains("Context") || last_row.contains("上下文"),
            "Bottom status bar should display Context %, got: `{last_row}`\nScreen:\n{dump}"
        );
    }

    /// Verifies user message has NO `You` label and uses full-width subtle background tint + `▎`.
    fn assert_user_message_tinted_without_you(&self, snippet: &str) {
        let dump = self.screen_text();
        assert!(
            !dump.contains("▸ You") && !dump.contains("You\n"),
            "User message must NOT render redundant `You` header!\nScreen:\n{dump}"
        );
        let row_idx = (0..self.rows)
            .find(|&r| self.row_text(r).contains(snippet))
            .unwrap_or_else(|| {
                panic!("Could not find user message `{snippet}` on 2D screen!\nScreen:\n{dump}")
            });
        let row = &self.grid[row_idx];
        assert!(
            row.iter().any(|c| c.ch == '▎'),
            "User message row {row_idx} should have left accent `▎`\nScreen:\n{dump}"
        );
        let tinted_cells = row
            .iter()
            .filter(|c| c.bg != TermColor::Default)
            .count();
        assert!(
            tinted_cells >= self.cols.saturating_sub(6),
            "User message row {row_idx} should be full-width background tinted (got {tinted_cells}/{} tinted cells)\nScreen:\n{dump}",
            self.cols
        );
    }

    fn assert_row_has_red_fg(&self, snippet: &str) {
        let dump = self.screen_text();
        let row_idx = (0..self.rows)
            .find(|&r| self.row_text(r).contains(snippet))
            .unwrap_or_else(|| {
                panic!("Could not find error row `{snippet}` on 2D screen!\nScreen:\n{dump}")
            });
        let has_red = self.grid[row_idx]
            .iter()
            .any(|c| matches!(c.fg, TermColor::Ansi16(1) | TermColor::Ansi16(9)));
        assert!(
            has_red,
            "Expected red foreground on error row {row_idx} (`{snippet}`)\nScreen:\n{dump}"
        );
    }
}

#[cfg(unix)]
struct PtySession {
    master_fd: libc::c_int,
    child: std::process::Child,
    raw_log: String,
    clean_log: String,
    vt: VirtualTerminal,
}

#[cfg(unix)]
impl PtySession {
    fn spawn(env: &IsolatedEnv) -> Self {
        Self::spawn_with_opts(env, &env.workspace, "en_US.UTF-8", 132, 36)
    }

    fn spawn_with_opts(
        env: &IsolatedEnv,
        project_dir: &std::path::Path,
        lang: &str,
        cols: u16,
        rows: u16,
    ) -> Self {
        use std::os::unix::process::CommandExt;

        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let mut ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ws,
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());

        // Set master non-blocking so reads never hang
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
            libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        }

        let mut cmd = env.cmd();
        cmd.env("TERM", "xterm-256color")
            .env("NOLA_LANG", lang)
            .arg("tui")
            .arg("--db")
            .arg(&env.db_path)
            .arg("--project")
            .arg(project_dir);

        unsafe {
            cmd.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "ios"))]
                {
                    let _ = libc::ioctl(slave, libc::TIOCSCTTY as libc::c_ulong, 0);
                }
                #[cfg(target_os = "linux")]
                {
                    let _ = libc::ioctl(slave, libc::TIOCSCTTY as libc::c_ulong, 0);
                }
                if libc::dup2(slave, 0) < 0
                    || libc::dup2(slave, 1) < 0
                    || libc::dup2(slave, 2) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if slave > 2 {
                    libc::close(slave);
                }
                libc::close(master);
                Ok(())
            });
        }

        let child = cmd.spawn().expect("spawn nola tui inside PTY");
        unsafe {
            libc::close(slave);
        }

        Self {
            master_fd: master,
            child,
            raw_log: String::new(),
            clean_log: String::new(),
            vt: VirtualTerminal::new(cols as usize, rows as usize),
        }
    }

    fn drain_once(&mut self) -> usize {
        let mut buf = [0u8; 8192];
        let mut total = 0;
        loop {
            let n = unsafe { libc::read(self.master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                let slice = &buf[..n as usize];
                self.vt.feed_bytes(slice);
                let chunk = String::from_utf8_lossy(slice);
                self.raw_log.push_str(&chunk);
                self.clean_log = strip_ansi(&self.raw_log);
                total += n as usize;
            } else {
                break;
            }
        }
        total
    }

    fn wait_screen_contains(&mut self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_once();
            if self.vt.screen_text().contains(needle) {
                return;
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain_once();
                panic!(
                    "nola tui exited early ({status:?}) while waiting for `{needle}` on 2D screen.\n2D Screen:\n{}",
                    self.vt.screen_text()
                );
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out waiting for `{needle}` on live 2D screen.\n2D Screen:\n{}",
                    self.vt.screen_text()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_screen_missing(&mut self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_once();
            if !self.vt.screen_text().contains(needle) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out waiting for `{needle}` to disappear from live 2D screen.\n2D Screen:\n{}",
                    self.vt.screen_text()
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for(&mut self, needle: &str, since_offset: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_once();
            if self.clean_log.len() >= since_offset && self.clean_log[since_offset..].contains(needle)
            {
                return;
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain_once();
                panic!(
                    "nola tui exited early ({status:?}) while waiting for `{needle}`.\nCaptured screen output:\n{}",
                    &self.clean_log[since_offset.min(self.clean_log.len())..]
                );
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out waiting for `{needle}` in PTY output.\nCaptured tail:\n{}",
                    &self.clean_log[since_offset.min(self.clean_log.len())..]
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn send_raw(&mut self, payload: &[u8]) -> usize {
        self.drain_once();
        let mark = self.clean_log.len();
        let mut written = 0;
        while written < payload.len() {
            let n = unsafe {
                libc::write(
                    self.master_fd,
                    payload[written..].as_ptr().cast(),
                    payload.len() - written,
                )
            };
            if n > 0 {
                written += n as usize;
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
        mark
    }

    fn send_command(&mut self, line: &str) -> usize {
        let payload = format!("{line}\r");
        self.send_raw(payload.as_bytes())
    }

    fn wait_exit(mut self, timeout: Duration) -> (std::process::ExitStatus, String) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_once();
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain_once();
                let clean = self.clean_log.clone();
                return (status, clean);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!(
                    "timed out waiting for `nola tui` to exit cleanly.\nCaptured PTY output:\n{}",
                    self.clean_log
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(unix)]
impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

#[cfg(unix)]
#[test]
fn tui_real_pty_interactive_slash_commands_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();

    // Seed isolated temp DB & workspace so TUI has model, memory, skill, and checkpointable file
    env.write_and_apply_mock_config(&mock.v1_endpoint());
    env.run_json(&[
        "memory",
        "create",
        "--title",
        "PTY Memory Fact",
        "--content",
        "Remember isolated PTY testing",
    ]);
    let skill_dir = env.workspace.join(".nola/skills/pty-demo-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: pty-demo-skill\ndescription: Demo skill for PTY TUI verification\n---\nHello.\n",
    )
    .unwrap();
    fs::write(env.workspace.join("tracked.txt"), "initial checkpoint content\n").unwrap();

    let mut pty = PtySession::spawn(&env);
    let t = Duration::from_secs(8);

    // 1. TUI boots cleanly into minimalist welcome screen with `›` prompt
    pty.wait_for("Nola", 0, t);
    pty.wait_for("›", 0, t);

    // 1b. Typing `/co` opens live floating full-enclosed slash command menu;
    // first Esc dismisses menu, second Esc clears composer back to placeholder.
    let m = pty.send_raw(b"/co");
    pty.wait_for("╭─", m, t);
    pty.wait_for("╮", m, t);
    pty.wait_for("/compact", m, t);
    pty.wait_for("╰─", m, t);
    pty.wait_for("╯", m, t);
    let _ = pty.send_raw(b"\x1b");
    thread::sleep(Duration::from_millis(300));
    let m = pty.send_raw(b"\x03");
    pty.wait_for("Type a message...", m, t);

    // 2. `/help`
    let m = pty.send_command("/help");
    pty.wait_for("Slash commands", m, t);

    // 3. `/status`
    let m = pty.send_command("/status");
    pty.wait_for("Permission:  cautious", m, t);

    let m = pty.send_command("/diff");
    pty.wait_for("Open a Git working tree", m, t);

    // 4. `/allow cautious` & `/allow allow_all` & reject `/allow accept_edits`
    let m = pty.send_command("/allow cautious");
    pty.wait_for("Switched permission profile to 'cautious'", m, t);

    let m = pty.send_command("/allow allow_all");
    pty.wait_for("Switched permission profile to 'allow_all'", m, t);

    let m = pty.send_command("/allow accept_edits");
    pty.wait_for("invalid permission profile 'accept_edits'", m, t);

    // Confirm permission stayed `allow_all` after rejected `/allow accept_edits`
    let m = pty.send_command("/status");
    pty.wait_for("Permission:  allow_all", m, t);

    // 5. `/thinking high` & `/thinking none`
    let m = pty.send_command("/thinking high");
    pty.wait_for("Switched reasoning effort to 'high'", m, t);

    let m = pty.send_command("/thinking none");
    pty.wait_for("Switched reasoning effort to 'none'", m, t);

    // 5b. Interactive `/model` picker modal: full-enclosed box -> Enter confirms active model
    let m = pty.send_command("/model");
    pty.wait_for("╭─", m, t);
    pty.wait_for("Select Model", m, t);
    pty.wait_for("╮", m, t);
    pty.wait_for("filter›", m, t);
    let m = pty.send_raw(b"\r");
    pty.wait_for("Switched active model to", m, t);

    // 5c. Interactive `/thinking` picker modal: open -> Esc cancels back to composer
    let m = pty.send_command("/thinking");
    pty.wait_for("Select Reasoning Effort", m, t);
    let m = pty.send_raw(b"\x1b");
    pty.wait_for("›", m, t);

    // 6. `/plan on` & `/plan off`
    let m = pty.send_command("/plan on");
    pty.wait_for("Plan Mode enabled [PLAN]", m, t);

    let m = pty.send_command("/plan off");
    pty.wait_for("Plan Mode disabled.", m, t);

    // 7. `/memory list`
    let m = pty.send_command("/memory list");
    pty.wait_for("PTY Memory Fact", m, t);

    // 8. `/skills`
    let m = pty.send_command("/skills");
    pty.wait_for("pty-demo-skill", m, t);

    // 9. `/tools` (full list + filtered lookup so both Deferred and Core tools are verified in viewport)
    let m = pty.send_command("/tools");
    pty.wait_for("[Deferred]", m, t);
    let m = pty.send_command("/tools read_file");
    pty.wait_for("read_file", m, t);

    // 10. `/checkpoint`
    let m = pty.send_command("/checkpoint pty-snap");
    pty.wait_for("Created checkpoint", m, t);

    for args in [&["init", "-q"][..], &["add", "tracked.txt"]] {
        let status = Command::new("git")
            .args(args)
            .current_dir(&env.workspace)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
    let m = pty.send_command("/diff");
    pty.wait_for("(no diff)", m, t);
    let m = pty.send_command("/diff --staged");
    pty.wait_for("1 insertion", m, t);

    // 11. `/quit` exits cleanly with code 0 and zero panic
    let _ = pty.send_command("/quit");
    let (status, full_screen) = pty.wait_exit(Duration::from_secs(8));
    assert!(
        status.success(),
        "expected `nola tui` to exit with code 0 after /quit, got {status:?}\nscreen:\n{full_screen}"
    );
    assert!(
        !full_screen.contains("panicked at"),
        "detected panic in TUI output:\n{full_screen}"
    );
    assert!(!full_screen.contains("git diff --no-index"));
}

// ============================================================================
// 6. 2D Terminal UI Geometry, CJK Alignment, Git Subdir & Picker Persistence E2E
// ============================================================================

#[cfg(unix)]
#[test]
fn tui_pty_2d_cjk_git_subdir_slash_menu_and_model_picker_persistence_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();

    // Multi-model config so /model picker has multiple entries to filter & switch
    let cfg_path = env.workspace.join("nola.json");
    fs::write(
        &cfg_path,
        serde_json::to_string_pretty(&json!({
            "llm": {
                "default_model": { "connection": "primary", "model": "mock-reasoner-7b" },
                "connections": [
                    {
                        "id": "primary",
                        "endpoint": mock.v1_endpoint(),
                        "protocol": "chat_completions",
                        "api_key": "sk-mock-primary",
                        "models": [
                            { "id": "mock-reasoner-7b", "context_window": 131072 },
                            { "id": "mock-coder-14b", "context_window": 262144 }
                        ]
                    },
                    {
                        "id": "backup",
                        "endpoint": mock.v1_endpoint(),
                        "protocol": "chat_completions",
                        "api_key": "sk-mock-backup",
                        "models": [
                            { "id": "openrouter/free", "context_window": 128000 }
                        ]
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    env.run_json(&["config", "apply", cfg_path.to_str().unwrap()]);

    // Create a Git repo at workspace root and launch TUI inside `scripts/` subdirectory!
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&env.workspace)
        .status()
        .unwrap()
        .success());
    let subdir = env.workspace.join("scripts");
    fs::create_dir_all(&subdir).unwrap();

    let t = Duration::from_secs(8);
    let mut pty = PtySession::spawn_with_opts(&env, &subdir, "zh-Hans", 104, 30);

    // 1. Verify minimalist zh-Hans welcome chrome & Git subdirectory root warning on 2D screen
    pty.wait_screen_contains("本地优先个人助手", t);
    pty.wait_screen_contains("Git 仓库根", t);
    pty.vt.assert_minimalist_chrome();

    // 2. Typing `/co` opens floating live slash menu -> verify ALL 4 box borders & CJK column alignment!
    let _ = pty.send_raw(b"/co");
    pty.wait_screen_contains("/compact", t);
    pty.wait_screen_contains("压缩模型上下文", t);
    pty.vt.assert_all_boxes_closed_and_aligned();

    // Esc dismisses slash menu cleanly, Ctrl+C clears `/co` back to placeholder
    let _ = pty.send_raw(b"\x1b");
    pty.wait_screen_missing("压缩模型上下文", t);
    let _ = pty.send_raw(b"\x03");
    pty.wait_screen_contains("输入消息...", t);

    // 3. `/model` opens interactive picker -> verify 4-sided full box -> filter `coder` -> Enter switches!
    let _ = pty.send_command("/model");
    pty.wait_screen_contains("选择模型", t);
    pty.wait_screen_contains("filter›", t);
    pty.vt.assert_all_boxes_closed_and_aligned();

    let _ = pty.send_raw(b"coder");
    pty.wait_screen_contains("(1/1)", t);
    pty.wait_screen_contains("mock-coder-14b", t);
    pty.vt.assert_all_boxes_closed_and_aligned();

    let _ = pty.send_raw(b"\r");
    pty.wait_screen_contains("Switched active model to primary:mock-coder-14b", t);
    pty.wait_screen_contains("mock-coder-14b · 谨慎", t);

    // 4. `/allow` opens interactive 2-mode permission picker -> verify full box -> Down + Enter -> 全部允许
    let _ = pty.send_command("/allow");
    pty.wait_screen_contains("选择权限模式", t);
    pty.vt.assert_all_boxes_closed_and_aligned();
    let _ = pty.send_raw(b"\x1b[B");
    thread::sleep(Duration::from_millis(60));
    let _ = pty.send_raw(b"\r");
    pty.wait_screen_contains("allow_all", t);
    pty.wait_screen_contains("全部允许", t);

    // 5. Exit and RELAUNCH TUI -> verify `/model` switch PERSISTED across restart!
    let _ = pty.send_command("/quit");
    let (status, _) = pty.wait_exit(t);
    assert!(status.success());

    let mut pty2 = PtySession::spawn_with_opts(&env, &subdir, "zh-Hans", 104, 30);
    pty2.wait_screen_contains("mock-coder-14b", t);
    pty2.vt.assert_minimalist_chrome();
    let _ = pty2.send_command("/quit");
    let (status2, _) = pty2.wait_exit(t);
    assert!(status2.success());
}

// ============================================================================
// 7. Real Multi-Turn Chat in PTY: User Tint (No You), Full-Box Code, Tools,
//    Cautious Read-Only Bypass, Prefix Approval Grant, Pause Feedback, HTTP 500
// ============================================================================

#[cfg(unix)]
#[test]
fn tui_pty_real_multiturn_chat_user_tint_codeblock_tools_approvals_and_errors_e2e() {
    let env = IsolatedEnv::new();
    let mock = MockLlmServer::start();
    env.write_and_apply_mock_config(&mock.v1_endpoint());

    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&env.workspace)
        .status()
        .unwrap()
        .success());
    fs::write(env.workspace.join("demo.rs"), "fn main() {}\n").unwrap();

    // Queue Turn 1: read-only `bash("pwd")` (must NOT prompt!) -> Markdown reply with reasoning + Rust code block
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_ro_pwd".into(),
        name: "bash".into(),
        arguments: json!({ "command": "pwd" }),
    });
    mock.push_turn(MockSseTurn::Text {
        reasoning: Some("先确认当前目录，再给用户一个带中文注释的代码示例。".into()),
        text: "已完成目录检查，示例代码如下：\n\n```rust\nfn greet() {\n    println!(\"你好 Nola\");\n}\n```\n\n- 四边完整闭合".into(),
    });

    // Queue Turn 2: failing write bash (exit 128 + Operation not permitted -> prompts -> press `2` -> prefix grant!)
    // followed by second command with SAME `sh` prefix (must NOT re-prompt!) -> summary reply.
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_fail_128".into(),
        name: "bash".into(),
        arguments: json!({ "command": "sh -c 'echo \"fatal: Operation not permitted\" >&2; exit 128'" }),
    });
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_prefix_reuse".into(),
        name: "bash".into(),
        arguments: json!({ "command": "sh -c 'echo prefix_reuse_ok'" }),
    });
    mock.push_turn(MockSseTurn::Text {
        reasoning: None,
        text: "已确认沙箱报错并验证同类前缀免重复审批。".into(),
    });

    // Queue Turn 3: new write tool command -> user presses `3` (ReturnToInput) -> must show pause feedback, not blank!
    mock.push_turn(MockSseTurn::ToolCall {
        call_id: "call_pause_me".into(),
        name: "bash".into(),
        arguments: json!({ "command": "touch should_not_be_created.txt" }),
    });

    // Queue Turn 4: non-retryable HTTP 400 error -> must display red error & persist `turn_failed` event in SQLite!
    mock.push_turn(MockSseTurn::HttpError {
        status: 400,
        body: json!({"error": {"message": "mock upstream rejection"}}).to_string(),
    });

    let t = Duration::from_secs(12);
    let mut pty = PtySession::spawn_with_opts(&env, &env.workspace, "zh-Hans", 108, 34);
    pty.wait_screen_contains("输入消息...", t);

    // --- Turn 1 ---
    let _ = pty.send_command("帮我检查目录并展示代码");
    pty.wait_screen_contains("四边完整闭合", t);
    pty.vt.assert_minimalist_chrome();
    pty.vt.assert_user_message_tinted_without_you("帮我检查目录并展示代码");
    // Markdown fenced code block on 2D screen must be 4-side enclosed & aligned!
    pty.vt.assert_all_boxes_closed_and_aligned();
    // Collapsed thinking summary & informative read-only tool card (without approval modal!)
    let s1 = pty.vt.screen_text();
    assert!(s1.contains("思考完成"), "expected collapsed thinking summary:\n{s1}");
    assert!(s1.contains("✓ bash") && s1.contains("pwd"), "expected readable tool card with command preview:\n{s1}");
    assert!(!s1.contains("权限审批"), "read-only `pwd` must NOT pop up approval modal:\n{s1}");

    // --- Turn 2 ---
    let _ = pty.send_command("测试失败命令和前缀授权");
    pty.wait_screen_contains("权限审批", t);
    pty.vt.assert_all_boxes_closed_and_aligned();
    assert!(
        !pty.vt.screen_text().contains("[sha256:"),
        "approval modal must not show ugly [sha256:...] noise:\n{}",
        pty.vt.screen_text()
    );
    // Press `2` (Allow in session) -> failing exit 128 runs, then second `sh -c` runs WITHOUT re-prompting!
    let _ = pty.send_raw(b"2");
    pty.wait_screen_contains("已确认沙箱报错并验证同类前缀免重复审批", t);
    let s2 = pty.vt.screen_text();
    assert!(
        s2.contains("✗ bash") && s2.contains("128") && s2.contains("Operation not permitted"),
        "failed bash exit 128 must render as red `✗ bash` with stderr, not `OK bash`:\n{s2}"
    );
    pty.vt.assert_row_has_red_fg("✗ bash");

    // Verify the model ACTUALLY received `sandbox_hint` in the subsequent chat request!
    let reqs = mock.chat_requests();
    let saw_sandbox_hint = reqs.iter().any(|r| {
        r.body
            .as_ref()
            .map(|b| b.to_string().contains("sandbox_hint"))
            .unwrap_or(false)
    });
    assert!(saw_sandbox_hint, "model must receive `sandbox_hint` on Operation not permitted!");

    // --- Turn 3: press `3` (ReturnToInput) on Approval modal ---
    let _ = pty.send_command("执行待拦截命令");
    pty.wait_screen_contains("权限审批", t);
    pty.vt.assert_all_boxes_closed_and_aligned();
    let _ = pty.send_raw(b"3");
    pty.wait_screen_contains("Paused before `bash`", t);
    assert!(
        !env.workspace.join("should_not_be_created.txt").exists(),
        "paused command must not have executed"
    );

    // --- Turn 4: HTTP 400 -> red error rendered + `turn_failed` persisted in SQLite ---
    let _ = pty.send_command("触发上游报错测试");
    pty.wait_screen_contains("mock upstream rejection", t);
    pty.vt.assert_row_has_red_fg("mock upstream rejection");

    let _ = pty.send_command("/quit");
    let (status, _) = pty.wait_exit(t);
    assert!(status.success());

    // Inspect SQLite directly to verify `turn_failed` event was persisted!
    let store = nola_store::Store::open(&env.db_path).expect("open test db");
    let sessions = store.list_sessions(5).expect("list sessions");
    assert!(!sessions.is_empty());
    let events = store
        .read_event_range(&sessions[0].id, 0, 500)
        .expect("list events");
    assert!(
        events.iter().any(|e| e.kind == "turn_failed"),
        "HTTP 500 turn failure must persist a `turn_failed` event; got kinds: {:?}",
        events.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

// ============================================================================
// 8. `ask --help` / `chat --help` Zero-Session Invariant & Native `git add` E2E
// ============================================================================

#[test]
fn cli_help_never_creates_sessions_and_git_add_stage_e2e() {
    let env = IsolatedEnv::new();

    // 1. All help invocations must exit 0 without creating any DB sessions!
    for args in [
        &["--help"][..],
        &["ask", "--help"][..],
        &["chat", "--help"][..],
        &["tui", "--help"][..],
    ] {
        let out = env
            .cmd()
            .arg("--db")
            .arg(&env.db_path)
            .args(args)
            .output()
            .expect("run help");
        assert!(
            out.status.success(),
            "nola {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    if env.db_path.exists() {
        let store = nola_store::Store::open(&env.db_path).expect("open db");
        assert!(
            store.list_sessions(20).unwrap().is_empty(),
            "help commands must NEVER create session records!"
        );
    }

    // 2. Native `git` tool `add`/`stage` works cleanly in a real repo
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&env.workspace)
        .status()
        .unwrap()
        .success());
    fs::write(env.workspace.join("staged_via_tool.txt"), "hello stage\n").unwrap();

    let reg = nola_tools::ToolRegistry::with_builtin_tools();
    let ctx = nola_tools::ToolContext::new(&env.workspace);
    let add_res = reg.execute(
        "git",
        &ctx,
        json!({ "action": "add", "paths": ["staged_via_tool.txt"] }),
    );
    assert!(!add_res.is_error, "git add failed: {:?}", add_res.payload);
    let status_res = reg.execute("git", &ctx, json!({ "action": "status" }));
    assert!(
        status_res.payload.to_model_value()["output"]
            .as_str()
            .unwrap_or("")
            .contains("A  staged_via_tool.txt"),
        "expected file to be staged: {:?}",
        status_res.payload
    );
}
