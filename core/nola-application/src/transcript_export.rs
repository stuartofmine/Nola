// SPDX-License-Identifier: Apache-2.0
//! Canonical archive export uses metadata pages and revision-pinned body ranges.
use nola_store_port::transcript_export::*;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const COMPACTION_LINEAGE_BODY_BYTE_LIMIT: usize = 256 * 1024;
const COMPACTION_LINEAGE_METADATA_BYTE_LIMIT: usize = 2 * 1024 * 1024;
const COMPACTION_LINEAGE_MAX_PAGES: usize = 4_096;
const EXPORT_RANGE_BYTE_LIMIT: usize = 32 * 1024;

/// Archive-only original sources. No receiver-local trust labels leave this port.
pub fn read_tool_sources<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    owner: &CanonicalPartOwner,
    revision: i64,
) -> Result<Vec<nola_domain::tool_source::ToolSourceClaim>, String> {
    let bindings = store.tool_sources_for_part(owner, revision)?;
    if bindings.len() > 2 {
        return Err("tool source metadata exceeds one call/result pair".into());
    }
    bindings.into_iter().map(|binding| {
        if binding.canonical_owner()? != *owner {
            return Err("tool source export owner differs".into());
        }
        binding.claim()
    }).collect()
}

/// Bounded raw archive bytes, deliberately separate from model-safe previews.
pub fn read_tool_source_range<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    owner: &CanonicalPartOwner,
    source_item_id: &str,
    revision: i64,
    offset: u64,
    limit: usize,
) -> Result<nola_domain::tool_source::ToolSourceExportRange, String> {
    if limit == 0 || limit > EXPORT_RANGE_BYTE_LIMIT {
        return Err("tool source export range must be between 1 and 32768 bytes".into());
    }
    let binding = store.tool_source_by_id(owner.conversation_id(), source_item_id, revision)?
        .ok_or("tool source export identity was not found")?;
    if binding.canonical_owner()? != *owner {
        return Err("tool source export owner differs".into());
    }
    let total = binding.body.total_utf8_bytes();
    let page = store.read_tool_source_range(&binding, revision, offset, limit)?;
    if page.offset_bytes != offset || page.text.len() > limit
        || offset.checked_add(page.text.len() as u64) != Some(page.next_offset_bytes)
        || page.next_offset_bytes > total
        || page.end_of_content != (page.next_offset_bytes == total)
        || (!page.end_of_content && page.next_offset_bytes == offset)
    {
        return Err("tool source export returned an invalid bounded range".into());
    }
    Ok(nola_domain::tool_source::ToolSourceExportRange {
        owner: owner.clone(), source_item_id: binding.source_item_id,
        body_sha256: binding.body_sha256, revision, offset_bytes: page.offset_bytes,
        next_offset_bytes: page.next_offset_bytes, total_utf8_bytes: total,
        text: page.text, end_of_content: page.end_of_content,
    })
}

pub fn load_page<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    id: &str,
    after: Option<i64>,
    revision: Option<i64>,
) -> Result<Option<ExportPage>, String> {
    store
        .load_export_page(id, after, revision)
        .map_err(|error| error.to_string())
}

pub fn read_range<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    id: &str,
    message: &str,
    part: &str,
    field: ExportBodyField,
    revision: i64,
    offset: u64,
    limit: usize,
) -> Result<ExportRange, String> {
    store
        .read_export_range(id, message, part, field, revision, offset, limit)
        .map_err(|error| error.to_string())
}

pub fn read_part_derivation<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    owner: &CanonicalPartOwner,
    revision: i64,
) -> Result<Option<StoredPartDerivation>, String> {
    store
        .read_part_derivation(owner, revision)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Debug)]
struct CompactionSelection {
    message_id: String,
    part_id: String,
    revision: i64,
    source_item_ids: Vec<String>,
    source_fingerprint: String,
}

fn next_export_cursor(
    requested: Option<i64>,
    returned: Option<i64>,
    has_more: bool,
    seen: &mut HashSet<i64>,
) -> Result<Option<i64>, String> {
    if !has_more {
        return Ok(None);
    }
    let cursor = returned
        .filter(|cursor| *cursor > 0 && requested.is_none_or(|prior| *cursor > prior))
        .ok_or_else(|| "canonical conversation export cursor stalled".to_string())?;
    if !seen.insert(cursor) {
        return Err("canonical conversation export cursor repeated".to_string());
    }
    Ok(Some(cursor))
}

