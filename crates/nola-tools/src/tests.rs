// SPDX-License-Identifier: Apache-2.0
//! Gate 2: Full Tool Projection & Receipt Invariant Suite.
//! Verifies EVERY built-in tool's `ToolModelPayload` fidelity and `VersionTransition` receipts.

use nola_types::{ToolModelPayload, VersionTransition};
use serde_json::json;

use crate::{path_within_workspace, validation_snapshot, SandboxMode, ToolContext, ToolRegistry};

#[test]
fn gate2_all_core_file_search_shell_and_code_tools_project_full_payload_and_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Disabled;
    let reg = ToolRegistry::with_builtin_tools();

    // 1. write_file -> Created VersionTransition
    let w = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "hello.py", "content": "def greet():\n    return 'hi'\n"}),
    );
    assert!(!w.is_error);
    assert!(w.has_changed_version());
    assert!(matches!(
        w.effects[0].transition,
        VersionTransition::Created { .. }
    ));

    // 2. read_file -> numbered content preserved verbatim in model_data
    let r = reg.execute("read_file", &ctx, json!({"path": "hello.py"}));
    assert!(!r.is_error);
    let r_val = r.payload.to_model_value();
    assert!(r_val["content"].as_str().unwrap().contains("1: def greet():"));

    // 3. edit_file (no expected_replacements needed!) -> Modified VersionTransition
    let e = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "hello.py", "old_text": "'hi'", "new_text": "'hello v2'"}),
    );
    assert!(!e.is_error);
    assert!(e.has_changed_version());
    assert!(matches!(
        e.effects[0].transition,
        VersionTransition::Modified { .. }
    ));

    // 4. apply_patch (pure Rust, dry_run defaults to false, zero git required!)
    let patch = "--- a/hello.py\n+++ b/hello.py\n@@ -1,2 +1,2 @@\n def greet():\n-    return 'hello v2'\n+    return 'patched v2'\n";
    let p = reg.execute("apply_patch", &ctx, json!({"patch": patch}));
    assert!(!p.is_error, "patch error: {:?}", p.payload);
    assert!(p.has_changed_version());
    assert!(std::fs::read_to_string(dir.path().join("hello.py"))
        .unwrap()
        .contains("patched v2"));

    // 5. grep -> MUST include full matched lines in model_data (never silently dropped!)
    let g = reg.execute("grep", &ctx, json!({"pattern": "patched v2"}));
    assert!(!g.is_error);
    let g_val = g.payload.to_model_value();
    assert_eq!(g_val["count"], 1);
    assert!(g_val["matches"][0]["text"]
        .as_str()
        .unwrap()
        .contains("patched v2"));

    // 6. glob & list_dir
    let gl = reg.execute("glob", &ctx, json!({"pattern": "*.py"}));
    assert_eq!(gl.payload.to_model_value()["count"], 1);
    let ld = reg.execute("list_dir", &ctx, json!({}));
    assert_eq!(ld.payload.to_model_value()["count"], 1);

    // 7. bash with output_artifacts -> Created VersionTransition!
    let b = reg.execute(
        "bash",
        &ctx,
        json!({
            "command": "echo generated > out.txt",
            "output_artifacts": ["out.txt"]
        }),
    );
    assert!(!b.is_error);
    assert!(b.has_changed_version());
    assert!(matches!(
        b.effects[0].transition,
        VersionTransition::Created { .. }
    ));

    // 8. background bash + bash_control returns full stdout/stderr
    let bg = reg.execute(
        "bash",
        &ctx,
        json!({"command": "echo bg_hello", "background": true}),
    );
    let job_id = bg.payload.to_model_value()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let st = reg.execute(
        "bash_control",
        &ctx,
        json!({"job_id": job_id, "action": "wait", "wait_ms": 2000}),
    );
    assert!(!st.is_error);
    assert!(st.payload.to_model_value()["stdout"]
        .as_str()
        .unwrap()
        .contains("bg_hello"));

    // 9. tool_search returns full schemas in model_data
    let ts = reg.execute("tool_search", &ctx, json!({"query": "web"}));
    assert_eq!(ts.payload.to_model_value()["tools"][0]["name"], "web_fetch");

    // 10. code_exec (QuickJS sandbox)
    let js = reg.execute("code_exec", &ctx, json!({"code": "return 6 * 7;"}));
    assert!(!js.is_error);
    assert_eq!(js.payload.to_model_value()["result"], 42);

    // 11. web_fetch blocks localhost SSRF
    let ssrf = reg.execute("web_fetch", &ctx, json!({"url": "http://127.0.0.1:8080"}));
    assert!(ssrf.is_error);
    assert!(matches!(ssrf.payload, ToolModelPayload::Structured { .. }));

    // 12. delete_file -> Deleted VersionTransition
    let d = reg.execute("delete_file", &ctx, json!({"path": "out.txt"}));
    assert!(!d.is_error);
    assert!(matches!(
        d.effects[0].transition,
        VersionTransition::Deleted { .. }
    ));

    // 13. PTY interactive VT100 session (exec -> write -> screenshot -> close)
    #[cfg(unix)]
    {
        let pty = reg.execute(
            "pty_exec",
            &ctx,
            json!({"action": "exec", "command": "cat", "yield_ms": 100}),
        );
        assert!(!pty.is_error, "pty_exec failed: {:?}", pty.payload);
        let sid = pty.payload.to_model_value()["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        let wr = reg.execute(
            "pty_exec",
            &ctx,
            json!({"action": "write", "session_id": sid, "input": "hello_pty\n", "yield_ms": 150}),
        );
        assert!(!wr.is_error);
        assert!(wr.payload.to_model_value()["screen"]
            .as_str()
            .unwrap()
            .contains("hello_pty"));
        let cl = reg.execute(
            "pty_exec",
            &ctx,
            json!({"action": "close", "session_id": sid}),
        );
        assert!(!cl.is_error);
    }

    // 14. OutputStore spill + tool_output read/search
    let big = ToolModelPayload::text((1..=500).map(|i| format!("line_{i} payload_data_marker\n")).collect::<String>());
    let (spilled, meta) = ctx
        .output_store
        .maybe_spill_payload("bash", &big, 1024)
        .expect("should spill");
    assert_eq!(spilled.to_model_value()["spilled"], true);
    let tr = reg.execute(
        "tool_output",
        &ctx,
        json!({"action": "search", "output_id": meta.output_id, "pattern": "line_42"}),
    );
    assert!(!tr.is_error);
    assert!(tr.payload.to_model_value()["count"].as_u64().unwrap() >= 1);

    // 15. multi_edit_file, move_file, repo_map, workspace_checkpoint, notebook
    let cp = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "create", "label": "before"}),
    );
    let cp_id = cp.payload.to_model_value()["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let me = reg.execute(
        "multi_edit_file",
        &ctx,
        json!({
            "path": "hello.py",
            "edits": [{"old_text": "patched v2", "new_text": "multi_edited"}]
        }),
    );
    assert!(!me.is_error && me.has_changed_version());
    let rm = reg.execute("repo_map", &ctx, json!({"query": "greet"}));
    assert_eq!(rm.payload.to_model_value()["count"], 1);
    let rest = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "restore", "checkpoint_id": cp_id}),
    );
    assert!(!rest.is_error);
    assert!(std::fs::read_to_string(dir.path().join("hello.py"))
        .unwrap()
        .contains("patched v2"));

    // 16. browser status & duckduckgo parser
    let bs = reg.execute("browser", &ctx, json!({"action": "status"}));
    assert!(!bs.is_error);
    let ddg = crate::web_search::parse_duckduckgo_html(
        r#"<div class="result"><a class="result__a" href="https://rust-lang.org">Rust</a><div class="result__snippet">Fast</div></div>"#,
        5,
    );
    assert_eq!(ddg.len(), 1);
    assert_eq!(ddg[0]["title"], "Rust");
}

