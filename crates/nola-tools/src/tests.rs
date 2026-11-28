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
    // Must read_file first before edit_file!
    let unread_edit = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "code.rs", "old_text": "let x = 1;", "new_text": "let x = 2;"}),
    );
    assert!(unread_edit.is_error);
    assert_eq!(unread_edit.payload.to_model_value()["error"], "read_required");
    assert!(!reg.execute("read_file", &ctx, json!({"path": "code.rs"})).is_error);

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

#[test]
fn lsp_security_protocol_and_no_silent_fallback() {
    use std::io::Cursor;

    let reg = ToolRegistry::with_builtin_tools();
    let lsp_spec = reg.get("lsp").unwrap().spec();
    assert_eq!(lsp_spec.domain, nola_types::CapabilityDomain::Process);
    assert!(!lsp_spec.read_only);
    let actions = lsp_spec.schema.parameters["properties"]["action"]["enum"]
        .as_array()
        .unwrap();
    assert!(actions.iter().any(|v| v == "implementation"));

    // Reverse server request policy: allow ONLY workspace/configuration, reject applyEdit/executeCommand/unknown
    let cfg_reply = crate::lsp::build_reverse_request_reply(json!(1), "workspace/configuration");
    assert_eq!(cfg_reply["result"], json!([]));
    assert!(cfg_reply.get("error").is_none());

    for forbidden in [
        "workspace/applyEdit",
        "workspace/executeCommand",
        "window/showDocument",
        "unknown/customMethod",
    ] {
        let reply = crate::lsp::build_reverse_request_reply(json!(7), forbidden);
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(
            reply["error"]["message"],
            "client does not allow server-initiated edits or commands"
        );
    }

    // Frame validation: duplicate Content-Length rejected, >8 MiB frame rejected, >8 KiB header rejected
    let dup = b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}";
    let err = crate::lsp::read_lsp_frame(&mut Cursor::new(&dup[..])).unwrap_err();
    assert!(err.contains("duplicate Content-Length"), "got: {err}");

    let too_big = format!(
        "Content-Length: {}\r\n\r\n{{}}",
        crate::lsp::MAX_LSP_FRAME_BYTES + 1
    );
    let err = crate::lsp::read_lsp_frame(&mut Cursor::new(too_big.as_bytes())).unwrap_err();
    assert!(err.contains("invalid lsp Content-Length"), "got: {err}");

    let huge_hdr = format!(
        "X-Pad: {}\r\nContent-Length: 2\r\n\r\n{{}}",
        "a".repeat(crate::lsp::MAX_LSP_HEADER_BYTES + 64)
    );
    let err = crate::lsp::read_lsp_frame(&mut Cursor::new(huge_hdr.as_bytes())).unwrap_err();
    assert!(err.contains("header exceeds"), "got: {err}");

    // Valid frame parses cleanly
    let valid = b"Content-Length: 15\r\n\r\n{\"jsonrpc\":\"2\"}";
    let parsed = crate::lsp::read_lsp_frame(&mut Cursor::new(&valid[..])).unwrap();
    assert_eq!(parsed["jsonrpc"], "2");

    // No silent fallback when LSP server unavailable (e.g. .json or missing LSP binary)
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), "{bad json").unwrap();
    let ctx = ToolContext::new(dir.path());
    let res = reg.execute(
        "lsp",
        &ctx,
        json!({"action": "diagnostics", "path": "broken.json"}),
    );
    assert!(res.is_error, "LSP unavailability must return error, not silent fallback");
    assert_eq!(res.payload.to_model_value()["error"], "lsp_unavailable");
}

#[test]
fn web_fetch_and_web_search_reject_undeclared_parameters_even_under_allow_all() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.permission_profile = nola_types::PermissionProfile::AllowAll;
    let reg = ToolRegistry::with_builtin_tools();

    // Schemas must declare additionalProperties: false
    let wf_spec = reg.get("web_fetch").unwrap().spec();
    assert_eq!(wf_spec.schema.parameters["additionalProperties"], json!(false));
    let ws_spec = reg.get("web_search").unwrap().spec();
    assert_eq!(ws_spec.schema.parameters["additionalProperties"], json!(false));

    for bad_key in ["method", "body", "files", "headers", "data", "form", "upload"] {
        let mut args = json!({"url": "https://example.com"});
        args[bad_key] = json!("exfiltrated");
        let res = reg.execute("web_fetch", &ctx, args);
        assert!(res.is_error, "web_fetch must reject undeclared key '{bad_key}'");
        assert_eq!(
            res.payload.to_model_value()["error"],
            "undeclared_parameter_rejected"
        );

        let mut s_args = json!({"query": "rust"});
        s_args[bad_key] = json!("exfiltrated");
        let s_res = reg.execute("web_search", &ctx, s_args);
        assert!(s_res.is_error, "web_search must reject undeclared key '{bad_key}'");
        assert_eq!(
            s_res.payload.to_model_value()["error"],
            "undeclared_parameter_rejected"
        );
    }
}

#[test]
fn output_store_session_isolation_utf8_byte_slicing_and_spill_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(crate::OutputStore::new(dir.path().join("outputs")));
    let reg = ToolRegistry::with_builtin_tools();

    let mut ctx_a = ToolContext::new(dir.path()).with_session_id("session-alpha");
    ctx_a.output_store = std::sync::Arc::clone(&store);
    let mut ctx_b = ToolContext::new(dir.path()).with_session_id("session-beta");
    ctx_b.output_store = std::sync::Arc::clone(&store);

    // Multi-byte UTF-8 text: 4 CJK chars (3 bytes each = 12 bytes) + emoji (4 bytes) = 16 bytes
    let text = "你好世界🚀";
    let meta = store.save_scoped("session-alpha", "bash", text).unwrap();
    assert!(meta.path.exists());

    // Cross-session read MUST fail!
    assert!(store.read_bytes_scoped("session-beta", &meta.output_id, 0, 64).is_err());
    let cross = reg.execute(
        "tool_output",
        &ctx_b,
        json!({"action": "slice", "output_id": meta.output_id, "offset": 0, "max_bytes": 64}),
    );
    assert!(cross.is_error, "session-beta must NOT read session-alpha output");

    // UTF-8 char-boundary slicing across multiple calls reassembles exact original string
    let mut offset = 0usize;
    let mut reconstructed = String::new();
    loop {
        let s = reg.execute(
            "tool_output",
            &ctx_a,
            json!({"action": "slice", "output_id": meta.output_id, "offset": offset, "max_bytes": 5}),
        );
        assert!(!s.is_error, "slice failed: {:?}", s.payload);
        let v = s.payload.to_model_value();
        let chunk = v["content"].as_str().unwrap();
        assert_eq!(v["bytes_returned"].as_u64().unwrap() as usize, chunk.len());
        assert_eq!(v["size_bytes"].as_u64().unwrap() as usize, text.len());
        reconstructed.push_str(chunk);
        offset = v["next_offset"].as_u64().unwrap() as usize;
        if !v["has_more"].as_bool().unwrap() {
            break;
        }
    }
    assert_eq!(reconstructed, text);

    // Spill failure explicitly marks full_output_available: false instead of silently returning None
    let blocker_file = dir.path().join("not_a_dir");
    std::fs::write(&blocker_file, b"file").unwrap();
    let bad_store = crate::OutputStore::new(blocker_file);
    let big = ToolModelPayload::text("x".repeat(5_000));
    let (notice, _meta) = bad_store
        .maybe_spill_payload("bash", &big, 512)
        .expect("spill failure must still return truncation notice");
    let nv = notice.to_model_value();
    assert_eq!(nv["full_output_available"], json!(false));
    assert_eq!(nv["spilled"], json!(false));
}