fn read_complete_field<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
    field: ExportBodyField,
    revision: i64,
    total_utf8_bytes: u64,
) -> Result<String, String> {
    if total_utf8_bytes > COMPACTION_LINEAGE_BODY_BYTE_LIMIT as u64 {
        return Err(format!(
            "canonical {field:?} for Part {part_id} exceeds {COMPACTION_LINEAGE_BODY_BYTE_LIMIT} bytes"
        ));
    }
    let mut text = String::new();
    let mut offset = 0_u64;
    while offset < total_utf8_bytes {
        let range = read_range(
            store,
            conversation_id,
            message_id,
            part_id,
            field,
            revision,
            offset,
            EXPORT_RANGE_BYTE_LIMIT,
        )?;
        if range.offset_bytes != offset
            || range.total_utf8_bytes != total_utf8_bytes
            || range.next_offset_bytes <= offset
        {
            return Err("canonical compaction export range did not advance".to_string());
        }
        text.push_str(&range.text);
        offset = range.next_offset_bytes;
        if range.end_of_content {
            break;
        }
    }
    if offset != total_utf8_bytes || text.len() as u64 != total_utf8_bytes {
        return Err("canonical compaction export range was incomplete".to_string());
    }
    Ok(text)
}

fn source_part_preview<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    message_id: &str,
    part: &ExportPart,
    revision: i64,
    remaining_bytes: &mut usize,
    any_truncated: &mut bool,
) -> Result<Value, String> {
    let mut value = serde_json::to_value(part)
        .map_err(|error| format!("encode canonical source Part metadata failed: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "canonical source Part metadata is not an object".to_string())?;
    for (name, field, total) in [
        (
            "content_text",
            ExportBodyField::ContentText,
            Some(part.content_text_utf8_bytes),
        ),
        (
            "content_json",
            ExportBodyField::ContentJson,
            part.content_json_utf8_bytes,
        ),
    ] {
        let Some(total) = total else {
            object.insert(name.to_string(), Value::Null);
            continue;
        };
        if total == 0 {
            object.insert(name.to_string(), Value::String(String::new()));
            object.insert(format!("{name}_complete"), Value::Bool(true));
            continue;
        }
        let total_for_limit = usize::try_from(total).unwrap_or(usize::MAX);
        let limit = (*remaining_bytes)
            .min(EXPORT_RANGE_BYTE_LIMIT)
            .min(total_for_limit);
        if limit < 4 {
            *any_truncated = true;
            object.insert(name.to_string(), Value::Null);
            object.insert(format!("{name}_complete"), Value::Bool(false));
            continue;
        }
        let range = read_range(
            store,
            conversation_id,
            message_id,
            &part.id,
            field,
            revision,
            0,
            limit,
        )?;
        *remaining_bytes = remaining_bytes.saturating_sub(range.text.len());
        *any_truncated |= !range.end_of_content;
        object.insert(name.to_string(), Value::String(range.text));
        object.insert(
            format!("{name}_complete"),
            Value::Bool(range.end_of_content),
        );
    }
    Ok(value)
}

fn insert_bounded_source(
    located: &mut HashMap<String, Value>,
    source_id: String,
    value: Value,
    remaining_metadata_bytes: &mut usize,
) -> Result<(), String> {
    if let Some(existing)=located.get(&source_id) {
        if existing!=&value { return Err(format!("canonical source identity {source_id} has conflicting owners or item kinds")); }
        return Ok(());
    }
    let encoded_bytes = serde_json::to_vec(&value)
        .map_err(|error| format!("encode canonical compaction source failed: {error}"))?
        .len();
    if encoded_bytes > *remaining_metadata_bytes {
        return Err(format!(
            "compaction source metadata exceeds {COMPACTION_LINEAGE_METADATA_BYTE_LIMIT} bytes"
        ));
    }
    *remaining_metadata_bytes -= encoded_bytes;
    located.insert(source_id, value);
    Ok(())
}

/// Resolves a compaction checkpoint to its canonical source items without
/// exposing internal Compaction Parts through the ordinary UI projection.
/// Metadata is paged and revision-pinned; original bodies are bounded previews.
pub fn load_compaction_lineage<S: TranscriptExportRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    requested_part_id: Option<&str>,
) -> Result<Value, String> {
    let mut after_ordinal = None;
    let mut expected_revision = None;
    let mut seen_cursors = HashSet::new();
    let mut selection = None;

    for page_index in 0..COMPACTION_LINEAGE_MAX_PAGES {
        let page = load_page(store, conversation_id, after_ordinal, expected_revision)?
            .ok_or_else(|| format!("conversation {conversation_id} was not found"))?;
        if page.conversation.id != conversation_id {
            return Err("canonical conversation export identity mismatch".to_string());
        }
        let revision = page.conversation.revision;
        if expected_revision.is_some_and(|expected| expected != revision) {
            return Err(
                "canonical conversation changed during compaction source lookup".to_string(),
            );
        }
        expected_revision = Some(revision);
        for message in &page.messages {
            for part in &message.parts {
                if part.kind != "compaction"
                    || requested_part_id.is_some_and(|requested| requested != part.id)
                {
                    continue;
                }
                let total = part.content_json_utf8_bytes.ok_or_else(|| {
                    format!("compaction Part {} has no canonical semantic body", part.id)
                })?;
                let raw = read_complete_field(
                    store,
                    conversation_id,
                    &message.id,
                    &part.id,
                    ExportBodyField::ContentJson,
                    revision,
                    total,
                )?;
                let semantic: Value = serde_json::from_str(&raw).map_err(|error| {
                    format!("decode compaction Part {} failed: {error}", part.id)
                })?;
                let source_item_ids = semantic
                    .get("retained_item_ids")
                    .and_then(Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .map(|id| {
                                id.as_str()
                                    .filter(|id| !id.is_empty())
                                    .map(str::to_string)
                                    .ok_or_else(|| {
                                        "compaction source identity is invalid".to_string()
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let source_fingerprint = semantic
                    .get("source_fingerprint")
                    .and_then(Value::as_str)
                    .filter(|value| {
                        value.len() == 64
                            && value
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                    .ok_or_else(|| {
                        format!(
                            "compaction Part {} has an invalid source fingerprint",
                            part.id
                        )
                    })?
                    .to_string();
                if source_item_ids.is_empty() {
                    if requested_part_id.is_some() {
                        return Err(format!(
                            "compaction Part {} has no canonical source item identities",
                            part.id
                        ));
                    }
                    continue;
                }
                selection = Some(CompactionSelection {
                    message_id: message.id.clone(),
                    part_id: part.id.clone(),
                    revision,
                    source_item_ids,
                    source_fingerprint,
                });
            }
        }
        if requested_part_id.is_some() && selection.is_some() {
            break;
        }
        let Some(cursor) = next_export_cursor(
            after_ordinal,
            page.next_ordinal,
            page.has_more,
            &mut seen_cursors,
        )?
        else {
            break;
        };
        if page_index + 1 == COMPACTION_LINEAGE_MAX_PAGES {
            return Err(format!(
                "compaction source lookup exceeded {COMPACTION_LINEAGE_MAX_PAGES} canonical pages"
            ));
        }
        after_ordinal = Some(cursor);
    }

    let selection = selection.ok_or_else(|| match requested_part_id {
        Some(part_id) => format!(
            "compaction Part {part_id} was not found in canonical conversation {conversation_id}"
        ),
        None => format!(
            "conversation {conversation_id} has no compaction Part with canonical source identities"
        ),
    })?;
    let wanted = selection
        .source_item_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut located = HashMap::<String, Value>::new();
    let mut remaining_body_bytes = COMPACTION_LINEAGE_BODY_BYTE_LIMIT;
    let mut remaining_metadata_bytes = COMPACTION_LINEAGE_METADATA_BYTE_LIMIT;
    let mut any_truncated = false;
    // Tool transcript IDs are not visible Part IDs. Resolve the immutable
    // accepted source relation directly, never a prefix or latest checkpoint.
    for source_id in &selection.source_item_ids {
        let Some(binding)=store.tool_source_by_id(conversation_id,source_id,selection.revision)? else { continue; };
        let total=binding.body.total_utf8_bytes();
        // A partial JSON prefix may split a local control URL/credential. Only
        // complete bounded objects enter the existing evidence projection;
        // larger original objects remain available through the range port.
        let (body,truncated)=if total<=remaining_body_bytes as u64 {
            let raw=crate::tool_source::read_complete(store,&binding,selection.revision,remaining_body_bytes)?;
            let original:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
            let safe=nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&original).into_value();
            remaining_body_bytes=remaining_body_bytes.saturating_sub(raw.len());
            (Some(safe),false)
        } else { (None,true) };
        any_truncated |= truncated;
        insert_bounded_source(&mut located,source_id.clone(),json!({
            "source_item_id":source_id,"identity_kind":"tool_item","binding":binding,
            "original_item_preview":body,"body_truncated":truncated,
        }),&mut remaining_metadata_bytes)?;
    }
    after_ordinal = None;
    seen_cursors.clear();

    for page_index in 0..COMPACTION_LINEAGE_MAX_PAGES {
        let page = load_page(
            store,
            conversation_id,
            after_ordinal,
            Some(selection.revision),
        )?
        .ok_or_else(|| format!("conversation {conversation_id} was not found"))?;
        for message in &page.messages {
            if wanted.contains(&message.id) {
                let mut message_value = serde_json::to_value(message)
                    .map_err(|error| format!("encode canonical source Message failed: {error}"))?;
                let parts = message
                    .parts
                    .iter()
                    .map(|part| {
                        source_part_preview(
                            store,
                            conversation_id,
                            &message.id,
                            part,
                            selection.revision,
                            &mut remaining_body_bytes,
                            &mut any_truncated,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                message_value["parts"] = Value::Array(parts);
                insert_bounded_source(
                    &mut located,
                    message.id.clone(),
                    json!({
                        "source_item_id": message.id,
                        "identity_kind": "message",
                        "message": message_value,
                    }),
                    &mut remaining_metadata_bytes,
                )?;
            }
            for part in &message.parts {
                if wanted.contains(&part.id) {
                    let part_value = source_part_preview(
                        store,
                        conversation_id,
                        &message.id,
                        part,
                        selection.revision,
                        &mut remaining_body_bytes,
                        &mut any_truncated,
                    )?;
                    insert_bounded_source(
                        &mut located,
                        part.id.clone(),
                        json!({
                            "source_item_id": part.id,
                            "identity_kind": "part",
                            "message_id": message.id,
                            "part": part_value,
                        }),
                        &mut remaining_metadata_bytes,
                    )?;
                }
            }
        }
        if wanted
            .iter()
            .all(|source_id| located.contains_key(source_id))
        {
            break;
        }
        let Some(cursor) = next_export_cursor(
            after_ordinal,
            page.next_ordinal,
            page.has_more,
            &mut seen_cursors,
        )?
        else {
            break;
        };
        if page_index + 1 == COMPACTION_LINEAGE_MAX_PAGES {
            return Err(format!(
                "compaction source lookup exceeded {COMPACTION_LINEAGE_MAX_PAGES} canonical pages"
            ));
        }
        after_ordinal = Some(cursor);
    }

    let sources = selection
        .source_item_ids
        .iter()
        .filter_map(|source_id| located.get(source_id).cloned())
        .collect::<Vec<_>>();
    let missing_source_item_ids = selection
        .source_item_ids
        .iter()
        .filter(|source_id| !located.contains_key(*source_id))
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!({
        "type": "compaction_sources",
        "conversation_id": conversation_id,
        "selection": if requested_part_id.is_some() { "explicit" } else { "latest" },
        "compaction_message_id": selection.message_id,
        "compaction_part_id": selection.part_id,
        "compaction_revision": selection.revision,
        "source_fingerprint": selection.source_fingerprint,
        "source_item_ids": selection.source_item_ids,
        "sources": sources,
        "missing_source_item_ids": missing_source_item_ids,
        "source_body_byte_limit": COMPACTION_LINEAGE_BODY_BYTE_LIMIT,
        "source_bodies_are_previews": any_truncated,
    }))
}
