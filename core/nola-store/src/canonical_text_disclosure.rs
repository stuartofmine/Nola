// SPDX-License-Identifier: Apache-2.0
//! Ordinary transcript disclosure, separate from preservation of original bytes.
//! Review is derived at publication, bound to an invalidated content generation.
use crate::canonical_field_reader::CanonicalFieldReader;
use crate::semantic_json_projection::{ExtractedValue, ScalarValue, TokenKind};
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::PartBodyField;

const DISCLOSURE_QUERY: &str = "SELECT d.evaluated_generation,d.generation,d.conversation_id,d.message_id,d.backing_json,d.decision,
    CASE WHEN length(CAST(r.reference_json AS BLOB))<=4096 THEN r.reference_json END,
    r.part_id,m.status,p.state
    FROM message_parts p JOIN messages m ON m.id=p.message_id
    JOIN canonical_part_disclosures d ON d.part_id=p.id
    LEFT JOIN message_part_body_references r ON r.part_id=p.id AND r.field='content_json'
    WHERE m.conversation_id=?1 AND m.id=?2 AND p.id=?3
    AND COALESCE((SELECT e.visible FROM message_projection_events e
        JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
        WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
        ORDER BY o.resulting_revision DESC LIMIT 1),1)=1";

/// Only absent/null review or explicit approval may become ordinary prose.
/// Unknown, duplicated, deferred or incorrectly typed review remains withheld.
/// Original fields are retained for explicit private audit/export workflows.
pub(super) fn allows_product_text(
    connection: &Connection,
    conversation: &str,
    message: &str,
    part: &str,
) -> StoreResult<bool> {
    let row: Option<(
        Option<i64>,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
    )> = connection
        .query_row(
            DISCLOSURE_QUERY,
            params![conversation, message, part],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        evaluated,
        generation,
        owner_conversation,
        owner_message,
        backing,
        decision,
        current_backing,
        bound,
        message_state,
        part_state,
    )) = row
    else {
        return Err(invalid("canonical disclosure fact or owner is missing"));
    };
    if evaluated != Some(generation)
        || owner_conversation.as_deref() != Some(conversation)
        || owner_message.as_deref() != Some(message)
        || backing != current_backing
    {
        return Err(invalid(
            "canonical disclosure generation or backing is not published",
        ));
    }
    // The publication generation proves the inline slot has not changed since
    // derivation (including inline/reference conflicts). Do not open that slot
    // here: even computing its byte length can materialize a large JSON value.
    // Sealed bindings retain their independent lifecycle and physical-owner
    // validation; oversized references never become absence.
    if bound.is_some() {
        if !matches!(part_state.as_str(), "complete" | "completed")
            || (message_state != "completed"
                && !crate::canonical_message_ownership::has_trusted_resumption(
                    connection,
                    conversation,
                    message,
                )?)
        {
            return Err(invalid("published disclosure reference is not finalized"));
        }
        let reference: nola_domain::CanonicalBodyReference = serde_json::from_str(
            current_backing
                .as_deref()
                .ok_or_else(|| invalid("oversized published field reference"))?,
        )?;
        if reference.conversation_id() != conversation
            || reference.message_id() != message
            || reference.part_id() != part
            || reference.field() != PartBodyField::ContentJson
        {
            return Err(invalid("published disclosure reference owner mismatch"));
        }
        crate::canonical_text_store::validate_reference_from_connection(connection, &reference)?;
    }
    match decision.as_deref() {
        Some("allow") => Ok(true),
        Some("withheld") => Ok(false),
        _ => Err(invalid("canonical disclosure source is invalid")),
    }
}

fn derive(
    connection: &Connection,
    conversation: &str,
    message: &str,
    part: &str,
) -> StoreResult<bool> {
    let field =
        CanonicalFieldReader::open_disclosure_source(connection, conversation, message, part)?;
    let Some(projection) = field.extract_json_scalar_fields(&["review"], 512)? else {
        return Ok(true);
    };
    if projection.root_kind != TokenKind::Object {
        return Ok(false);
    }
    let Some(review) = projection.fields.first() else {
        return Ok(true);
    };
    if review.duplicate_known_key {
        return Ok(false);
    }
    Ok(match &review.value {
        ExtractedValue::Complete(ScalarValue::Null) => true,
        ExtractedValue::Complete(ScalarValue::String(value)) => value == "approved",
        _ => false,
    })
}

/// Every official mutation settles through this boundary. Triggers only mark
/// changed JSON/backing/owners, so an empty queue is an indexed constant probe.
pub(super) fn flush(connection: &Connection) -> StoreResult<()> {
    loop {
        let pending: Vec<(String,i64)> = connection.prepare(
            "SELECT part_id,generation FROM canonical_part_disclosures WHERE evaluated_generation IS NULL ORDER BY part_id LIMIT 64")?
            .query_map([], |row| Ok((row.get(0)?,row.get(1)?)))?.collect::<Result<_,_>>()?;
        if pending.is_empty() {
            return Ok(());
        }
        for (part, generation) in pending {
            let (conversation,message,kind,backing): (String,String,String,Option<String>) = connection.query_row(
                "SELECT m.conversation_id,p.message_id,p.kind,
                 CASE WHEN length(CAST(r.reference_json AS BLOB))<=4096 THEN r.reference_json END
                 FROM message_parts p JOIN messages m ON m.id=p.message_id
                 LEFT JOIN message_part_body_references r ON r.part_id=p.id AND r.field='content_json' WHERE p.id=?1",
                [&part], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
            let decision = if kind != "text" {
                "withheld"
            } else {
                match derive(connection, &conversation, &message, &part) {
                    Ok(true) => "allow",
                    Ok(false) => "withheld",
                    Err(StoreError::Invariant(_)) => "invalid",
                    Err(error) => return Err(error),
                }
            };
            if connection.execute(
                "UPDATE canonical_part_disclosures SET evaluated_generation=generation,
                conversation_id=?3,message_id=?4,backing_json=?5,decision=?6
                WHERE part_id=?1 AND generation=?2 AND evaluated_generation IS NULL",
                params![part, generation, conversation, message, backing, decision],
            )? != 1
            {
                return Err(invalid("canonical disclosure changed during publication"));
            }
            // Classify changed provider content once at publication rather than
            // reparsing all historical tool output for every tail query. Invalid
            // or not-yet-published content has no fact and keeps read-time errors.
            connection.execute("DELETE FROM canonical_provider_classifications WHERE part_id=?1", [&part])?;
            if kind == "provider_message" {
                match crate::conversation_read_store::part_has_provider_tool_result(connection, &conversation, &message, &part) {
                    Ok(has_tool_result) => {
                        connection.execute("INSERT INTO canonical_provider_classifications
                            SELECT p.id,?2,m.status,p.state,?3 FROM message_parts p
                            JOIN messages m ON m.id=p.message_id WHERE p.id=?1",
                            params![part,generation,has_tool_result])?;
                    }
                    Err(StoreError::Invariant(_)) | Err(StoreError::Json(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
#[path = "canonical_text_disclosure_tests.rs"]
mod tests;