#[test]
fn write_file_expected_sha256_dry_run_mtime_and_read_file_byte_range() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // Create initial file
    let w1 = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "doc.txt", "content": "你好世界\nline2\n"}),
    );
    assert!(!w1.is_error);
    let file_path = dir.path().join("doc.txt");
    let orig_mtime = std::fs::metadata(&file_path).unwrap().modified().unwrap();
    let actual_sha = nola_types::sha256_hex("你好世界\nline2\n".as_bytes());

    // Sleep briefly, write identical content -> mtime MUST be preserved!
    std::thread::sleep(std::time::Duration::from_millis(30));
    let w_same = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "doc.txt", "content": "你好世界\nline2\n"}),
    );
    assert!(!w_same.is_error);
    assert!(!w_same.has_changed_version());
    let after_mtime = std::fs::metadata(&file_path).unwrap().modified().unwrap();
    assert_eq!(orig_mtime, after_mtime, "identical write_file must preserve mtime");

    // Wrong expected_sha256 -> existing_file_version_mismatch
    let w_bad = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "doc.txt", "content": "updated", "expected_sha256": "0000deadbeef"}),
    );
    assert!(w_bad.is_error);
    assert_eq!(
        w_bad.payload.to_model_value()["error"],
        "existing_file_version_mismatch"
    );

    // dry_run=true with matching expected_sha256 -> succeeds without mutating file
    let w_dry = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "doc.txt", "content": "dry content", "expected_sha256": actual_sha, "dry_run": true}),
    );
    assert!(!w_dry.is_error);
    assert_eq!(w_dry.payload.to_model_value()["dry_run"], json!(true));
    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "你好世界\nline2\n"
    );

    // read_file byte-range reading (UTF-8 safe)
    let rb = reg.execute(
        "read_file",
        &ctx,
        json!({"path": "doc.txt", "start_byte": 0, "max_bytes": 7}),
    );
    assert!(!rb.is_error);
    let rbv = rb.payload.to_model_value();
    assert_eq!(rbv["content"], "你好");
    assert_eq!(rbv["next_start_byte"], 6);
    assert_eq!(rbv["truncated"], true);
}

#[test]
fn move_file_overwrite_false_default_and_shell_full_output_preservation() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path()).with_session_id("sess-move-shell");
    ctx.sandbox_mode = SandboxMode::Disabled;
    let reg = ToolRegistry::with_builtin_tools();

    std::fs::write(dir.path().join("a.txt"), "from_a").unwrap();
    std::fs::write(dir.path().join("b.txt"), "existing_b").unwrap();

    // Default overwrite=false rejects clobbering existing destination
    let mv_blocked = reg.execute(
        "move_file",
        &ctx,
        json!({"source": "a.txt", "destination": "b.txt"}),
    );
    assert!(mv_blocked.is_error);
    assert_eq!(
        mv_blocked.payload.to_model_value()["error"],
        "destination_exists"
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("b.txt")).unwrap(), "existing_b");

    // overwrite=true succeeds
    let mv_ok = reg.execute(
        "move_file",
        &ctx,
        json!({"source": "a.txt", "destination": "b.txt", "overwrite": true}),
    );
    assert!(!mv_ok.is_error);
    assert_eq!(std::fs::read_to_string(dir.path().join("b.txt")).unwrap(), "from_a");

    // Bash >256 KiB output preserves middle bytes via session-scoped OutputStore
    let big_cmd = reg.execute(
        "bash",
        &ctx,
        json!({"command": "python3 -c 'print(\"H\"); print(\"a\"*140000); print(\"MIDDLE_SECRET_TOKEN\"); print(\"b\"*140000); print(\"T\")'"}),
    );
    assert!(!big_cmd.is_error);
    let bv = big_cmd.payload.to_model_value();
    assert_eq!(bv["full_output_available"], json!(true));
    let full_id = bv["full_output_id"].as_str().expect("expected full_output_id");
    let found = reg.execute(
        "tool_output",
        &ctx,
        json!({"action": "search", "output_id": full_id, "pattern": "MIDDLE_SECRET_TOKEN"}),
    );
    assert!(!found.is_error);
    assert_eq!(found.payload.to_model_value()["count"], 1);

    // Timeout reports local_stop: true
    let to = reg.execute(
        "bash",
        &ctx,
        json!({"command": "sleep 5", "timeout_secs": 1}),
    );
    assert!(to.is_error);
    assert_eq!(to.payload.to_model_value()["local_stop"], json!(true));
}

#[cfg(unix)]
#[test]
fn pty_resize_and_non_destructive_offset_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Disabled;
    let reg = ToolRegistry::with_builtin_tools();

    let pty = reg.execute(
        "pty_exec",
        &ctx,
        json!({"action": "exec", "command": "cat", "cols": 80, "rows": 24, "yield_ms": 100}),
    );
    assert!(!pty.is_error, "pty exec failed: {:?}", pty.payload);
    let sid = pty.payload.to_model_value()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Resize PTY
    let rs = reg.execute(
        "pty_exec",
        &ctx,
        json!({"action": "resize", "session_id": sid, "cols": 132, "rows": 40}),
    );
    assert!(!rs.is_error, "pty resize failed: {:?}", rs.payload);
    assert_eq!(rs.payload.to_model_value()["resized"], json!(true));
    assert_eq!(rs.payload.to_model_value()["cols"], json!(132));
    assert_eq!(rs.payload.to_model_value()["rows"], json!(40));

    // Write output and read twice from offset=0 non-destructively
    let wr = reg.execute(
        "pty_exec",
        &ctx,
        json!({"action": "write", "session_id": sid, "input": "alpha_marker\n", "yield_ms": 150}),
    );
    assert!(!wr.is_error);

    let r1 = reg.execute(
        "pty_exec",
        &ctx,
        json!({"action": "read", "session_id": sid, "offset": 0, "max_bytes": 4096}),
    );
    let r2 = reg.execute(
        "pty_exec",
        &ctx,
        json!({"action": "read", "session_id": sid, "offset": 0, "max_bytes": 4096}),
    );
    assert!(!r1.is_error && !r2.is_error);
    let c1 = r1.payload.to_model_value()["content"].as_str().unwrap().to_string();
    let c2 = r2.payload.to_model_value()["content"].as_str().unwrap().to_string();
    assert!(c1.contains("alpha_marker"));
    assert_eq!(c1, c2, "offset reads must be non-destructive");

    let _ = reg.execute("pty_exec", &ctx, json!({"action": "close", "session_id": sid}));
}

