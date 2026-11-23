// SPDX-License-Identifier: Apache-2.0
// Conversation-owned PDF upload and read-only preview; never accepts paths.

const MAX_PDF_UPLOAD_BODY_BYTES: u64 = 11 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PdfResourceUploadRequest {
    display_name: String,
    mime_type: String,
    data_base64: String,
}

fn pdf_resource_error(error: &str) -> Response<Box<dyn Read + Send>> {
    let (status, code) = if error.contains("oversized") {
        (StatusCode(413), "pdf_too_large")
    } else if error.contains("quota") {
        (StatusCode(409), "resource_quota_exhausted")
    } else if error.contains("invalid") {
        (StatusCode(422), "invalid_pdf_resource")
    } else {
        (StatusCode(500), "resource_store_failed")
    };
    json_response(status, json!({"error":code,"code":code}))
}

fn put_conversation_pdf_resource_response(
    request: &mut Request,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    if request.body_length().is_some_and(|size| size as u64 > MAX_PDF_UPLOAD_BODY_BYTES) {
        return json_response(StatusCode(413), json!({"error":"pdf_too_large","maxBytes":nola_store_port::conversation_resource::MAX_PDF_RESOURCE_BYTES}));
    }
    let mut body = Vec::new();
    if request.as_reader().take(MAX_PDF_UPLOAD_BODY_BYTES + 1).read_to_end(&mut body).is_err() {
        return json_response(StatusCode(400), json!({"error":"invalid_pdf_upload"}));
    }
    if body.len() as u64 > MAX_PDF_UPLOAD_BODY_BYTES {
        return json_response(StatusCode(413), json!({"error":"pdf_too_large","maxBytes":nola_store_port::conversation_resource::MAX_PDF_RESOURCE_BYTES}));
    }
    let upload: PdfResourceUploadRequest = match serde_json::from_slice(&body) {
        Ok(upload) => upload,
        Err(_) => return json_response(StatusCode(400), json!({"error":"invalid_pdf_upload"})),
    };
    let input = nola_store_port::conversation_resource::PutConversationPdfResource {
        conversation_id: conversation_id.into(),
        resource_id: uuid::Uuid::new_v4().to_string(),
        display_name: upload.display_name,
        mime_type: upload.mime_type,
        data_base64: upload.data_base64,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Option<nola_store_port::conversation_resource::ConversationResourceDescriptor>>(
            nola_application::ApplicationCommand::PutConversationPdfResource { input },
        ));
    match result {
        Ok(Some(descriptor)) => json_response(StatusCode(200), json!({
            "resourceId":descriptor.resource_id,
            "conversationId":descriptor.conversation_id,
            "displayName":descriptor.display_name,
            "mimeType":descriptor.mime_type,
            "byteCount":descriptor.byte_count,
            "sha256":descriptor.sha256,
            "previewUrl":format!("/api/chat/conversations/{}/resources/{}", descriptor.conversation_id, descriptor.resource_id),
        })).with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap()),
        Ok(None) => not_found("conversation not found"),
        Err(error) => pdf_resource_error(&error),
    }
}

fn read_conversation_pdf_resource_response(
    conversation_id: &str,
    resource_id: &str,
    _head: bool,
) -> Response<Box<dyn Read + Send>> {
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Option<nola_store_port::conversation_resource::ConversationResourceContent>>(
            nola_application::ApplicationCommand::ReadConversationResource {
                conversation_id: conversation_id.into(), resource_id: resource_id.into(),
            },
        ));
    let Some(resource) = (match result {
        Ok(value) => value,
        Err(error) => return pdf_resource_error(&error),
    }) else { return not_found("resource not found"); };
    // tiny_http suppresses the response body for HEAD while retaining the
    // original representation length. An empty body here would report zero.
    let bytes = match general_purpose::STANDARD.decode(resource.data_base64) {
        Ok(bytes) => bytes,
        Err(_) => return json_response(StatusCode(500), json!({"error":"resource_integrity_failed"})),
    };
    let media_type = if nola_store_port::conversation_resource::is_supported_conversation_resource_mime(
        &resource.descriptor.mime_type,
    ) {
        resource.descriptor.mime_type.as_str()
    } else {
        return json_response(StatusCode(500), json!({"error":"resource_integrity_failed"}));
    };
    Response::from_data(bytes).boxed()
        .with_status_code(StatusCode(200))
        .with_header(content_type(media_type))
        .with_header(Header::from_bytes(&b"Content-Disposition"[..], &b"inline"[..]).unwrap())
        .with_header(Header::from_bytes(&b"X-Content-Type-Options"[..], &b"nosniff"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Cross-Origin-Resource-Policy"[..], &b"same-origin"[..]).unwrap())
}

fn parse_conversation_resource_ids(body: &Value) -> Result<Vec<String>, String> {
    let Some(raw) = body.get("resourceIds").or_else(|| body.get("resource_ids")) else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let Some(items) = raw.as_array() else {
        return Err("resourceIds must be an array of resource UUID strings".into());
    };
    if items.len() > 10 {
        return Err("too many conversation resources attached to one turn".into());
    }
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item.as_str().map(str::trim).filter(|value| !value.is_empty()) else {
            return Err("each resourceId must be a non-empty UUID string".into());
        };
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(format!("invalid conversation resource id `{id}`"));
        }
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn conversation_resource_extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "application/pdf" => Some("pdf"),
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn materialize_conversation_resource_bytes(
    base_dir: &std::path::Path,
    conversation_id: &str,
    descriptor: &nola_store_port::conversation_resource::ConversationResourceDescriptor,
    bytes: &[u8],
) -> Result<crate::external_execution_settings::HostAttachment, String> {
    use sha2::{Digest, Sha256};
    let ext = conversation_resource_extension(&descriptor.mime_type)
        .ok_or_else(|| format!("unsupported conversation resource media type `{}`", descriptor.mime_type))?;
    let conv_hash = format!("{:x}", Sha256::digest(conversation_id.as_bytes()));
    let dir = base_dir.join(&conv_hash[..24]);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to prepare conversation resource directory: {error}"))?;
    let file_path = dir.join(format!("{}.{ext}", descriptor.resource_id));
    let needs_write = match std::fs::metadata(&file_path) {
        Ok(meta) => meta.len() != bytes.len() as u64,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&file_path, bytes)
            .map_err(|error| format!("failed to materialize conversation resource: {error}"))?;
    }
    Ok(crate::external_execution_settings::HostAttachment {
        path: file_path,
        name: descriptor.display_name.clone(),
        mime_type: descriptor.mime_type.clone(),
        byte_count: bytes.len() as u64,
    })
}

