#!/usr/bin/env python3
"""Real-model discovery/authorization/clangd query in an isolated project."""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import tempfile

from test_real_cli_dirty_audit import run_cli
from test_real_cli_commit_followup import assert_completed_turn
from canonical_run_audit import audit_runs


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--clangd", type=Path)
    parser.add_argument("--verify-evidence", type=Path,
                        help="Read-only revalidation of an existing isolated sample; no model/server/config access")
    args = parser.parse_args()
    if args.verify_evidence is not None:
        print(json.dumps(verify_evidence(args.verify_evidence.resolve())))
        return
    if args.binary is None or args.clangd is None:
        parser.error("--binary and --clangd are required for a new sample")
    assert args.clangd.is_absolute() and args.clangd.is_file()
    root = Path(tempfile.mkdtemp(prefix="nola-language-service-"))
    workspace = (root / "workspace")
    workspace.mkdir()
    workspace = workspace.resolve()
    step = root / "turn-1"
    step.mkdir()
    source = workspace / "example.cpp"
    local_line = "int local() { int answer = 7; return answer; }"
    local_use = local_line.rindex("answer") + 1
    text = "int answer() { return 42; }\nint main() { return answer(); }\n" + local_line + "\n"
    header = workspace / "shared.hpp"
    header_text = "inline int shared_answer() { return 19; }\n"
    header.write_text(header_text)
    cross_line = "int cross_file() { return shared_answer(); }"
    cross_use = cross_line.index("shared_answer") + 1
    text += '#include "shared.hpp"\n' + cross_line + "\n"
    base_line = "struct Base { virtual int run() = 0; };"
    implementation_use = base_line.index("run") + 1
    text += base_line + "\nstruct Derived : Base { int run() override { return 1; } };\n"
    source.write_text(text)
    config = workspace / "nola.json"
    config.write_text(json.dumps({"language_services": [{
        "id": "acceptance-clangd", "executable": str(args.clangd),
        "arguments": ["--background-index=false", "--clang-tidy=false", "--log=error"],
        "project_root": str(workspace), "language_ids": ["cpp"], "environment": {}, "timeout_ms": 30000}]}))
    before_config = config.read_bytes()
    llm = json.loads((Path.home() / ".nola/config.json").read_text())["llm"]
    database = root / "fixture.db"
    env = dict(os.environ, NOLA_DB_PATH=str(database), NOLA_LONG_TERM_MEMORY_ENABLED="0",
               NOLA_CLI_INTERACTION_MODE="never")
    prompt = ("使用已配置的语言服务检查example.cpp。先发现服务及其配置指纹，再查询文档符号，"
              "查询第2行第22列answer调用的定义、引用和hover；"
              f"再查询第3行第{local_use}列局部answer使用的定义。报告函数与同名局部变量是否是不同绑定。"
              f"另外查询第5行第{cross_use}列shared_answer调用的定义和引用，报告跨文件目标及位置。"
              "跨文件引用查询须把语义定义发现的相关文档作为supporting_documents打开同步，避免冷索引漏报；不要猜测目标路径。"
              f"查询第6行第{implementation_use}列虚函数run的implementations；"
              "再使用workspace_symbols查询Derived并报告位置。"
              "必须通过language_service_query验证，不要用文本扫描或shell代替语义查询，"
              "不修改源文件或配置，不安装服务。")
    command = [str(args.binary.resolve()), prompt, "--project", str(workspace),
               "--allow-dir", str(workspace), "--session", "language-service-acceptance",
               "--yes", "--max-iterations", "20", "--json", "--endpoint", llm["endpoint"],
               "--model", llm["model"], "--api-key", llm["api_key"]]
    print(f"Isolated evidence: {root}", flush=True)
    assert run_cli(Path(__file__).resolve().parents[1], step, command, env, 240, llm["endpoint"]) == 0
    result = verify_evidence(root)
    assert source.read_text() == text and config.read_bytes() == before_config
    assert header.read_text() == header_text, "Cross-file navigation modified the target"
    print(json.dumps(result))


def verify_evidence(root):
    step = root / "turn-1"
    database = root / "fixture.db"
    source = (root / "workspace/example.cpp").resolve()
    header = (root / "workspace/shared.hpp").resolve()
    events = [json.loads(line) for line in (step / "events.ndjson").read_text().splitlines()]
    assert_completed_turn(events)
    calls = {}
    for event in events:
        trace = event.get("trace", {})
        payload = trace.get("payload", {})
        if trace.get("kind") == "tool.completed" and payload.get("tool_name") == "language_service_query" and trace.get("status") == "succeeded":
            arguments = payload["arguments"]
            calls[payload["call_id"]] = (arguments["operation"], arguments.get("line"))
    results = {}
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as db:
        canonical = audit_runs(db, "language-service-acceptance")
        assert canonical["runCount"] == 1 and canonical["outcomes"][0] in {"answered", "action_completed"}
        for (raw,) in db.execute("SELECT payload_json FROM host_event_inbox"):
            event = json.loads(raw)
            payload = event.get("payload", {})
            if payload.get("type") == "tool_completed" and payload.get("call_id") in calls:
                assert event["session_id"] == "language-service-acceptance"
                data = payload["output"]["data"]
                assert data["code_evidence"]["backend"] == "language_service"
                assert data["service_id"] == "acceptance-clangd"
                results[calls[payload["call_id"]]] = data["result"]
    symbols = next(value for (operation, _), value in results.items() if operation == "document_symbols")
    assert any(symbol["name"] == "answer" for symbol in symbols)
    for queried_line, definition_line, target in [(2, 0, source), (3, 2, source), (5, 0, header)]:
        definitions = results[("definitions", queried_line)]
        if isinstance(definitions, dict):
            definitions = [definitions]
        assert len(definitions) == 1, definitions
        location = definitions[0]
        assert location.get("uri", location.get("targetUri")) == target.as_uri()
        assert location.get("range", location.get("targetSelectionRange"))["start"]["line"] == definition_line
    references = results[("references", 2)]
    assert {item["range"]["start"]["line"] for item in references} == {0, 1}, references
    assert all(item["uri"] == source.as_uri() for item in references)
    cross_references = results[("references", 5)]
    assert {(item["uri"], item["range"]["start"]["line"]) for item in cross_references} == {
        (header.as_uri(), 0), (source.as_uri(), 4)
    }, cross_references
    assert results[("hover", 2)]["contents"], "Empty semantic hover"
    implementations = results[("implementations", 6)]
    assert len(implementations) == 1, "Expected exactly one implementation for the virtual method"
    assert all(item.get("uri", item.get("targetUri")) == source.as_uri()
               and item.get("range", item.get("targetSelectionRange"))["start"]["line"] == 6
               for item in implementations), implementations
    workspace_symbols = next(value for (operation, _), value in results.items() if operation == "workspace_symbols")
    derived = [item for item in workspace_symbols if item["name"] == "Derived"]
    assert derived, "Workspace query did not find Derived"
    assert all(item["location"]["uri"] == source.as_uri()
               and item["location"]["range"]["start"]["line"] == 6 for item in derived), derived
    return {"passed": True,
                      "verified_operations": ["definitions", "document_symbols", "hover", "references",
                                              "implementations", "workspace_symbols"],
                      "observed_operations": sorted({op for op, _ in results}),
                      "cross_file_definition_verified": True,
                      "cross_file_references_verified": True, "evidence": str(root),
                      "limitations": ["Read-only revalidation verifies persisted semantic results and Run completion, not the original pre-run file/config snapshot."]}


if __name__ == "__main__":
    main()
