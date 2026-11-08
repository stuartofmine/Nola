// SPDX-License-Identifier: Apache-2.0
// Conversation-owned managed browser controls. The client never supplies a session ID.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationBrowserSwitchRequest {
    index: u64,
}

fn conversation_browser_response(
    conversation_id: &str,
    page_index: Option<u64>,
) -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode(500), json!({"error":"conversation_store_failed"})),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {},
        Ok(None) => return not_found("conversation not found"),
        Err(_) => return json_response(StatusCode(500), json!({"error":"conversation_store_failed"})),
    }
    if page_index.is_some_and(|index| index > i32::MAX as u64) {
        return json_response(StatusCode(400), json!({"error":"invalid_browser_page_index"}));
    }
    match crate::tools::managed_browser_pages_for_conversation(conversation_id, page_index) {
        Ok(result) => json_response(StatusCode(200), browser_page_result(&result))
            .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap()),
        Err(_) => json_response(StatusCode(409), json!({"error":"browser_session_unavailable"})),
    }
}

fn browser_page_result(result: &serde_json::Map<String, Value>) -> Value {
    let pages = result.get("pages").and_then(Value::as_array).map(|items| {
        items.iter().take(20).filter_map(|item| {
            let url = item.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() { return None; }
            Some(json!({
                "index":item.get("index").and_then(Value::as_u64).unwrap_or(0),
                "url":url,
                "title":item.get("title").and_then(Value::as_str).unwrap_or(""),
                "active":item.get("active").and_then(Value::as_bool).unwrap_or(false),
            }))
        }).collect::<Vec<_>>()
    }).unwrap_or_default();
    json!({
        "pages":pages,
        "previewUrl":result.get("preview_url").and_then(Value::as_str).unwrap_or(""),
        "currentUrl":result.get("final_url").or_else(|| result.get("url")).and_then(Value::as_str).unwrap_or(""),
        "running":result.get("running").and_then(Value::as_bool)
            .or_else(|| result.get("ok").and_then(Value::as_bool)).unwrap_or(false),
    })
}

#[cfg(test)]
mod conversation_browser_tests {
    use super::*;

    #[test]
    fn browser_receipt_whitelists_display_fields_and_bounds_pages() {
        let result = json!({
            "pages":[{"index":2,"url":"https://example.test","title":"Example","active":true,"secret":"hidden"}],
            "preview_url":"http://127.0.0.1:4321/view?token=preview",
            "running":true,
            "credentials":"never publish",
        });
        let body = browser_page_result(result.as_object().unwrap());
        assert_eq!(body["pages"][0]["index"], 2);
        assert_eq!(body["pages"][0]["secret"], Value::Null);
        assert_eq!(body["credentials"], Value::Null);
        assert_eq!(body["running"], true);
        let tabs = json!({"ok":true,"pages":[]});
        assert_eq!(browser_page_result(tabs.as_object().unwrap())["running"], true);
    }
}