fn materialize_conversation_resources_as_host_attachments(
    conversation_id: &str,
    resource_ids: &[String],
) -> Result<Vec<crate::external_execution_settings::HostAttachment>, String> {
    if resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let base_dir = std::env::temp_dir().join("nola-conversation-resources");
    let mut attachments = Vec::with_capacity(resource_ids.len());
    for resource_id in resource_ids {
        let content: Option<nola_store_port::conversation_resource::ConversationResourceContent> = application
            .execute(nola_application::ApplicationCommand::ReadConversationResource {
                conversation_id: conversation_id.to_string(),
                resource_id: resource_id.clone(),
            })?;
        let Some(resource) = content else {
            return Err(format!("conversation resource `{resource_id}` was not found in this conversation"));
        };
        let bytes = general_purpose::STANDARD
            .decode(&resource.data_base64)
            .map_err(|_| format!("conversation resource `{resource_id}` has invalid encoding"))?;
        attachments.push(materialize_conversation_resource_bytes(
            &base_dir,
            conversation_id,
            &resource.descriptor,
            &bytes,
        )?);
    }
    Ok(attachments)
}

fn list_conversation_resources_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<Option<Vec<nola_store_port::conversation_resource::ConversationResourceDescriptor>>>(
            nola_application::ApplicationCommand::ListConversationResources { conversation_id: conversation_id.into() },
        ));
    match result {
        Ok(Some(descriptors)) => {
            let resources = descriptors.into_iter().map(|descriptor| json!({
                "resourceId":descriptor.resource_id,
                "conversationId":descriptor.conversation_id,
                "displayName":descriptor.display_name,
                "mimeType":descriptor.mime_type,
                "byteCount":descriptor.byte_count,
                "sha256":descriptor.sha256,
                "previewUrl":format!("/api/chat/conversations/{}/resources/{}",descriptor.conversation_id,descriptor.resource_id),
            })).collect::<Vec<_>>();
            json_response(StatusCode(200),json!({"conversationId":conversation_id,"resources":resources,"localFirst":true}))
                .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
        }
        Ok(None) => not_found("conversation not found"),
        Err(error) => pdf_resource_error(&error),
    }
}

fn delete_conversation_resource_response(
    conversation_id: &str,
    resource_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let result = crate::application_port::CliApplicationPort::persistent_default()
        .and_then(|mut application| application.execute::<bool>(
            nola_application::ApplicationCommand::DeleteConversationResource {
                conversation_id: conversation_id.into(), resource_id: resource_id.into(),
            },
        ));
    match result {
        Ok(true) => json_response(StatusCode(200), json!({"ok":true})),
        Ok(false) => not_found("resource not found"),
        Err(error) => pdf_resource_error(&error),
    }
}

#[cfg(test)]
mod conversation_resource_tests {
    use super::*;

    #[test]
    fn parse_conversation_resource_ids_accepts_uuids_and_rejects_invalid() {
        let id_a = uuid::Uuid::new_v4().to_string();
        let id_b = uuid::Uuid::new_v4().to_string();
        assert_eq!(parse_conversation_resource_ids(&json!({})).unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_conversation_resource_ids(&json!({"resourceIds": [id_a, id_b, id_a]})).unwrap(),
            vec![id_a.clone(), id_b.clone()]
        );
        assert_eq!(
            parse_conversation_resource_ids(&json!({"resource_ids": [id_a]})).unwrap(),
            vec![id_a]
        );
        assert!(parse_conversation_resource_ids(&json!({"resourceIds": "not-an-array"})).is_err());
        assert!(parse_conversation_resource_ids(&json!({"resourceIds": ["not-a-uuid"]})).is_err());
        let too_many = (0..11).map(|_| uuid::Uuid::new_v4().to_string()).collect::<Vec<_>>();
        assert!(parse_conversation_resource_ids(&json!({"resourceIds": too_many})).is_err());
    }

    #[test]
    fn materialize_conversation_resource_bytes_preserves_media_type_and_bytes() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let png = b"\x89PNG\r\n\x1a\nhello-image";
        let descriptor = nola_store_port::conversation_resource::ConversationResourceDescriptor {
            conversation_id: "conv-media".into(),
            resource_id: id.clone(),
            display_name: "diagram.png".into(),
            mime_type: "image/png".into(),
            byte_count: png.len() as u64,
            sha256: "deadbeef".into(),
        };
        let attachment =
            materialize_conversation_resource_bytes(root.path(), "conv-media", &descriptor, png)
                .unwrap();
        assert_eq!(attachment.name, "diagram.png");
        assert_eq!(attachment.mime_type, "image/png");
        assert_eq!(attachment.byte_count, png.len() as u64);
        assert!(attachment.path.ends_with(format!("{id}.png")));
        assert_eq!(std::fs::read(&attachment.path).unwrap(), png);
    }
}