#[test]
fn browser_attached_target_routing_and_env_scrubbing() {
    let dir = tempfile::tempdir().unwrap();
    let shared_bridge = std::sync::Arc::new(crate::NativeBrowserBridge::default());
    let mut ctx = ToolContext::new(dir.path());
    ctx.browser = std::sync::Arc::new(crate::BrowserManager::with_native_bridge(
        std::sync::Arc::clone(&shared_bridge),
    ));
    let reg = ToolRegistry::with_builtin_tools();

    // Env scrubbing classifier
    assert!(crate::browser::is_sensitive_env_key("OPENAI_API_KEY"));
    assert!(crate::browser::is_sensitive_env_key("ANTHROPIC_API_KEY"));
    assert!(crate::browser::is_sensitive_env_key("AWS_SECRET_ACCESS_KEY"));
    assert!(crate::browser::is_sensitive_env_key("GITHUB_TOKEN"));
    assert!(crate::browser::is_sensitive_env_key("MY_APP_SECRET"));
    assert!(!crate::browser::is_sensitive_env_key("PATH"));

    // Spawn mock extension host polling the EXACT shared bridge
    let host_bridge = std::sync::Arc::clone(&shared_bridge);
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Some(cmd) = host_bridge.poll_next() {
                let id = cmd["command_id"].as_str().unwrap().to_string();
                let action = cmd["action"].as_str().unwrap().to_string();
                host_bridge.submit_result(
                    id,
                    json!({"ok": true, "routed_action": action, "target": "attached"}),
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    let res = reg.execute(
        "browser",
        &ctx,
        json!({"action": "snapshot", "target": "attached", "timeout_ms": 2000}),
    );
    handle.join().unwrap();
    assert!(!res.is_error, "attached browser call failed: {:?}", res.payload);
    assert_eq!(res.payload.to_model_value()["routed_action"], "snapshot");
}


#[cfg(target_os = "macos")]
#[test]
fn macos_seatbelt_sandbox_isolates_writes_and_network() {
    assert!(
        crate::probe_seatbelt_available(),
        "expected /usr/bin/sandbox-exec to be available on macOS"
    );

    let dir = tempfile::tempdir().unwrap();
    let git_hooks = dir.path().join(".git/hooks");
    std::fs::create_dir_all(&git_hooks).unwrap();
    std::fs::write(git_hooks.join("pre-commit"), "#!/bin/sh\n").unwrap();

    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Required;
    ctx.permission_profile = nola_types::PermissionProfile::Cautious;
    ctx.network_allowed = false;
    let reg = ToolRegistry::with_builtin_tools();

    // Writing inside workspace succeeds under Seatbelt
    let ok = reg.execute(
        "bash",
        &ctx,
        json!({"command": "echo sandbox_ok > inside.txt && cat inside.txt"}),
    );
    assert!(!ok.is_error, "seatbelt write inside workspace failed: {:?}", ok.payload);
    assert_eq!(ok.payload.to_model_value()["exit_code"], 0);
    assert!(ok.payload.to_model_value()["stdout"]
        .as_str()
        .unwrap()
        .contains("sandbox_ok"));

    // Writing to protected .git/hooks fails under Seatbelt
    let hook_blocked = reg.execute(
        "bash",
        &ctx,
        json!({"command": "echo pwn >> .git/hooks/pre-commit"}),
    );
    assert_ne!(
        hook_blocked.payload.to_model_value()["exit_code"],
        0,
        "seatbelt must block writes to .git/hooks"
    );

    // Writing outside workspace and outside /tmp/TMPDIR (e.g. /var/tmp) is denied
    let outside_probe = format!("/var/tmp/nola_seatbelt_denied_{}", nola_types::new_id());
    let out_blocked = reg.execute(
        "bash",
        &ctx,
        json!({"command": format!("echo nope > {outside_probe}")}),
    );
    assert_ne!(
        out_blocked.payload.to_model_value()["exit_code"],
        0,
        "seatbelt must block writes outside workspace and tmp"
    );
    assert!(!std::path::Path::new(&outside_probe).exists());

    // Network denied when network_allowed == false
    let net_blocked = reg.execute(
        "bash",
        &ctx,
        json!({"command": "python3 -c 'import socket; s = socket.socket(); s.bind((\"127.0.0.1\", 0))'"}),
    );
    assert_ne!(
        net_blocked.payload.to_model_value()["exit_code"],
        0,
        "seatbelt must deny network when network_allowed=false"
    );
}

#[test]
fn file_continuity_guards_multi_edit_overlap_append_outline_and_persistent_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // 1. write_file includes (sha256: <hex>) in summary and grants Full coverage for immediate edit/patch
    let w1 = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "authored.txt", "content": "line1\nline2\nline3\n"}),
    );
    assert!(!w1.is_error);
    let sha1 = nola_types::sha256_hex(b"line1\nline2\nline3\n");
    assert!(w1.payload.ui_summary().contains(&format!("sha256: {sha1}")));

    // Immediate edit_file without read_file succeeds because write_file granted Full coverage
    let e_after_w = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "authored.txt", "old_text": "line2", "new_text": "LINE_TWO"}),
    );
    assert!(!e_after_w.is_error, "authored file edit failed: {:?}", e_after_w.payload);

    // 2. Blind overwrite of unread pre-existing file fails with existing_file_version_required
    std::fs::write(dir.path().join("pre.txt"), "original pre content\n").unwrap();
    let pre_sha = nola_types::sha256_hex(b"original pre content\n");
    let blind = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "pre.txt", "content": "clobbered\n"}),
    );
    assert!(blind.is_error);
    assert_eq!(
        blind.payload.to_model_value()["error"],
        "existing_file_version_required"
    );
    assert_eq!(blind.payload.to_model_value()["actual_sha256"], pre_sha);

    // Passing matching expected_sha256 succeeds
    let with_sha = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "pre.txt", "content": "updated safely\n", "expected_sha256": pre_sha}),
    );
    assert!(!with_sha.is_error);

    // External modification after write causes edit_file AND write_file to fail with file_modified_externally
    std::fs::write(dir.path().join("pre.txt"), "external change\n").unwrap();
    let stale_edit = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "pre.txt", "old_text": "external", "new_text": "mine"}),
    );
    assert!(stale_edit.is_error);
    assert_eq!(
        stale_edit.payload.to_model_value()["error"],
        "file_modified_externally"
    );
    let stale_write = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "pre.txt", "content": "overwrite stale\n"}),
    );
    assert!(stale_write.is_error);
    assert_eq!(
        stale_write.payload.to_model_value()["error"],
        "file_modified_externally"
    );

    // 3. apply_patch checks per-hunk old-line range coverage
    let numbered_lines: String = (1..=15).map(|i| format!("row_{i}\n")).collect();
    std::fs::write(dir.path().join("rows.txt"), &numbered_lines).unwrap();
    // Read only lines 1..=5
    let r_part = reg.execute(
        "read_file",
        &ctx,
        json!({"path": "rows.txt", "start_line": 1, "end_line": 5}),
    );
    assert!(!r_part.is_error);
    // Patching line 12 MUST fail with unread_patch_range
    let patch_far = "--- a/rows.txt\n+++ b/rows.txt\n@@ -11,3 +11,3 @@\n row_11\n-row_12\n+ROW_TWELVE\n row_13\n";
    let p_fail = reg.execute("apply_patch", &ctx, json!({"patch": patch_far}));
    assert!(p_fail.is_error);
    assert_eq!(
        p_fail.payload.to_model_value()["error"],
        "unread_patch_range"
    );
    // Read lines 10..=15 -> now patching line 12 succeeds!
    assert!(!reg
        .execute(
            "read_file",
            &ctx,
            json!({"path": "rows.txt", "start_line": 10, "end_line": 15}),
        )
        .is_error);
    let p_ok = reg.execute("apply_patch", &ctx, json!({"patch": patch_far}));
    assert!(!p_ok.is_error, "expected patch to succeed after reading range: {:?}", p_ok.payload);
    assert!(std::fs::read_to_string(dir.path().join("rows.txt"))
        .unwrap()
        .contains("ROW_TWELVE"));

    // 4. multi_edit_file rejects overlapping edits against original snapshot without mutating
    let _ = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "multi.txt", "content": "alpha beta gamma\n"}),
    );
    let ov = reg.execute(
        "multi_edit_file",
        &ctx,
        json!({
            "path": "multi.txt",
            "edits": [
                {"old_text": "alpha beta", "new_text": "A B"},
                {"old_text": "beta gamma", "new_text": "B G"}
            ]
        }),
    );
    assert!(ov.is_error);
    assert_eq!(ov.payload.to_model_value()["error"], "overlapping_edits");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("multi.txt")).unwrap(),
        "alpha beta gamma\n"
    );

    // 5. append_file & file_outline
    let app = reg.execute(
        "append_file",
        &ctx,
        json!({"path": "notes.md", "content": "# Title\n## Section A\nBody\n"}),
    );
    assert!(!app.is_error && app.has_changed_version());
    let app2 = reg.execute(
        "append_file",
        &ctx,
        json!({"path": "notes.md", "content": "### Sub", "ensure_newline": true}),
    );
    assert!(!app2.is_error);
    let outl = reg.execute("file_outline", &ctx, json!({"path": "notes.md"}));
    assert!(!outl.is_error);
    assert_eq!(outl.payload.to_model_value()["count"], 3);
    assert_eq!(outl.payload.to_model_value()["entries"][0]["kind"], "h1");

    // 6. Persistent CheckpointManager: survives restart, diff, restore preserves unrelated files, undo & redo!
    std::fs::write(dir.path().join("tracked.txt"), "v1\n").unwrap();
    let cp1 = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "create", "label": "baseline"}),
    );
    assert!(!cp1.is_error);
    let cp1_id = cp1.payload.to_model_value()["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(dir
        .path()
        .join(".nola/checkpoints")
        .join(&cp1_id)
        .join("manifest.json")
        .is_file());

    // Edit tracked.txt AND create an unrelated file outside checkpoint
    std::fs::write(dir.path().join("tracked.txt"), "v2\n").unwrap();
    std::fs::write(dir.path().join("unrelated_user_file.txt"), "keep me!\n").unwrap();

    // Simulate process restart with a brand-new ToolContext / CheckpointManager!
    let restarted_ctx = ToolContext::new(dir.path());
    let listed = reg.execute("workspace_checkpoint", &restarted_ctx, json!({"action": "list"}));
    assert!(!listed.is_error);
    assert_eq!(listed.payload.to_model_value()["count"], 1);

    let diff = reg.execute(
        "workspace_checkpoint",
        &restarted_ctx,
        json!({"action": "diff", "checkpoint_id": cp1_id}),
    );
    assert!(!diff.is_error);
    assert_eq!(
        diff.payload.to_model_value()["modified"][0]["path"],
        "tracked.txt"
    );

    // Restore cp1 -> tracked.txt returns to v1, unrelated_user_file.txt is NOT deleted!
    let restored = reg.execute(
        "workspace_checkpoint",
        &restarted_ctx,
        json!({"action": "restore", "checkpoint_id": cp1_id}),
    );
    assert!(!restored.is_error);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "v1\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("unrelated_user_file.txt")).unwrap(),
        "keep me!\n"
    );

    // Undo reverts restore back to v2; Redo re-applies v1!
    let undone = reg.execute("workspace_checkpoint", &restarted_ctx, json!({"action": "undo"}));
    assert!(!undone.is_error, "undo failed: {:?}", undone.payload);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "v2\n"
    );
    let redone = reg.execute("workspace_checkpoint", &restarted_ctx, json!({"action": "redo"}));
    assert!(!redone.is_error, "redo failed: {:?}", redone.payload);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "v1\n"
    );
}

