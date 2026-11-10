// Inventory reads can fail with Application Service stderr or private paths.
// Never infer a storage failure category by parsing diagnostic text.
fn conversation_inventory_failure(_diagnostic: &str) -> Value {
    json!({
        "code": "conversation_inventory_unavailable",
        "error": "Conversation inventory is unavailable. Check local service diagnostics."
    })
}

#[cfg(test)]
mod public_web_error_tests {
    use super::*;

    #[test]
    fn conversation_inventory_errors_never_expose_service_or_storage_diagnostics() {
        for diagnostic in [
            "Nola Application Service exited with exit status: 1: schema rejected; last health probe: /private/owner/service.sock",
            "Nola Application Service did not become ready at /private/service.sock: private stderr",
            "parse web conversation failed: private database content",
        ] {
            assert_eq!(conversation_inventory_failure(diagnostic), json!({
                "code": "conversation_inventory_unavailable",
                "error": "Conversation inventory is unavailable. Check local service diagnostics."
            }));
        }
    }
}
