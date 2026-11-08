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
    Response::from_data(bytes).boxed()
        .with_status_code(StatusCode(200))
        .with_header(content_type("application/pdf"))
        .with_header(Header::from_bytes(&b"Content-Disposition"[..], &b"inline"[..]).unwrap())
        .with_header(Header::from_bytes(&b"X-Content-Type-Options"[..], &b"nosniff"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Cross-Origin-Resource-Policy"[..], &b"same-origin"[..]).unwrap())
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