#[test]
fn git_worktree_integrate_and_blame_range() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let run = |cwd: &std::path::Path, args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(dir.path(), &["init", "-b", "main"]);
    run(dir.path(), &["config", "user.name", "Nola Test"]);
    run(dir.path(), &["config", "user.email", "test@nola.local"]);
    std::fs::write(dir.path().join("file.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    run(dir.path(), &["add", "."]);
    run(dir.path(), &["commit", "-m", "initial"]);

    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // Custom blame line range
    let bl = reg.execute(
        "git",
        &ctx,
        json!({"action": "blame", "target": "file.txt", "start_line": 2, "end_line": 4}),
    );
    assert!(!bl.is_error, "blame failed: {:?}", bl.payload);
    let bl_out = bl.payload.to_model_value()["output"].as_str().unwrap().to_string();
    assert_eq!(bl_out.lines().count(), 3);

    // Create worktree + commit clean non-conflicting change -> integrate cleanly
    let wt_rel = "wt_clean";
    let wt_create = reg.execute(
        "git",
        &ctx,
        json!({"action": "worktree_create", "target": wt_rel, "branch": "feat-clean"}),
    );
    assert!(!wt_create.is_error, "wt create failed: {:?}", wt_create.payload);
    let wt_path = dir.path().join(wt_rel);
    std::fs::write(wt_path.join("feature.txt"), "new feature\n").unwrap();
    run(&wt_path, &["add", "feature.txt"]);
    run(&wt_path, &["commit", "-m", "add feature"]);

    let integ = reg.execute(
        "git",
        &ctx,
        json!({"action": "worktree_integrate", "target": wt_rel, "strategy": "merge"}),
    );
    assert!(!integ.is_error, "worktree_integrate failed: {:?}", integ.payload);
    assert!(dir.path().join("feature.txt").is_file());
    assert!(integ.has_changed_version());

    // Conflicting worktree integration MUST refuse and cleanly roll back!
    let wt_conflict_rel = "wt_conflict";
    assert!(!reg
        .execute(
            "git",
            &ctx,
            json!({"action": "worktree_create", "target": wt_conflict_rel, "branch": "feat-conflict"}),
        )
        .is_error);
    let wt_c_path = dir.path().join(wt_conflict_rel);
    std::fs::write(wt_c_path.join("file.txt"), "conflict from worktree\n").unwrap();
    run(&wt_c_path, &["add", "file.txt"]);
    run(&wt_c_path, &["commit", "-m", "wt conflict"]);

    // Advance main with conflicting edit on same line
    std::fs::write(dir.path().join("file.txt"), "conflict from main\n").unwrap();
    run(dir.path(), &["add", "file.txt"]);
    run(dir.path(), &["commit", "-m", "main conflict"]);

    let conflict_res = reg.execute(
        "git",
        &ctx,
        json!({"action": "worktree_integrate", "target": wt_conflict_rel, "strategy": "merge"}),
    );
    assert!(conflict_res.is_error);
    let cv = conflict_res.payload.to_model_value();
    assert_eq!(cv["error"], "integration_conflict");
    assert_eq!(cv["rolled_back"], json!(true));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
        "conflict from main\n"
    );
}

#[test]
fn binary_docs_magic_byte_sniffing_docx_and_attachment_extraction() {
    use std::io::Write;

    // Build a real in-memory .docx ZIP with paragraphs + a table
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("word/document.xml", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello &amp; Welcome</w:t></w:r></w:p>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>Col A</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>Col B</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
  </w:body>
</w:document>"#,
        )
        .unwrap();
        zw.finish().unwrap();
    }
    let docx_bytes = buf.into_inner();

    // Magic-byte sniffing recognizes DOCX even with an unrelated filename!
    let extracted = crate::extract_document_text_from_bytes("mystery.bin", "", &docx_bytes)
        .expect("expected DOCX extraction via magic bytes");
    assert!(extracted.contains("Hello & Welcome"), "got: {extracted}");
    assert!(extracted.contains("Col A | Col B"), "got: {extracted}");

    // read_file reads .docx (and even .bin with DOCX magic bytes!) cleanly
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.bin"), &docx_bytes).unwrap();
    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();
    let r = reg.execute("read_file", &ctx, json!({"path": "report.bin"}));
    assert!(!r.is_error, "docx read_file failed: {:?}", r.payload);
    assert_eq!(r.payload.to_model_value()["format"], "docx");
    assert!(r.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("Col A | Col B"));

    // UTF-8 text extraction works; PNG magic bytes return None
    assert_eq!(
        crate::extract_document_text_from_bytes("note.txt", "text/plain", b"plain text"),
        Some("plain text".to_string())
    );
    assert_eq!(
        crate::extract_document_text_from_bytes("pic.dat", "", b"\x89PNG\r\n\x1a\n0000"),
        None
    );
}

#[test]
fn code_mode_tools_proxy_all_tools_cap_and_lsp_browser_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let reg = ToolRegistry::with_builtin_tools();

    // 1. code_exec with host dispatcher exposes frozen ALL_TOOLS + tools.<name>(args)
    let ctx = ToolContext::new(dir.path()).with_code_mode_dispatcher(
        std::sync::Arc::new(|name, args| {
            if name == "echo_tool" {
                Ok(json!({"echoed": args["val"]}))
            } else {
                Err(format!("unknown host tool {name}"))
            }
        }),
        vec!["echo_tool".into(), "read_file".into()],
    );

    let ok = reg.execute(
        "code_exec",
        &ctx,
        json!({
            "code": "const frozen = Object.isFrozen(ALL_TOOLS); const r = tools.echo_tool({val: 21}); return {frozen, list: ALL_TOOLS, doubled: r.echoed * 2};"
        }),
    );
    assert!(!ok.is_error, "code_exec tools proxy failed: {:?}", ok.payload);
    let ov = ok.payload.to_model_value();
    assert_eq!(ov["result"]["frozen"], json!(true));
    assert_eq!(ov["result"]["doubled"], json!(42));
    assert_eq!(ov["tool_calls_used"], json!(1));

    // Recursive code_exec call is blocked
    let rec = reg.execute(
        "code_exec",
        &ctx,
        json!({"code": "return tools.code_exec({code: 'return 1;'});"}),
    );
    assert!(rec.is_error);

    // Bounded max tool calls (24) is enforced
    let spam = reg.execute(
        "code_exec",
        &ctx,
        json!({"code": "for (let i = 0; i < 30; i++) { tools.echo_tool({val: i}); } return true;"}),
    );
    assert!(spam.is_error);
    assert!(spam
        .payload
        .to_model_value()["message"]
        .as_str()
        .unwrap_or("")
        .contains("24"));

    // 2. LSP list_servers includes Swift (sourcekit-lsp) and Kotlin (kotlin-language-server)
    let lsp_list = reg.execute("lsp", &ctx, json!({"action": "list_servers"}));
    assert!(!lsp_list.is_error, "lsp list_servers failed: {:?}", lsp_list.payload);
    let servers = lsp_list.payload.to_model_value()["servers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(servers.iter().any(|s| s["bin"] == "sourcekit-lsp"));
    assert!(servers.iter().any(|s| s["bin"] == "kotlin-language-server"));

    // collect_post_edit_diagnostics returns None gracefully for unsupported extension
    assert!(crate::collect_post_edit_diagnostics(&ctx, &dir.path().join("foo.unknown")).is_none());

    // 3. Browser schema includes all extended CDP actions
    let browser_spec = reg.get("browser").unwrap().spec();
    let actions = browser_spec.schema.parameters["properties"]["action"]["enum"]
        .as_array()
        .unwrap();
    for expected_action in [
        "switch_page",
        "hover",
        "drag",
        "select_option",
        "set_checked",
        "wait_for",
        "accessibility_snapshot",
        "console_logs",
        "network_events",
    ] {
        assert!(
            actions.iter().any(|v| v == expected_action),
            "browser schema missing action '{expected_action}'"
        );
    }
    // should_pass_no_sandbox helper compiles and runs cleanly
    let _ = crate::browser::should_pass_no_sandbox();
}

