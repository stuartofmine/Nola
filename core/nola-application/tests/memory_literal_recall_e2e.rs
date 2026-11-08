// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{ApplicationCommand, MemoryCommand};
use serde_json::{json, Value};

#[test]
fn literal_recall_survives_restart_without_sql_patterns_in_application() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("memory.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    for (id, content, namespace) in [
        ("percent", "100% complete", "fixture"),
        ("underscore", "snake_case", "fixture"),
        ("slash", r"path\name", "fixture"),
        ("unicode", "预算约束", "fixture"),
        ("plain", "ordinary content", "fixture"),
        ("foreign", "100% complete", "foreign"),
    ] {
        let command: MemoryCommand = serde_json::from_value(json!({
            "type": "commit", "intent": "explicit_user", "input": {
                "record": {
                    "id": id, "owner": {"kind":"profile","profile_id":"local"}, "namespace": {"kind": "custom", "name": namespace},
                    "scope": "global", "kind": "reference", "content": content,
                    "summary": null, "tags": [], "confidence": 0.8, "importance": 0.9,
                    "sensitivity": null, "retention": {"mode": "permanent"}, "supersedes": [],
                    "provenance": {"kind": "explicit_user", "source": "test", "session_id": null,
                        "turn_id": null, "item_ids": [], "observed_at": null}
                }, "quality": null, "proposed_at_ms": 1
            }
        }))
        .unwrap();
        runtime
            .execute(ApplicationCommand::Memory { command })
            .unwrap();
    }
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    for (query, id) in [
        ("%", "percent"),
        ("_", "underscore"),
        (r"\", "slash"),
        ("预算", "unicode"),
    ] {
        let command = serde_json::from_value(json!({
            "type": "recall", "input": {
                "binding": {"profile_id":"local"},
                "namespace": {"kind": "custom", "name": "fixture"}, "scope": "global",
                "query": query, "kind": null, "now_iso": "2026-09-08T00:00:00Z", "limit": 20
            }
        }))
        .unwrap();
        let result: Value = runtime
            .execute(ApplicationCommand::Memory { command })
            .unwrap();
        assert_eq!(result.as_array().unwrap().len(), 1, "query={query}");
        assert_eq!(result[0]["record"]["id"], id);
    }
}
