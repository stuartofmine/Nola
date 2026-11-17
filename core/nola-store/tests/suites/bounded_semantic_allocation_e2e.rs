// SPDX-License-Identifier: Apache-2.0
//! Consolidated bounded-allocation suite merging all 5 `*_bounded.rs` tests onto one shared
//! thread-local `MeasuredAllocator` in `store_suite.rs`.

use crate::allocator_probe::measure_allocations;
use nola_domain::{
    CanonicalBodyReference, CanonicalPartOwner, HistoricalProjectionStatus, PartBodyField,
    SemanticValue,
};
use nola_store::{
    semantic_json_projection::{extract_top_level, ExtractedValue, ScalarValue, TokenKind},
    semantic_nested_draft::{validate_nested, NestedKind, NestedStatus},
    semantic_part_draft::{extract_draft, DraftStatus, HistoricalSchema},
    structured_json_validation::validate_json_chunks,
    NolaStore,
};
use nola_store_port::canonical_body::CanonicalBodyRepository;
use rusqlite::{params, Connection};

#[test]
fn ten_million_byte_streaming_json_validators_and_semantic_drafts_keep_buffers_bounded() {
    // 1. Structured JSON chunk validator (strings, huge keys, surrogate escapes, numbers).
    for mode in 0..4 {
        let object_key = mode == 1;
        let escaped = mode == 2;
        let total = if escaped { 10_000_010_u64 } else { 10_000_000_u64 };
        let mut position = 0u64;
        let (result, largest, peak) = measure_allocations(|| {
            let source = std::iter::from_fn(move || {
                if position == total {
                    return None;
                }
                let offset = position;
                let length = (total - position).min(32768) as usize;
                let mut chunk = String::with_capacity(length);
                for index in 0..length {
                    let at = offset + index as u64;
                    let byte = if mode == 3 {
                        if at == 1 { b'.' } else { b'0' }
                    } else if object_key {
                        match at {
                            0 => b'{',
                            1 => b'"',
                            v if v == total - 4 => b'"',
                            v if v == total - 3 => b':',
                            v if v == total - 2 => b'0',
                            v if v == total - 1 => b'}',
                            _ => b'x',
                        }
                    } else {
                        match at {
                            0 => b'"',
                            v if v == total - 1 => b'"',
                            _ if escaped => b"\\uD83D\\uDE00"[((at - 1) % 12) as usize],
                            _ => b'x',
                        }
                    };
                    chunk.push(char::from(byte));
                }
                position += length as u64;
                Some(Ok((offset, chunk)))
            });
            validate_json_chunks(source, total)
        });
        let stats = result.unwrap();
        assert_eq!(stats.total_utf8_bytes, total);
        assert_eq!(stats.maximum_chunk_bytes, 32768);
        assert_eq!(stats.chunk_count, total.div_ceil(32768));
        assert!(largest <= 32768, "largest allocation={largest}");
        assert!(peak <= 32768 + 4096, "peak live allocated bytes={peak}");
    }

    // 2. Semantic top-level JSON projection (huge strings, huge keys, arrays, escapes).
    for mode in 0..4 {
        let (prefix, suffix, body_bytes): (&str, &str, u64) = match mode {
            0 => ("{\"huge\":\"", "\",\"kind\":\"tool\"}", 10_000_000),
            1 => ("{\"", "\":null,\"kind\":\"tool\"}", 10_000_000),
            2 => ("{\"huge\":[", "],\"kind\":\"tool\"}", 10_000_001),
            _ => ("{\"huge\":\"", "\",\"kind\":\"tool\"}", 10_000_008),
        };
        let total = prefix.len() as u64 + body_bytes + suffix.len() as u64;
        let (result, largest, peak) = measure_allocations(|| {
            extract_top_level(
                || {
                    let mut position = 0u64;
                    std::iter::from_fn(move || {
                        if position == total {
                            return None;
                        }
                        let offset = position;
                        let length = (total - position).min(32768) as usize;
                        let mut text = String::with_capacity(length);
                        for index in 0..length {
                            let at = offset + index as u64;
                            let byte = if at < prefix.len() as u64 {
                                prefix.as_bytes()[at as usize]
                            } else if at >= prefix.len() as u64 + body_bytes {
                                suffix.as_bytes()[(at - prefix.len() as u64 - body_bytes) as usize]
                            } else {
                                let body = at - prefix.len() as u64;
                                match mode {
                                    2 => {
                                        if body % 2 == 0 {
                                            b'0'
                                        } else {
                                            b','
                                        }
                                    }
                                    3 => b"\\uD83D\\uDE00"[(body % 12) as usize],
                                    _ => b'x',
                                }
                            };
                            text.push(char::from(byte));
                        }
                        position += length as u64;
                        Some(Ok((offset, text)))
                    })
                },
                total,
                &["huge", "kind"],
                4096,
            )
        });
        let result = result.unwrap();
        let kind = result.fields.iter().find(|e| e.field_index == 1).unwrap();
        assert_eq!(
            kind.value,
            ExtractedValue::Complete(ScalarValue::String("tool".into()))
        );
        if mode != 1 {
            let huge = result.fields.iter().find(|e| e.field_index == 0).unwrap();
            assert_eq!(huge.value, ExtractedValue::Deferred);
            assert_eq!(huge.span.end_bytes - huge.span.start_bytes, body_bytes + 2);
        } else {
            assert_eq!(result.fields.len(), 1);
        }
        assert!(largest <= 32768, "largest allocation={largest}");
        assert!(peak <= 32768 + 3 * 4096, "peak live bytes={peak}");
    }

    // 3. Semantic part draft extraction (AgentPart & MessagePartView tool shapes).
    for schema in [HistoricalSchema::AgentPart, HistoricalSchema::MessagePartView] {
        let prefix = "{\"execution_source\":{\"kind\":\"model\"},\"input\":\"";
        let suffix = if schema == HistoricalSchema::AgentPart {
            "\",\"kind\":\"tool\",\"id\":\"p\",\"message_id\":\"m\",\"turn_id\":\"r\",\"call_id\":\"call\",\"capability_id\":\"read\",\"title\":\"Read\",\"state\":\"completed\",\"artifacts\":[]}"
        } else {
            "\",\"kind\":\"tool\",\"part_id\":\"p\",\"revision\":1,\"tool_call_id\":\"call\",\"capability_id\":\"read\",\"title\":\"Read\",\"origin\":{\"kind\":\"unknown\"},\"state\":\"completed\",\"artifacts\":[],\"affected_resources\":[]}"
        };
        let body_bytes = 10_000_000u64;
        let total = prefix.len() as u64 + body_bytes + suffix.len() as u64;
        let (draft, maximum, peak) = measure_allocations(|| {
            extract_draft(
                || {
                    let mut position = 0u64;
                    std::iter::from_fn(move || {
                        if position == total {
                            return None;
                        }
                        let offset = position;
                        let length = (total - position).min(32768) as usize;
                        let mut text = String::with_capacity(length);
                        for i in 0..length {
                            let at = offset + i as u64;
                            let byte = if at < prefix.len() as u64 {
                                prefix.as_bytes()[at as usize]
                            } else if at >= prefix.len() as u64 + body_bytes {
                                suffix.as_bytes()[(at - prefix.len() as u64 - body_bytes) as usize]
                            } else {
                                b'x'
                            };
                            text.push(char::from(byte));
                        }
                        position += length as u64;
                        Some(Ok((offset, text)))
                    })
                },
                total,
                "tool",
                8192,
            )
        });
        let draft = draft.unwrap();
        assert_eq!(draft.status, DraftStatus::Recognized);
        assert_eq!(draft.schema, Some(schema));
        if schema == HistoricalSchema::MessagePartView {
            for (name, kind, raw) in [
                ("title", TokenKind::String, "\"Read\""),
                ("origin", TokenKind::Object, "{\"kind\":\"unknown\"}"),
                ("artifacts", TokenKind::Array, "[]"),
            ] {
                let field = draft.field(name).unwrap();
                assert_eq!(field.span.kind, kind);
                let suffix_offset = prefix.len() as u64 + body_bytes;
                let start = usize::try_from(field.span.start_bytes - suffix_offset).unwrap();
                let end = usize::try_from(field.span.end_bytes - suffix_offset).unwrap();
                assert_eq!(&suffix[start..end], raw);
            }
            for field in ["origin", "artifacts"] {
                assert!(draft.pending_nested_validation.contains(&field));
            }
        }
        let input = draft.field("input").unwrap();
        assert_eq!(input.value, ExtractedValue::Deferred);
        assert_eq!(input.span.end_bytes - input.span.start_bytes, body_bytes + 2);
        assert!(!draft.pending_nested_validation.is_empty());
        assert!(maximum <= 32768, "maximum={maximum}");
        assert!(peak <= 32768 + 3 * 8192, "peak={peak}");
    }

    // 4. Semantic nested draft validation (10 modes covering origin, permission, resources, artifacts, evidence, location, range).
    let resource_item = "{\"resource_type\":\"file\",\"uri\":\"a\",\"operation\":\"read\"},";
    let nested_cases: [(NestedKind, &str, &str, &str, u64, Option<u64>); 10] = [
        (NestedKind::Origin, "{\"kind\":\"mcp\",\"provider_title\":\"", "\"}", "x", 10_000_000, None),
        (NestedKind::Permission, "{\"request_id\":\"r\",\"permission_domain\":\"fs\",\"rationale\":\"", "\"}", "x", 10_000_000, None),
        (NestedKind::AffectedResources, "[{\"resource_type\":\"file\",\"uri\":\"", "\",\"operation\":\"read\"}]", "x", 10_000_000, None),
        (NestedKind::Artifact, "{\"uri\":\"file:///a\",\"name\":\"", "\"}", "x", 10_000_000, None),
        (NestedKind::AffectedResources, "[", "]", resource_item, resource_item.len() as u64 * 200_000 - 1, Some(200_000)),
        (NestedKind::Evidence, "{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"title\":\"", "\"}}", "x", 10_000_000, None),
        (NestedKind::Location, "{\"page\":1,\"section\":\"", "\"}", "x", 10_000_000, None),
        (NestedKind::Range, "{\"start\":0.", ",\"end\":1}", "0", 10_000_000, None),
        (NestedKind::Evidence, "{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"namespace\":{\"kind\":\"custom\",\"name\":\"", "\"}}}", "x", 10_000_000, None),
        (NestedKind::Evidence, "{\"kind\":\"memory\",\"usage\":{\"record_id\":\"r\",\"component_id\":\"c\",\"supersedes\":[", "]}}", "\"old\",", 10_000_001, None),
    ];
    for (kind, prefix, suffix, pattern, body, expected_items) in nested_cases {
        let total = prefix.len() as u64 + body + suffix.len() as u64;
        let (result, maximum, peak) = measure_allocations(|| {
            validate_nested(
                || {
                    let mut position = 0u64;
                    std::iter::from_fn(move || {
                        if position == total {
                            return None;
                        }
                        let offset = position;
                        let length = (total - position).min(32768) as usize;
                        let mut text = String::with_capacity(length);
                        for i in 0..length {
                            let at = offset + i as u64;
                            let byte = if at < prefix.len() as u64 {
                                prefix.as_bytes()[at as usize]
                            } else if at >= prefix.len() as u64 + body {
                                suffix.as_bytes()[(at - prefix.len() as u64 - body) as usize]
                            } else {
                                pattern.as_bytes()
                                    [((at - prefix.len() as u64) % pattern.len() as u64) as usize]
                            };
                            text.push(char::from(byte));
                        }
                        position += length as u64;
                        Some(Ok((offset, text)))
                    })
                },
                total,
                kind,
                8192,
            )
        });
        let draft = result.unwrap();
        assert_eq!(draft.status, NestedStatus::Deferred);
        assert!(draft.resources.is_empty());
        if let Some(expected) = expected_items {
            assert_eq!(draft.total_items, Some(expected));
        }
        assert!(maximum <= 32768, "maximum={maximum}");
        assert!(peak <= 32768 + 3 * 8192, "peak={peak}");
    }
}