/// Build a valid, standards-compliant multi-page PDF 1.4 byte buffer with Helvetica text streams
/// on each page and exact byte offsets in the xref table (parsed cleanly by `lopdf`).
fn build_multipage_pdf(pages: &[Vec<String>]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");

    let num_pages = pages.len();
    let total_objects = 3 + num_pages * 2;
    let mut offsets: Vec<usize> = vec![0; total_objects + 1];

    // Object 1: Catalog
    offsets[1] = out.len();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    let kids: Vec<String> = (0..num_pages)
        .map(|i| format!("{} 0 R", 4 + i * 2))
        .collect();
    offsets[2] = out.len();
    out.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
            kids.join(" "),
            num_pages
        )
        .as_bytes(),
    );

    // Object 3: Helvetica Type1 Font
    offsets[3] = out.len();
    out.extend_from_slice(
        b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n",
    );

    for (idx, lines) in pages.iter().enumerate() {
        let page_obj = 4 + idx * 2;
        let content_obj = page_obj + 1;

        offsets[page_obj] = out.len();
        out.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_obj} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );

        let mut stream = String::from("BT\n/F1 10 Tf\n40 760 Td\n");
        for (li, line) in lines.iter().enumerate() {
            let escaped = line
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            if li > 0 {
                stream.push_str("0 -12 Td\n");
            }
            stream.push_str(&format!("({escaped}) Tj\n"));
        }
        stream.push_str("ET\n");

        offsets[content_obj] = out.len();
        out.extend_from_slice(
            format!(
                "{content_obj} 0 obj\n<< /Length {} >>\nstream\n{stream}endstream\nendobj\n",
                stream.len()
            )
            .as_bytes(),
        );
    }

    let xref_offset = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total_objects + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            total_objects + 1
        )
        .as_bytes(),
    );
    out
}

fn build_minimal_xlsx_bytes() -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("[Content_Types].xml", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
        )
        .unwrap();
        zw.start_file("_rels/.rels", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
        )
        .unwrap();
        zw.start_file("xl/workbook.xml", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Quarterly" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#,
        )
        .unwrap();
        zw.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
        )
        .unwrap();
        zw.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="inlineStr"><is><t>Region</t></is></c>
      <c r="B1" t="inlineStr"><is><t>Revenue</t></is></c>
    </row>
    <row r="2">
      <c r="A2" t="inlineStr"><is><t>APAC-Q4</t></is></c>
      <c r="B2"><v>42500</v></c>
    </row>
  </sheetData>
</worksheet>"#,
        )
        .unwrap();
        zw.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn real_scenario_multipage_pdf_generation_default_and_explicit_page_extraction() {
    let dir = tempfile::tempdir().unwrap();
    // Generate a 15-page PDF with >36 KiB of text across pages and distinct markers on every page
    let pages: Vec<Vec<String>> = (1..=15)
        .map(|p| {
            let mut lines = vec![format!("PDF_PAGE_{p:02}_MARKER Title Section for Page {p}")];
            for row in 1..=28 {
                lines.push(format!(
                    "Page {p:02} row {row:02}: Detailed specification paragraph explaining Nola v2 subsystem telemetry and checksum validation."
                ));
            }
            lines
        })
        .collect();
    let total_text_bytes: usize = pages
        .iter()
        .flat_map(|p| p.iter())
        .map(|l| l.len())
        .sum();
    assert!(
        total_text_bytes > 32 * 1024,
        "expected >32 KiB total PDF text, got {total_text_bytes}"
    );

    let pdf_bytes = build_multipage_pdf(&pages);
    let pdf_path = dir.path().join("architecture_spec_15p.pdf");
    std::fs::write(&pdf_path, &pdf_bytes).unwrap();

    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // 1. Default read_file extracts pages 1-10 and summary explicitly reports `(pages 1-10 of 15)`
    let def_res = reg.execute("read_file", &ctx, json!({"path": "architecture_spec_15p.pdf"}));
    assert!(!def_res.is_error, "default PDF read failed: {:?}", def_res.payload);
    assert!(
        def_res.payload.ui_summary().contains("(pages 1-10 of 15)"),
        "expected summary to include '(pages 1-10 of 15)', got: {}",
        def_res.payload.ui_summary()
    );
    let def_val = def_res.payload.to_model_value();
    assert_eq!(def_val["format"], "pdf");
    assert_eq!(def_val["total_pages"], 15);
    let extracted = def_val["extracted_pages"].as_array().unwrap();
    assert_eq!(extracted.len(), 10);
    assert_eq!(extracted[0]["page"], 1);
    assert!(extracted[0]["text"].as_str().unwrap().contains("PDF_PAGE_01_MARKER"));
    assert_eq!(extracted[4]["page"], 5);
    assert!(extracted[4]["text"].as_str().unwrap().contains("PDF_PAGE_05_MARKER"));
    assert_eq!(extracted[9]["page"], 10);
    assert!(extracted[9]["text"].as_str().unwrap().contains("PDF_PAGE_10_MARKER"));
    assert!(
        !def_res.payload.to_model_string().contains("PDF_PAGE_15_MARKER"),
        "default read should stop at page 10"
    );

    // 2. Explicit `page: 1` -> summary `(page 1 of 15)` with page 1 marker
    let p1 = reg.execute(
        "read_file",
        &ctx,
        json!({"path": "architecture_spec_15p.pdf", "page": 1}),
    );
    assert!(!p1.is_error, "page 1 read failed: {:?}", p1.payload);
    assert!(
        p1.payload.ui_summary().contains("(page 1 of 15)"),
        "expected '(page 1 of 15)', got: {}",
        p1.payload.ui_summary()
    );
    let p1_pages = p1.payload.to_model_value()["extracted_pages"]
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(p1_pages.len(), 1);
    assert_eq!(p1_pages[0]["page"], 1);
    assert!(p1_pages[0]["text"].as_str().unwrap().contains("PDF_PAGE_01_MARKER"));

    // 3. Explicit `page: 15` -> summary `(page 15 of 15)` with page 15 marker
    let p15 = reg.execute(
        "read_file",
        &ctx,
        json!({"path": "architecture_spec_15p.pdf", "page": 15}),
    );
    assert!(!p15.is_error, "page 15 read failed: {:?}", p15.payload);
    assert!(
        p15.payload.ui_summary().contains("(page 15 of 15)"),
        "expected '(page 15 of 15)', got: {}",
        p15.payload.ui_summary()
    );
    let p15_pages = p15.payload.to_model_value()["extracted_pages"]
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(p15_pages.len(), 1);
    assert_eq!(p15_pages[0]["page"], 15);
    assert!(p15_pages[0]["text"].as_str().unwrap().contains("PDF_PAGE_15_MARKER"));

    // 4. In-memory attachment extraction with disguised extension uses magic bytes to extract all 15 pages
    let full_text = crate::extract_document_text_from_bytes("disguised.bin", "", &pdf_bytes)
        .expect("magic-byte PDF extraction should succeed");
    assert!(full_text.contains("PDF_PAGE_01_MARKER"));
    assert!(full_text.contains("PDF_PAGE_05_MARKER"));
    assert!(full_text.contains("PDF_PAGE_10_MARKER"));
    assert!(full_text.contains("PDF_PAGE_15_MARKER"));
}

