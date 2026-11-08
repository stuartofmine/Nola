use super::*;

#[test]
fn device_egress_scope_guards_are_neither_send_nor_sync() {
    // If a guard implements Send/Sync, both impls match and these compile-time
    // assertions become ambiguous, failing compilation rather than runtime.
    trait AmbiguousIfSend<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
    trait AmbiguousIfSync<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}
    let _ = <crate::tools::DeviceBridgeResolverGuard as AmbiguousIfSend<_>>::check;
    let _ = <crate::tools::DeviceBridgeResolverGuard as AmbiguousIfSync<_>>::check;
    let _ = <crate::tools::DeviceBridgeToolsGuard as AmbiguousIfSend<_>>::check;
    let _ = <crate::tools::DeviceBridgeToolsGuard as AmbiguousIfSync<_>>::check;
}

fn tool(now: u64) -> DeviceBridgeRuntimeTool {
    DeviceBridgeRuntimeTool {
        schema: json!({"type":"function","function":{"name":"device_write_file","parameters":{"type":"object"}},"x_nola":{"target":"device"}}),
        capability: "files_write".into(),
        result_egress_receipt: nola_core::tool_policy::RuntimeResultEgressReceipt {
            tool_id: "device_write_file".into(),
            route: ToolRoute::Device,
            manifest_id: "manifest".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            issued_at_ms: now,
            expires_at_ms: now + 60_000,
            allow_result_egress: true,
        },
        result_egress_grant: nola_core::tool_policy::ResultEgressGrant {
            tool_id: "device_write_file".into(),
            route: ToolRoute::Device,
            manifest_id: "manifest".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            expires_at_ms: now + 30_001,
        },
    }
}

fn request() -> ToolRequest {
    ToolRequest {
        id: new_id("egress-effect"),
        name: "device_write_file".into(),
        args: Default::default(),
    }
}

#[test]
fn device_egress_resolver_identity_is_scoped_and_empty_identity_is_rejected() {
    use crate::tools::{current_device_bridge_execution_identity, enter_device_bridge_resolver};
    let before = current_device_bridge_execution_identity();
    let identity = |session: &str| nola_core::tool_policy::RuntimeIdentity {
        host_id: Some("host".into()),
        client_id: Some("client".into()),
        session_id: Some(session.into()),
    };
    let outer =
        enter_device_bridge_resolver(identity("outer"), Arc::new(|_, _| panic!("not executing")))
            .unwrap();
    assert!(
        enter_device_bridge_resolver(identity(" "), Arc::new(|_, _| panic!("not executing")))
            .is_err()
    );
    assert_eq!(
        current_device_bridge_execution_identity()
            .unwrap()
            .session_id
            .as_deref(),
        Some("outer")
    );
    {
        let _inner = enter_device_bridge_resolver(
            identity("inner"),
            Arc::new(|_, _| panic!("not executing")),
        )
        .unwrap();
        assert_eq!(
            current_device_bridge_execution_identity()
                .unwrap()
                .session_id
                .as_deref(),
            Some("inner")
        );
    }
    assert_eq!(
        current_device_bridge_execution_identity()
            .unwrap()
            .session_id
            .as_deref(),
        Some("outer")
    );
    drop(outer);
    assert_eq!(
        serde_json::to_value(current_device_bridge_execution_identity()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn device_egress_disconnected_dispatch_retires_only_its_owned_pending_proof() {
    let _lock = crate::store::test_env_lock();
    let challenge_count = HOST_DEVICE_TOOL_CHALLENGES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .len();
    let waiter_count = HOST_DEVICE_TOOL_WAITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .len();
    let (tx, rx) = mpsc::channel();
    drop(rx);
    let outcome = wait_for_device_tool(
        &request(),
        &tool(device_bridge_now_ms()),
        "host",
        "client",
        "session",
        &tx,
        None,
    );
    assert!(outcome.error);
    assert_eq!(
        HOST_DEVICE_TOOL_CHALLENGES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .len(),
        challenge_count
    );
    assert_eq!(
        HOST_DEVICE_TOOL_WAITERS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .len(),
        waiter_count
    );
}

#[test]
fn device_egress_rejects_expired_mismatched_and_denied_proof_before_dispatch() {
    let _lock = crate::store::test_env_lock();
    for fault in [
        "expired", "host", "client", "session", "manifest", "tool", "route", "denied",
    ] {
        let mut tool = tool(device_bridge_now_ms());
        match fault {
            "expired" => tool.result_egress_grant.expires_at_ms = device_bridge_now_ms(),
            "host" => tool.result_egress_receipt.host_id = "other".into(),
            "client" => tool.result_egress_receipt.client_id = "other".into(),
            "session" => tool.result_egress_receipt.session_id = "other".into(),
            "manifest" => tool.result_egress_grant.manifest_id = "other".into(),
            "tool" => tool.result_egress_receipt.tool_id = "device_read_file".into(),
            "route" => tool.result_egress_receipt.route = ToolRoute::AgentHost,
            _ => tool.result_egress_receipt.allow_result_egress = false,
        }
        let (tx, rx) = mpsc::channel();
        let result =
            wait_for_device_tool(&request(), &tool, "host", "client", "session", &tx, None);
        assert!(result.error, "{fault}");
        assert!(result.artifacts.is_empty());
        assert!(rx.try_recv().is_err(), "no device event for {fault}");
    }
}

#[test]
fn device_egress_actual_wait_bounds_deadline_and_rejects_wrong_result_before_publication() {
    let _lock = crate::store::test_env_lock();
    let tool = tool(device_bridge_now_ms());
    let deadline = tool.result_egress_grant.expires_at_ms;
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        wait_for_device_tool(&request(), &tool, "host", "client", "session", &tx, None)
    });
    let frame = String::from_utf8(rx.recv_timeout(StdDuration::from_secs(2)).unwrap()).unwrap();
    let event: Value = serde_json::from_str(frame.trim().strip_prefix("data: ").unwrap()).unwrap();
    let nonce = event["nonce"].as_str().unwrap();
    let pending = HOST_DEVICE_TOOL_CHALLENGES
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .get(nonce)
        .unwrap()
        .clone();
    assert_eq!(pending.expires_at, deadline / 1_000);
    assert!(pending.validate_at(deadline - 1).is_ok());
    assert!(pending.validate_at(deadline).is_err());
    let sender = HOST_DEVICE_TOOL_WAITERS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .get(nonce)
        .unwrap()
        .clone();
    sender
        .send(nola_core::device_bridge::DeviceToolResult {
            protocol_version: nola_core::device_bridge::DEVICE_BRIDGE_PROTOCOL_VERSION,
            nonce: nonce.into(),
            request_id: pending.request_id.clone(),
            host_id: "other-host".into(),
            client_id: pending.client_id.clone(),
            session_id: pending.session_id.clone(),
            tool_name: pending.tool_name.clone(),
            output: json!({"text":"must never escape"}),
            error: false,
        })
        .unwrap();
    let outcome = worker.join().unwrap();
    assert!(outcome.error);
    assert!(!outcome.text.contains("must never escape"));
    assert!(outcome.artifacts.is_empty());
    assert!(!HOST_DEVICE_TOOL_CHALLENGES
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .contains_key(nonce));
}
