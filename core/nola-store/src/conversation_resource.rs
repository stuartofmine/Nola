// SPDX-License-Identifier: Apache-2.0
//! Bounded local PDF previews with canonical conversation ownership.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use nola_store_port::conversation_resource::{
    ConversationResourceContent, ConversationResourceDescriptor, ConversationResourceRepository,
    PutConversationPdfResource, MAX_PDF_RESOURCE_BYTES,
};
use sha2::{Digest, Sha256};

use super::{params, NolaStore, OptionalExtension, StoreError, StoreResult};

const MAX_RESOURCES_PER_CONVERSATION: i64 = 10;
const MAX_GLOBAL_RESOURCE_BYTES: i64 = 256 * 1024 * 1024;

fn invalid(reason: &str) -> StoreError {
    StoreError::Conflict(reason.to_string())
}

fn validate_pdf(bytes: &[u8]) -> StoreResult<()> {
    if bytes.len() > MAX_PDF_RESOURCE_BYTES {
        return Err(invalid("oversized PDF resource"));
    }
    if bytes.len() < 16 || !bytes.starts_with(b"%PDF-")
        || !bytes[bytes.len().saturating_sub(1024)..].windows(5).any(|part| part == b"%%EOF")
    {
        return Err(invalid("invalid PDF resource"));
    }
    Ok(())
}

fn validate_identity(input: &PutConversationPdfResource) -> StoreResult<()> {
    if input.conversation_id.trim().is_empty()
        || uuid::Uuid::parse_str(&input.resource_id).is_err()
        || input.mime_type != "application/pdf"
        || input.display_name.is_empty()
        || input.display_name.len() > 255
        || input.display_name.chars().any(char::is_control)
        || input.display_name.contains(['/', '\\'])
        || !input.display_name.to_ascii_lowercase().ends_with(".pdf")
        || input.created_at_ms <= 0
    {
        return Err(invalid("invalid PDF resource metadata"));
    }
    Ok(())
}

impl ConversationResourceRepository for NolaStore {
    type Error = StoreError;

    fn put_conversation_pdf_resource(
        &mut self,
        input: &PutConversationPdfResource,
    ) -> StoreResult<Option<ConversationResourceDescriptor>> {
        validate_identity(input)?;
        if input.data_base64.len() > (MAX_PDF_RESOURCE_BYTES + 2) / 3 * 4 + 8 {
            return Err(invalid("oversized PDF resource"));
        }
        let bytes = STANDARD.decode(&input.data_base64).map_err(|_| invalid("invalid PDF encoding"))?;
        validate_pdf(&bytes)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let descriptor = ConversationResourceDescriptor {
            conversation_id: input.conversation_id.clone(),
            resource_id: input.resource_id.clone(),
            display_name: input.display_name.clone(),
            mime_type: input.mime_type.clone(),
            byte_count: bytes.len() as u64,
            sha256: digest.clone(),
        };
        let transaction = self.write_transaction()?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            params![input.conversation_id], |row| row.get(0),
        )?;
        if !exists {
            transaction.commit()?;
            return Ok(None);
        }
        let previous: Option<(String, String, String, i64, String)> = transaction.query_row(
            "SELECT conversation_id,display_name,mime_type,byte_count,sha256 FROM conversation_pdf_resources WHERE resource_id=?1",
            params![input.resource_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?;
        if let Some((owner, name, mime, count, previous_digest)) = previous {
            if owner != input.conversation_id || name != input.display_name || mime != input.mime_type
                || count != bytes.len() as i64 || previous_digest != digest
            {
                return Err(invalid("PDF resource identity is already bound to different content"));
            }
            transaction.commit()?;
            return Ok(Some(descriptor));
        }
        let (count, conversation_bytes): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*),COALESCE(SUM(byte_count),0) FROM conversation_pdf_resources WHERE conversation_id=?1",
            params![input.conversation_id], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let global_bytes: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(byte_count),0) FROM conversation_pdf_resources", [], |row| row.get(0),
        )?;
        if count >= MAX_RESOURCES_PER_CONVERSATION
            || conversation_bytes.saturating_add(bytes.len() as i64) > MAX_RESOURCES_PER_CONVERSATION * MAX_PDF_RESOURCE_BYTES as i64
            || global_bytes.saturating_add(bytes.len() as i64) > MAX_GLOBAL_RESOURCE_BYTES
        {
            return Err(invalid("PDF resource quota exhausted"));
        }
        transaction.execute(
            "INSERT INTO conversation_pdf_resources(resource_id,conversation_id,display_name,mime_type,byte_count,sha256,body,created_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![input.resource_id,input.conversation_id,input.display_name,input.mime_type,bytes.len() as i64,digest,bytes,input.created_at_ms],
        )?;
        transaction.commit()?;
        Ok(Some(descriptor))
    }

    fn read_conversation_resource(
        &self,
        conversation_id: &str,
        resource_id: &str,
    ) -> StoreResult<Option<ConversationResourceContent>> {
        let value: Option<(String, String, i64, String, Vec<u8>)> = self.connection.query_row(
            "SELECT display_name,mime_type,byte_count,sha256,body FROM conversation_pdf_resources
             WHERE conversation_id=?1 AND resource_id=?2",
            params![conversation_id,resource_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?;
        let Some((display_name,mime_type,byte_count,sha256,bytes)) = value else { return Ok(None); };
        validate_pdf(&bytes)?;
        if byte_count != bytes.len() as i64 || sha256 != format!("{:x}", Sha256::digest(&bytes)) {
            return Err(StoreError::Invariant("PDF resource integrity check failed".into()));
        }
        Ok(Some(ConversationResourceContent {
            descriptor: ConversationResourceDescriptor {
                conversation_id: conversation_id.into(), resource_id: resource_id.into(),
                display_name, mime_type, byte_count: byte_count as u64, sha256,
            },
            data_base64: STANDARD.encode(bytes),
        }))
    }

    fn list_conversation_resources(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<Vec<ConversationResourceDescriptor>>> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            params![conversation_id], |row| row.get(0),
        )?;
        if !exists { return Ok(None); }
        let mut statement = self.connection.prepare(
            "SELECT resource_id,display_name,mime_type,byte_count,sha256
             FROM conversation_pdf_resources WHERE conversation_id=?1
             ORDER BY created_at_ms DESC,resource_id ASC LIMIT 10",
        )?;
        let resources = statement.query_map(params![conversation_id], |row| {
            Ok(ConversationResourceDescriptor {
                conversation_id: conversation_id.into(), resource_id: row.get(0)?,
                display_name: row.get(1)?, mime_type: row.get(2)?,
                byte_count: row.get::<_,i64>(3)? as u64, sha256: row.get(4)?,
            })
        })?.collect::<Result<Vec<_>,_>>()?;
        Ok(Some(resources))
    }

    fn delete_conversation_resource(&mut self, conversation_id: &str, resource_id: &str) -> StoreResult<bool> {
        let transaction = self.write_transaction()?;
        let deleted = transaction.execute(
            "DELETE FROM conversation_pdf_resources WHERE conversation_id=?1 AND resource_id=?2",
            params![conversation_id,resource_id],
        )? > 0;
        transaction.commit()?;
        Ok(deleted)
    }
}