#[test]
fn real_scenario_output_store_session_scoped_spill_full_actions_and_cross_session_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(crate::OutputStore::new(dir.path().join("spill_store")));
    let reg = ToolRegistry::with_builtin_tools();

    let mut ctx_a = ToolContext::new(dir.path()).with_session_id("sess-A");
    ctx_a.output_store = std::sync::Arc::clone(&store);
    let mut ctx_b = ToolContext::new(dir.path()).with_session_id("sess-B");
    ctx_b.output_store = std::sync::Arc::clone(&store);

    // Construct >32 KiB payload with distinct line markers & regex hits
    let mut body = String::new();
    for i in 1..=800 {
        let tag = match i {
            1 => "FIRST_ROW_MARKER",
            400 => "MID_ROW_400_ERROR_CODE_404",
            720 => "LATE_ROW_720_ERROR_CODE_503",
            800 => "FINAL_ROW_800_MARKER",
            _ => "ordinary_log_payload_entry_for_spill_verification",
        };
        body.push_str(&format!("entry_{i:04}: {tag}\n"));
    }
    assert!(body.len() > crate::DEFAULT_SPILL_THRESHOLD_BYTES);

    let big_payload = ToolModelPayload::structured(
        json!({"path": "server.log", "content": body, "total_lines": 800}),
        "Read server.log (800 lines)",
    );
    let (compact, meta) = store
        .maybe_spill_payload_scoped(
            "sess-A",
            "read_file",
            &big_payload,
            crate::DEFAULT_SPILL_THRESHOLD_BYTES,
        )
        .expect("payload >24 KiB must spill");
    let cv = compact.to_model_value();
    assert_eq!(cv["spilled"], json!(true));
    assert_eq!(cv["full_output_available"], json!(true));
    assert_eq!(cv["output_id"], json!(meta.output_id));
    assert_eq!(cv["path"], json!("server.log"));

    // sess-A: action=list
    let list_a = reg.execute("tool_output", &ctx_a, json!({"action": "list"}));
    assert!(!list_a.is_error);
    assert_eq!(list_a.payload.to_model_value()["count"], 1);
    assert_eq!(
        list_a.payload.to_model_value()["outputs"][0]["output_id"],
        meta.output_id
    );

    // sess-A: action=read exact line range (note rendered content starts with "=== content ===" on line 1)
    let read_a = reg.execute(
        "tool_output",
        &ctx_a,
        json!({"action": "read", "output_id": meta.output_id, "start_line": 399, "end_line": 405}),
    );
    assert!(!read_a.is_error, "read failed: {:?}", read_a.payload);
    let read_text = read_a.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        read_text.contains("MID_ROW_400_ERROR_CODE_404"),
        "expected line 400 marker in range, got: {read_text}"
    );

    // sess-A: action=tail
    let tail_a = reg.execute(
        "tool_output",
        &ctx_a,
        json!({"action": "tail", "output_id": meta.output_id, "tail_lines": 5}),
    );
    assert!(!tail_a.is_error);
    assert!(tail_a.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("FINAL_ROW_800_MARKER"));

    // sess-A: action=search with regex
    let search_a = reg.execute(
        "tool_output",
        &ctx_a,
        json!({"action": "search", "output_id": meta.output_id, "pattern": "ERROR_CODE_(404|503)"}),
    );
    assert!(!search_a.is_error);
    let sv = search_a.payload.to_model_value();
    assert_eq!(sv["count"], 2);
    assert!(sv["matches"][0]["text"]
        .as_str()
        .unwrap()
        .contains("ERROR_CODE_404"));
    assert!(sv["matches"][1]["text"]
        .as_str()
        .unwrap()
        .contains("ERROR_CODE_503"));

    // sess-A: action=slice with byte offset continuation
    let slice1 = reg.execute(
        "tool_output",
        &ctx_a,
        json!({"action": "slice", "output_id": meta.output_id, "offset": 0, "max_bytes": 256}),
    );
    assert!(!slice1.is_error);
    let s1v = slice1.payload.to_model_value();
    assert_eq!(s1v["has_more"], json!(true));
    assert!(s1v["content"].as_str().unwrap().contains("FIRST_ROW_MARKER"));
    let next_off = s1v["next_offset"].as_u64().unwrap();
    let slice2 = reg.execute(
        "tool_output",
        &ctx_a,
        json!({"action": "slice", "output_id": meta.output_id, "offset": next_off, "max_bytes": 256}),
    );
    assert!(!slice2.is_error);
    assert_eq!(slice2.payload.to_model_value()["offset"], next_off);

    // Cross-session isolation: sess-B MUST NOT see or read sess-A's output!
    let list_b = reg.execute("tool_output", &ctx_b, json!({"action": "list"}));
    assert!(!list_b.is_error);
    assert_eq!(list_b.payload.to_model_value()["count"], 0);

    for (action, extra) in [
        ("read", json!({"start_line": 1, "end_line": 10})),
        ("tail", json!({"tail_lines": 5})),
        ("search", json!({"pattern": "ERROR_CODE"})),
        ("slice", json!({"offset": 0, "max_bytes": 128})),
    ] {
        let mut req = json!({"action": action, "output_id": meta.output_id});
        for (k, v) in extra.as_object().unwrap() {
            req[k] = v.clone();
        }
        let cross = reg.execute("tool_output", &ctx_b, req);
        assert!(
            cross.is_error,
            "sess-B tool_output action={action} must fail on sess-A output_id"
        );
        assert!(
            cross
                .payload
                .to_model_string()
                .contains("unknown output_id"),
            "expected 'unknown output_id' for cross-session action={action}, got: {:?}",
            cross.payload
        );
    }
}

#[test]
fn real_scenario_binary_format_sniffing_docx_xlsx_png_jpeg_pdf_with_disguised_extensions() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // 1. DOCX disguised as `.dat`
    let mut docx_buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut docx_buf);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("word/document.xml", opts).unwrap();
        zw.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Board Memo &amp; Forecast</w:t></w:r></w:p>
    <w:tbl><w:tr>
      <w:tc><w:p><w:r><w:t>Metric</w:t></w:r></w:p></w:tc>
      <w:tc><w:p><w:r><w:t>Target</w:t></w:r></w:p></w:tc>
    </w:tr></w:tbl>
  </w:body>
</w:document>"#,
        )
        .unwrap();
        zw.finish().unwrap();
    }
    let docx_bytes = docx_buf.into_inner();
    std::fs::write(dir.path().join("blob_docx.dat"), &docx_bytes).unwrap();
    let r_docx = reg.execute("read_file", &ctx, json!({"path": "blob_docx.dat"}));
    assert!(!r_docx.is_error, "docx failed: {:?}", r_docx.payload);
    assert_eq!(r_docx.payload.to_model_value()["format"], "docx");
    assert!(r_docx.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("Board Memo & Forecast"));

    // 2. XLSX disguised as `.dat`
    let xlsx_bytes = build_minimal_xlsx_bytes();
    std::fs::write(dir.path().join("blob_xlsx.dat"), &xlsx_bytes).unwrap();
    let r_xlsx = reg.execute("read_file", &ctx, json!({"path": "blob_xlsx.dat"}));
    assert!(!r_xlsx.is_error, "xlsx failed: {:?}", r_xlsx.payload);
    let xv = r_xlsx.payload.to_model_value();
    assert_eq!(xv["format"], "spreadsheet");
    assert_eq!(xv["sheets"][0]["name"], "Quarterly");
    assert_eq!(xv["sheets"][0]["preview_rows"][1][0], "APAC-Q4");
    let xlsx_txt = crate::extract_document_text_from_bytes("blob_xlsx.dat", "", &xlsx_bytes)
        .expect("xlsx text extraction");
    assert!(xlsx_txt.contains("APAC-Q4") && xlsx_txt.contains("42500"));

    // 3. PNG disguised as `.dat`
    let mut png_bytes = Vec::new();
    let png_img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        12,
        image::Rgba([20, 40, 60, 255]),
    ));
    png_img
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.path().join("blob_png.dat"), &png_bytes).unwrap();
    let r_png = reg.execute("read_file", &ctx, json!({"path": "blob_png.dat"}));
    assert!(!r_png.is_error, "png failed: {:?}", r_png.payload);
    let pv = r_png.payload.to_model_value();
    assert_eq!(pv["format"], "image");
    assert_eq!(pv["mime_type"], "image/png");
    assert_eq!(pv["dimensions"], json!({"width": 16, "height": 12}));
    assert_eq!(r_png.media.len(), 1);
    assert_eq!(r_png.media[0].mime_type, "image/png");
    assert!(crate::extract_document_text_from_bytes("blob_png.dat", "", &png_bytes).is_none());

    // 4. JPEG disguised as `.dat`
    let mut jpg_bytes = Vec::new();
    let jpg_img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        24,
        18,
        image::Rgb([200, 110, 50]),
    ));
    jpg_img
        .write_to(&mut std::io::Cursor::new(&mut jpg_bytes), image::ImageFormat::Jpeg)
        .unwrap();
    std::fs::write(dir.path().join("blob_jpg.dat"), &jpg_bytes).unwrap();
    let r_jpg = reg.execute("read_file", &ctx, json!({"path": "blob_jpg.dat"}));
    assert!(!r_jpg.is_error, "jpeg failed: {:?}", r_jpg.payload);
    let jv = r_jpg.payload.to_model_value();
    assert_eq!(jv["format"], "image");
    assert_eq!(jv["mime_type"], "image/jpeg");
    assert_eq!(jv["dimensions"], json!({"width": 24, "height": 18}));
    assert_eq!(r_jpg.media.len(), 1);
    assert_eq!(r_jpg.media[0].mime_type, "image/jpeg");
    assert!(crate::extract_document_text_from_bytes("blob_jpg.dat", "", &jpg_bytes).is_none());

    // 5. PDF disguised as `.dat`
    let pdf_bytes = build_multipage_pdf(&[vec!["DISGUISED_PDF_MAGIC_OK".to_string()]]);
    std::fs::write(dir.path().join("blob_pdf.dat"), &pdf_bytes).unwrap();
    let r_pdf = reg.execute("read_file", &ctx, json!({"path": "blob_pdf.dat"}));
    assert!(!r_pdf.is_error, "pdf failed: {:?}", r_pdf.payload);
    assert_eq!(r_pdf.payload.to_model_value()["format"], "pdf");
    assert!(r_pdf.payload.to_model_value()["extracted_pages"][0]["text"]
        .as_str()
        .unwrap()
        .contains("DISGUISED_PDF_MAGIC_OK"));
}

