// SPDX-License-Identifier: Apache-2.0
//! Bounded canonical compaction lineage remains available outside UI projection.
use nola_store::NolaStore;
use serde_json::json;

#[test]
fn compaction_lineage_reads_original_source_while_ui_projection_hides_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("lineage.db");
    drop(NolaStore::open(&database).unwrap());

    let source = format!("SOURCE_FACT_7319:{}", "x".repeat(40_000));
    let semantic = json!({
        "retained_item_ids": ["source-message"],
        "source_fingerprint": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    });
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,?2,1,1,2)",
            ("lineage", "Lineage"),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO messages VALUES(?1,?2,NULL,'user','completed',1,1,2)",
            ("source-message", "lineage"),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES('source-part','source-message','text',1,'completed',?1,NULL,1,2)",
            [&source],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO messages VALUES(?1,?2,NULL,'assistant','completed',2,1,2)",
            ("compaction-message", "lineage"),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES('compaction-part','compaction-message','compaction',1,'completed','summary',?1,1,2)",
            [semantic.to_string()],
        )
        .unwrap();
    drop(connection);

    let store = NolaStore::open(&database).unwrap();
    let projection = nola_application::load_conversation_projection_v1(&store, "lineage").unwrap();
    assert!(projection
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .all(|part| !matches!(part, nola_domain::MessagePartView::Compaction { .. })));

    let lineage = nola_application::load_compaction_lineage(&store, "lineage", None).unwrap();
    assert_eq!(lineage["compaction_part_id"], "compaction-part");
    assert_eq!(lineage["source_item_ids"], json!(["source-message"]));
    assert_eq!(lineage["missing_source_item_ids"], json!([]));
    assert_eq!(lineage["sources"].as_array().unwrap().len(), 1);
    assert_eq!(lineage["source_bodies_are_previews"], true);
    let preview = lineage["sources"][0]["message"]["parts"][0]["content_text"]
        .as_str()
        .unwrap();
    assert!(preview.starts_with("SOURCE_FACT_7319:"));
    assert!(preview.len() < source.len());
    assert_eq!(
        lineage["sources"][0]["message"]["parts"][0]["content_text_complete"],
        false
    );
}