#[test]
fn ten_megabyte_historical_window_and_projection_do_not_materialize_original_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.db");
    let mut store = NolaStore::open(&path).unwrap();
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(
        "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);
         INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);
         INSERT INTO message_parts VALUES('p','m','text',1,'completed','',NULL,NULL,1,1);",
    )
    .unwrap();
    let prefix = "{\"kind\":\"text\",\"part_id\":\"p\",\"revision\":1,\"markdown\":\"";
    let suffix = "\",\"is_final\":true}";
    let body = 10_000_000u64;
    let total = prefix.len() as u64 + body + suffix.len() as u64;
    let reference = CanonicalBodyReference::for_field(
        "large-json".into(),
        "c".into(),
        "m".into(),
        "p".into(),
        PartBodyField::ContentJson,
        total,
        total.div_ceil(32768) as u32,
    )
    .unwrap();
    let mut position = 0;
    let mut input = std::iter::from_fn(|| {
        if position == total {
            return None;
        }
        let offset = position;
        let length = (total - position).min(32768) as usize;
        let mut text = String::with_capacity(length);
        for i in 0..length {
            let at = offset + i as u64;
            let byte = if at < prefix.len() as u64 {
                prefix.as_bytes()[at as usize]
            } else if at >= prefix.len() as u64 + body {
                suffix.as_bytes()[(at - prefix.len() as u64 - body) as usize]
            } else {
                b'x'
            };
            text.push(char::from(byte));
        }
        position += length as u64;
        Some(Ok((offset, text)))
    });
    store.prepare_body(&reference, &mut input, 20_000_000).unwrap();
    drop(input);
    sql.execute(
        "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
        params![serde_json::to_string(&reference).unwrap()],
    )
    .unwrap();
    let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
    assert!(store.read_historical_part(&owner, 1, 8192).is_err());
    store.begin_write_unit_of_work().unwrap();
    store.commit_write_unit_of_work().unwrap();

    let ((window, projection), maximum, peak) = measure_allocations(|| {
        (
            store.load_conversation_message_window("c", None, 10, 4096),
            store.read_historical_part(&owner, 1, 8192),
        )
    });
    let window = window.unwrap().unwrap();
    assert_eq!(window.conversation.messages.len(), 1);
    assert!(window.conversation.messages[0].parts[0].content_json.is_none());
    let projection = projection.unwrap().unwrap();
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    assert!(matches!(
        projection.fields["markdown"],
        SemanticValue::Deferred { .. }
    ));
    assert!(serde_json::to_vec(&projection).unwrap().len() <= 8192);
    assert!(maximum <= 32768, "maximum allocation={maximum}");
    assert!(peak <= 256 * 1024, "peak live bytes={peak}");
}