#[test]
fn real_scenario_safe_file_mutation_pipeline_and_workspace_escape_rejection() {
    let ws = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_secret = outside.path().join("secret.txt");
    std::fs::write(&outside_secret, "DO_NOT_TOUCH\n").unwrap();

    #[cfg(unix)]
    {
        let link = ws.path().join("escape_dir");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    }

    let ctx = ToolContext::new(ws.path());
    let reg = ToolRegistry::with_builtin_tools();

    // 1. Safe in-workspace lifecycle: write_file -> read_file -> edit_file -> apply_patch -> move_file -> delete_file
    let w = reg.execute(
        "write_file",
        &ctx,
        json!({
            "path": "src/service.rs",
            "content": "pub fn port() -> u16 {\n    8080\n}\n\npub fn name() -> &'static str {\n    \"alpha\"\n}\n"
        }),
    );
    assert!(!w.is_error);
    assert!(matches!(w.effects[0].transition, VersionTransition::Created { .. }));

    let r = reg.execute("read_file", &ctx, json!({"path": "src/service.rs"}));
    assert!(!r.is_error);
    assert!(r.payload.to_model_value()["content"]
        .as_str()
        .unwrap()
        .contains("2:     8080"));

    let e = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "src/service.rs", "old_text": "8080", "new_text": "8443"}),
    );
    assert!(!e.is_error);
    assert!(matches!(e.effects[0].transition, VersionTransition::Modified { .. }));

    let patch = "--- a/src/service.rs\n+++ b/src/service.rs\n@@ -4,4 +4,4 @@\n \n pub fn name() -> &'static str {\n-    \"alpha\"\n+    \"beta\"\n }\n";
    let p = reg.execute("apply_patch", &ctx, json!({"patch": patch}));
    assert!(!p.is_error, "apply_patch failed: {:?}", p.payload);
    assert_eq!(
        std::fs::read_to_string(ws.path().join("src/service.rs")).unwrap(),
        "pub fn port() -> u16 {\n    8443\n}\n\npub fn name() -> &'static str {\n    \"beta\"\n}\n"
    );

    let mv = reg.execute(
        "move_file",
        &ctx,
        json!({"source": "src/service.rs", "destination": "src/server.rs"}),
    );
    assert!(!mv.is_error, "move_file failed: {:?}", mv.payload);
    assert!(!ws.path().join("src/service.rs").exists());
    assert!(ws.path().join("src/server.rs").is_file());

    let del = reg.execute("delete_file", &ctx, json!({"path": "src/server.rs"}));
    assert!(!del.is_error);
    assert!(matches!(del.effects[0].transition, VersionTransition::Deleted { .. }));
    assert!(!ws.path().join("src/server.rs").exists());

    // 2. Workspace escape rejection across EVERY mutation tool (relative `..`, absolute path, symlink)
    let abs_outside = outside.path().join("pwned.txt").display().to_string();
    let escape_targets = vec![
        "../escaped_rel.txt".to_string(),
        abs_outside.clone(),
        #[cfg(unix)]
        "escape_dir/pwned_via_symlink.txt".to_string(),
    ];
    for target in &escape_targets {
        let rw = reg.execute(
            "write_file",
            &ctx,
            json!({"path": target, "content": "malicious"}),
        );
        assert!(rw.is_error, "write_file must reject escape target '{target}'");

        let rd = reg.execute("delete_file", &ctx, json!({"path": target}));
        assert!(rd.is_error, "delete_file must reject escape target '{target}'");

        let rm = reg.execute(
            "move_file",
            &ctx,
            json!({"source": "nonexistent", "destination": target}),
        );
        assert!(rm.is_error, "move_file must reject escape destination '{target}'");

        let rp = reg.execute(
            "apply_patch",
            &ctx,
            json!({"patch": format!("--- /dev/null\n+++ {target}\n@@ -0,0 +1 @@\n+malicious\n")}),
        );
        assert!(rp.is_error, "apply_patch must reject escape target '{target}'");
    }

    // Outside secret file must remain untouched
    assert_eq!(
        std::fs::read_to_string(&outside_secret).unwrap(),
        "DO_NOT_TOUCH\n"
    );
    assert!(!outside.path().join("pwned.txt").exists());
    assert!(!outside.path().join("pwned_via_symlink.txt").exists());
}

#[test]
fn real_scenario_checkpoint_create_diff_restore_undo_redo_across_create_edit_delete() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "alpha_v1\n").unwrap();
    std::fs::write(dir.path().join("beta.txt"), "beta_v1\n").unwrap();

    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // Baseline checkpoint
    let cp = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "create", "label": "before-multi-op"}),
    );
    assert!(!cp.is_error);
    let cp_id = cp.payload.to_model_value()["checkpoint_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Mutate: edit alpha, delete beta, create gamma
    assert!(!reg.execute("read_file", &ctx, json!({"path": "alpha.txt"})).is_error);
    assert!(!reg
        .execute(
            "edit_file",
            &ctx,
            json!({"path": "alpha.txt", "old_text": "alpha_v1", "new_text": "alpha_v2"}),
        )
        .is_error);
    assert!(!reg
        .execute("delete_file", &ctx, json!({"path": "beta.txt"}))
        .is_error);
    assert!(!reg
        .execute(
            "write_file",
            &ctx,
            json!({"path": "gamma.txt", "content": "gamma_new\n"}),
        )
        .is_error);

    let diff = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "diff", "checkpoint_id": cp_id}),
    );
    assert!(!diff.is_error);
    let dv = diff.payload.to_model_value();
    assert!(dv["modified"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["path"] == "alpha.txt"));
    assert!(dv["deleted"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d == "beta.txt" || d["path"] == "beta.txt"));

    // Restore -> alpha returns to v1, beta recreated as v1, unrelated gamma preserved
    let restored = reg.execute(
        "workspace_checkpoint",
        &ctx,
        json!({"action": "restore", "checkpoint_id": cp_id}),
    );
    assert!(!restored.is_error);
    assert_eq!(std::fs::read_to_string(dir.path().join("alpha.txt")).unwrap(), "alpha_v1\n");
    assert_eq!(std::fs::read_to_string(dir.path().join("beta.txt")).unwrap(), "beta_v1\n");
    assert_eq!(std::fs::read_to_string(dir.path().join("gamma.txt")).unwrap(), "gamma_new\n");

    // Undo -> returns to post-edit state (alpha_v2, beta deleted)
    let undo = reg.execute("workspace_checkpoint", &ctx, json!({"action": "undo"}));
    assert!(!undo.is_error, "undo failed: {:?}", undo.payload);
    assert_eq!(std::fs::read_to_string(dir.path().join("alpha.txt")).unwrap(), "alpha_v2\n");
    assert!(!dir.path().join("beta.txt").exists());

    // Redo -> re-applies restored baseline (alpha_v1, beta_v1)
    let redo = reg.execute("workspace_checkpoint", &ctx, json!({"action": "redo"}));
    assert!(!redo.is_error, "redo failed: {:?}", redo.payload);
    assert_eq!(std::fs::read_to_string(dir.path().join("alpha.txt")).unwrap(), "alpha_v1\n");
    assert_eq!(std::fs::read_to_string(dir.path().join("beta.txt")).unwrap(), "beta_v1\n");
}

