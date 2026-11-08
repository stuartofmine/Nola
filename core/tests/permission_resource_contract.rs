use nola_core::agent::permissions::{permission_resource_for_input, permission_resource_keys};
use serde_json::json;

#[test]
fn arbitrary_capability_names_do_not_define_permission_semantics() {
    for capability in [
        "vendor_read_everything",
        "writeish-calendar",
        "memory_like_extension",
        "chrome_clone",
        "搜索所有文件",
    ] {
        assert!(
            permission_resource_keys(capability).is_empty(),
            "{capability} acquired permission semantics from its name"
        );
    }
}

#[test]
fn permission_domain_labels_do_not_define_external_resource_shape() {
    let input = json!({"path": "/tmp/a", "query": "current"});
    assert_eq!(
        permission_resource_for_input("opaque-capability-17", Some("filesystem.read"), &input,),
        input.to_string(),
    );
    assert_eq!(
        permission_resource_for_input("vendor.calendar.v2", Some("network"), &input),
        input.to_string(),
    );
}

#[test]
fn versioned_builtin_ids_keep_their_declared_resource_shape() {
    assert_eq!(
        permission_resource_keys("read_file"),
        &["path", "root", "cwd", "source", "paths"]
    );
    // Memory writes bind consent to the complete proposed content; projecting
    // only scope/kind would silently authorize a different payload.
    assert!(permission_resource_keys("memory_save").is_empty());
    let input = json!({"scope":"global", "kind":"preference", "content":"first"});
    assert_eq!(permission_resource_for_input("memory_save", Some("memory_save"), &input), input.to_string());
    assert_ne!(permission_resource_for_input("memory_save", Some("memory_save"), &input),
        permission_resource_for_input("memory_save", Some("memory_save"),
            &json!({"scope":"global", "kind":"preference", "content":"changed"})));
}
