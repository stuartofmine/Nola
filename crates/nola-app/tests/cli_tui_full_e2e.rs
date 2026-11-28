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
struct PtySession {
    master_fd: libc::c_int,
    child: std::process::Child,
    raw_log: String,
    clean_log: String,
}

#[cfg(unix)]
impl PtySession {
    fn spawn(env: &IsolatedEnv) -> Self {
        use std::os::unix::process::CommandExt;

        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let mut ws = libc::winsize {
            ws_row: 44,
            ws_col: 132,
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
            .arg("tui")
            .arg("--db")
            .arg(&env.db_path)
            .arg("--project")
            .arg(&env.workspace);

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
        }
    }

    fn drain_once(&mut self) -> usize {
        let mut buf = [0u8; 8192];
        let mut total = 0;
        loop {
            let n = unsafe { libc::read(self.master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                let chunk = String::from_utf8_lossy(&buf[..n as usize]);
                self.raw_log.push_str(&chunk);
                self.clean_log = strip_ansi(&self.raw_log);
                total += n as usize;
            } else {
                break;
            }
        }
        total
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

    fn send_command(&mut self, line: &str) -> usize {
        self.drain_once();
        let mark = self.clean_log.len();
        let payload = format!("{line}\r");
        let bytes = payload.as_bytes();
        let mut written = 0;
        while written < bytes.len() {
            let n = unsafe {
                libc::write(
                    self.master_fd,
                    bytes[written..].as_ptr().cast(),
                    bytes.len() - written,
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

    // 1. TUI boots cleanly into interactive screen
    pty.wait_for("Nola v2", 0, t);
    pty.wait_for("nola>", 0, t);

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