#[test]
fn real_scenario_multi_edit_and_grep_glob_search_pipeline_over_nested_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src/net")).unwrap();
    std::fs::create_dir_all(dir.path().join("tests")).unwrap();
    std::fs::create_dir_all(dir.path().join("build")).unwrap();

    std::fs::write(
        dir.path().join("src/net/client.rs"),
        "const TIMEOUT: u64 = 10;\nconst RETRIES: u32 = 2;\nconst HOST: &str = \"old.local\";\nconst TAG: &str = \"v1\";\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tests/net_test.rs"),
        "// checks old.local\nfn test_client() {}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("build/ignored.rs"),
        "const HOST: &str = \"old.local\";\n",
    )
    .unwrap();

    let ctx = ToolContext::new(dir.path());
    let reg = ToolRegistry::with_builtin_tools();

    // glob skips gitignored build/ directory
    let gl = reg.execute("glob", &ctx, json!({"pattern": "**/*.rs"}));
    assert!(!gl.is_error);
    let gl_files: Vec<String> = gl.payload.to_model_value()["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect();
    assert_eq!(gl_files.len(), 2, "expected 2 non-ignored .rs files, got {gl_files:?}");
    assert!(!gl_files.iter().any(|f| f.contains("build")));

    // grep respects .gitignore and supports include + context_lines
    let gr = reg.execute(
        "grep",
        &ctx,
        json!({"pattern": "old\\.local", "include": "*.rs", "context_lines": 1}),
    );
    assert!(!gr.is_error);
    assert_eq!(gr.payload.to_model_value()["count"], 2);

    // multi_edit_file atomically rejects partial failure (missing 2nd anchor) without modifying file
    assert!(!reg.execute("read_file", &ctx, json!({"path": "src/net/client.rs"})).is_error);
    let bad_multi = reg.execute(
        "multi_edit_file",
        &ctx,
        json!({
            "path": "src/net/client.rs",
            "edits": [
                {"old_text": "TIMEOUT: u64 = 10;", "new_text": "TIMEOUT: u64 = 30;"},
                {"old_text": "DOES_NOT_EXIST", "new_text": "X"}
            ]
        }),
    );
    assert!(bad_multi.is_error);
    assert!(std::fs::read_to_string(dir.path().join("src/net/client.rs"))
        .unwrap()
        .contains("TIMEOUT: u64 = 10;"));

    // multi_edit_file applies all 4 non-overlapping edits cleanly in one shot
    let ok_multi = reg.execute(
        "multi_edit_file",
        &ctx,
        json!({
            "path": "src/net/client.rs",
            "edits": [
                {"old_text": "TIMEOUT: u64 = 10;", "new_text": "TIMEOUT: u64 = 30;"},
                {"old_text": "RETRIES: u32 = 2;", "new_text": "RETRIES: u32 = 5;"},
                {"old_text": "\"old.local\"", "new_text": "\"prod.local\""},
                {"old_text": "\"v1\"", "new_text": "\"v2\""}
            ]
        }),
    );
    assert!(!ok_multi.is_error, "multi_edit_file failed: {:?}", ok_multi.payload);
    let updated = std::fs::read_to_string(dir.path().join("src/net/client.rs")).unwrap();
    assert!(updated.contains("TIMEOUT: u64 = 30;"));
    assert!(updated.contains("RETRIES: u32 = 5;"));
    assert!(updated.contains("\"prod.local\""));
    assert!(updated.contains("\"v2\""));
}

#[test]
fn real_scenario_large_pdf_read_file_spills_and_recovers_via_session_scoped_tool_output() {
    let dir = tempfile::tempdir().unwrap();
    let pages: Vec<Vec<String>> = (1..=15)
        .map(|p| {
            let mut lines = vec![format!("PDF_SPILL_PAGE_{p:02}_HEADER")];
            for r in 1..=35 {
                lines.push(format!(
                    "Page {p:02} item {r:02}: Large structured document content to ensure default 10-page extraction exceeds 24 KiB spill threshold."
                ));
            }
            lines
        })
        .collect();
    let pdf_bytes = build_multipage_pdf(&pages);
    std::fs::write(dir.path().join("large_paper.pdf"), &pdf_bytes).unwrap();

    let store = std::sync::Arc::new(crate::OutputStore::new(dir.path().join("outputs")));
    let mut ctx = ToolContext::new(dir.path()).with_session_id("uuid-pdf-sess-001");
    ctx.output_store = std::sync::Arc::clone(&store);
    let reg = ToolRegistry::with_builtin_tools();

    let pdf_res = reg.execute("read_file", &ctx, json!({"path": "large_paper.pdf"}));
    assert!(!pdf_res.is_error);
    assert!(
        pdf_res.payload.to_model_string().len() > crate::DEFAULT_SPILL_THRESHOLD_BYTES,
        "expected default 10-page PDF payload to exceed 24 KiB"
    );

    let (compact, meta) = store
        .maybe_spill_payload_scoped(
            "uuid-pdf-sess-001",
            "read_file",
            &pdf_res.payload,
            crate::DEFAULT_SPILL_THRESHOLD_BYTES,
        )
        .expect("large PDF read_file output must spill");
    assert_eq!(compact.to_model_value()["spilled"], json!(true));

    let found = reg.execute(
        "tool_output",
        &ctx,
        json!({"action": "search", "output_id": meta.output_id, "pattern": "PDF_SPILL_PAGE_08_HEADER"}),
    );
    assert!(!found.is_error, "tool_output search failed: {:?}", found.payload);
    assert_eq!(found.payload.to_model_value()["count"], 1);

    let mut other_ctx = ToolContext::new(dir.path()).with_session_id("uuid-other-sess");
    other_ctx.output_store = std::sync::Arc::clone(&store);
    let denied = reg.execute(
        "tool_output",
        &other_ctx,
        json!({"action": "search", "output_id": meta.output_id, "pattern": "PDF_SPILL_PAGE_08_HEADER"}),
    );
    assert!(denied.is_error);
    assert!(denied.payload.to_model_string().contains("unknown output_id"));
}

#[test]
fn web_fetch_fake_ip_policy_blocks_literals_and_private_ips_while_allowing_public_domain_tun() {
    use std::net::{IpAddr, Ipv4Addr};

    // Literal 198.18.x.x URLs MUST still be blocked (SSRF protection)
    let literal = crate::web::resolve_and_validate_public_url("http://198.18.0.15/secret");
    assert!(literal.is_err(), "literal 198.18.0.15 must be rejected");
    let loopback = crate::web::resolve_and_validate_public_url("http://127.0.0.1:8080/");
    assert!(loopback.is_err(), "loopback must be rejected");
    let metadata = crate::web::resolve_and_validate_public_url("http://169.254.169.254/latest");
    assert!(metadata.is_err(), "metadata IP must be rejected");

    // Helper checks: RFC 2544 benchmark / TUN Fake-IP detection
    assert!(crate::web::is_fake_ip_benchmark(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1))));
    assert!(crate::web::is_fake_ip_benchmark(IpAddr::V4(Ipv4Addr::new(198, 19, 255, 254))));
    assert!(!crate::web::is_fake_ip_benchmark(IpAddr::V4(Ipv4Addr::new(198, 20, 0, 1))));
    assert!(!crate::web::is_fake_ip_benchmark(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));

    // Public domain detection rejects literals, localhost, .local, .internal
    assert!(crate::web::is_public_domain_name("arxiv.org"));
    assert!(crate::web::is_public_domain_name("openreview.net"));
    assert!(!crate::web::is_public_domain_name("198.18.0.15"));
    assert!(!crate::web::is_public_domain_name("localhost"));
    assert!(!crate::web::is_public_domain_name("myhost.local"));
    assert!(!crate::web::is_public_domain_name("db.internal"));
}

#[test]
fn web_search_query_relaxation_strips_quotes_and_boolean_operators() {
    assert_eq!(
        crate::web_search::relax_search_query(r#""Sibling-Conditioned State Transfer" OR "DFD-R" arxiv"#).as_deref(),
        Some("Sibling-Conditioned State Transfer DFD-R arxiv")
    );
    assert_eq!(
        crate::web_search::relax_search_query("simple query"),
        None,
        "simple short query without quotes/OR should not spuriously relax"
    );
    let long = crate::web_search::relax_search_query(
        "one two three four five six seven eight nine ten",
    );
    assert_eq!(long.as_deref(), Some("one two three four five six seven"));
}