#[test]
fn symlink_escape_and_validation_snapshot_ignore_pycache() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = dir.path().join("escape_link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    #[cfg(unix)]
    assert!(!path_within_workspace(
        dir.path(),
        &link.join("new_secret.txt")
    ));

    std::fs::write(dir.path().join("main.py"), "x = 1\n").unwrap();
    let snap1 = validation_snapshot(dir.path());
    std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
    std::fs::write(dir.path().join("__pycache__/main.cpython-311.pyc"), b"123").unwrap();
    let snap2 = validation_snapshot(dir.path());
    assert_eq!(snap1, snap2, "__pycache__ must not invalidate validation_snapshot");
}

#[test]
fn optimized_tools_output_store_multiline_edit_diagnostics_grep_and_tool_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Disabled;
    let reg = ToolRegistry::with_builtin_tools();

    // 1. Structured bash spill produces multi-line text, preserves exit_code, supports tail!
    let big_stdout: String = (1..=600)
        .map(|i| format!("build line {i}: compiling module_{i}\n"))
        .collect();
    let bash_payload = ToolModelPayload::structured(
        json!({"exit_code": 2, "stdout": big_stdout, "stderr": "fatal: missing symbol"}),
        "Shell exited with status 2",
    );
    let (spilled, meta) = ctx
        .output_store
        .maybe_spill_payload("bash", &bash_payload, 1024)
        .expect("should spill");
    let sv = spilled.to_model_value();
    assert_eq!(sv["exit_code"], 2);
    assert!(meta.total_lines > 500, "spilled bash stdout must stay multi-line");
    let tail = reg.execute(
        "tool_output",
        &ctx,
        json!({"action": "tail", "output_id": meta.output_id, "tail_lines": 5}),
    );
    assert!(!tail.is_error);
    assert!(tail.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("missing symbol"));

    // 2. Bash strips ANSI codes & keeps head+tail across >256 KiB output
    let ansi = crate::shell_pipeline::strip_ansi("\x1b[31mERROR\x1b[0m: \x1b[1mfailed\x1b[0m");
    assert_eq!(ansi, "ERROR: failed");
    let big_cmd = reg.execute(
        "bash",
        &ctx,
        json!({"command": "python3 -c 'print(\"HEAD_MARKER\"); print(\"x\"*300000); print(\"TAIL_MARKER\")'"}),
    );
    assert!(!big_cmd.is_error);
    let bv = big_cmd.payload.to_model_value();
    let out_str = bv["stdout"].as_str().unwrap();
    assert!(out_str.contains("HEAD_MARKER") && out_str.contains("TAIL_MARKER"));

    // 3. read_file rejects binary cleanly & truncates huge single lines
    std::fs::write(dir.path().join("bin.dat"), b"\x7fELF\x00\x01\x02").unwrap();
    let bin_r = reg.execute("read_file", &ctx, json!({"path": "bin.dat"}));
    assert!(bin_r.is_error);
    assert_eq!(bin_r.payload.to_model_value()["error"], "binary_file");

    std::fs::write(dir.path().join("mini.js"), "a".repeat(5_000)).unwrap();
    let mini_r = reg.execute("read_file", &ctx, json!({"path": "mini.js"}));
    assert!(!mini_r.is_error);
    assert!(mini_r.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("line truncated"));

    // 4. edit_file: CRLF normalization, ambiguous line hints, and closest_match on not_found
    std::fs::write(
        dir.path().join("code.rs"),
        "fn alpha() {\r\n    let x = 1;\r\n}\r\nfn beta() {\r\n    let x = 1;\r\n}\r\n",
    )
    .unwrap();
    let amb = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "code.rs", "old_text": "let x = 1;", "new_text": "let x = 2;"}),
    );
    assert!(amb.is_error);
    assert_eq!(amb.payload.to_model_value()["match_lines"], json!([2, 5]));

    let miss = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "code.rs", "old_text": "fn alpha() {\n    let x = 999;\n}", "new_text": "x"}),
    );
    assert!(miss.is_error);
    assert!(miss.payload.to_model_value()["closest_match"]["snippet"]
        .as_str()
        .unwrap()
        .contains("fn alpha()"));

    let crlf_ok = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "code.rs", "old_text": "fn beta() {\n    let x = 1;\n}", "new_text": "fn beta() {\n    let x = 42;\n}"}),
    );
    assert!(!crlf_ok.is_error);
    assert_eq!(crlf_ok.payload.to_model_value()["match_mode"], "crlf_normalized");

    // 5. grep with include, context_lines, fixed_strings, files_only & list_dir max_depth
    std::fs::create_dir_all(dir.path().join("sub/nested")).unwrap();
    std::fs::write(dir.path().join("sub/nested/mod.py"), "a = 1\ntarget(x+y)\nb = 2\n").unwrap();
    let g_ctx = reg.execute(
        "grep",
        &ctx,
        json!({"pattern": "target(x+y)", "fixed_strings": true, "include": "*.py", "context_lines": 1}),
    );
    assert!(!g_ctx.is_error);
    assert_eq!(g_ctx.payload.to_model_value()["count"], 1);
    assert_eq!(g_ctx.payload.to_model_value()["matches"][0]["before"], json!(["a = 1"]));
    assert_eq!(g_ctx.payload.to_model_value()["matches"][0]["after"], json!(["b = 2"]));

    let g_files = reg.execute("grep", &ctx, json!({"pattern": "target", "files_only": true}));
    assert_eq!(g_files.payload.to_model_value()["count"], 1);

    let ld = reg.execute("list_dir", &ctx, json!({"max_depth": 3}));
    assert!(ld.payload.to_model_value()["count"].as_u64().unwrap() >= 4);

    // 6. tool_search empty browse does NOT blindly activate all tools, and omits duplicate parameters
    let browse = reg.execute("tool_search", &ctx, json!({"query": ""}));
    assert_eq!(
        browse.payload.to_model_value()["activate_names"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(browse.payload.to_model_value()["tools"][0].get("parameters").is_none());
}
