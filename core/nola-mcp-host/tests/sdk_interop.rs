// SPDX-License-Identifier: Apache-2.0
//! Opt-in reference SDK acceptance, separate from hermetic protocol fixtures.
//! NOLA_MCP_SDK_PYTHON=<venv>/bin/python cargo test -p nola_mcp_host --test sdk_interop -- --ignored
use nola_core::agent::capability::McpTransportKind;
use nola_mcp_host::{mcp_connection_authority as authority, mcp_host as host};
use serde_json::json;
use std::{
    collections::BTreeMap,
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn exercise(http: bool) {
    let python = std::env::var("NOLA_MCP_SDK_PYTHON")
        .expect("Set an isolated Python with mcp 1.x installed");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/e2e/mcp_sdk_fixture.py");
    let mut process = None;
    let transport = if http {
        McpTransportKind::StreamableHttp
    } else {
        McpTransportKind::Stdio
    };
    let config = if http {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        process = Some(Server(
            Command::new(&python)
                .arg(&fixture)
                .args(["streamable-http", &port.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        ));
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                process.as_mut().unwrap().0.try_wait().unwrap().is_none(),
                "SDK server exited"
            );
            assert!(Instant::now() < deadline, "SDK startup timed out");
            std::thread::sleep(Duration::from_millis(30));
        }
        json!({"server_id":"sdk-http", "endpoint":format!("http://127.0.0.1:{port}/mcp"), "timeout_ms":10000})
    } else {
        json!({"server_id":"sdk-stdio", "type":"stdio", "command":python, "args":[fixture], "timeout_ms":10000})
    };
    let bound =
        authority::bind_configurations(&[config], "sdk-interop", &uuid::Uuid::new_v4().to_string())
            .unwrap();
    host::register_configured_servers(&bound).unwrap();
    let reference = bound[0]["connection_ref"].as_str().unwrap();
    assert!(host::call_tool(
        transport,
        reference,
        "echo",
        json!({"value":"before approval"})
    )
    .is_err());
    let expected = host::connection_resource(reference).unwrap();
    let connection = host::connect_approved(reference, &expected).unwrap();
    assert_eq!(connection.server.tools.len(), 1);
    assert_eq!(connection.server.resources.len(), 1);
    assert_eq!(connection.server.prompts.len(), 1);
    let result = host::call_tool(
        transport,
        reference,
        "echo",
        json!({"value":"你好 MCP — 42"}),
    )
    .unwrap();
    assert!(!result.is_error);
    assert!(result.output.to_string().contains("你好 MCP — 42"));
    assert!(
        host::read_resource(transport, reference, "fixture://greeting")
            .unwrap()
            .output
            .to_string()
            .contains("你好 Nola resource")
    );
    assert!(host::get_prompt(
        transport,
        reference,
        "welcome",
        BTreeMap::from([("name".into(), "Nola".into())])
    )
    .unwrap()
    .output
    .to_string()
    .contains("Welcome Nola"));
    drop(host::revoke_connection(transport, reference).unwrap());
    assert!(host::call_tool(
        transport,
        reference,
        "echo",
        json!({"value":"after revocation"})
    )
    .is_err());
    drop(process);
}

#[test]
#[ignore = "requires isolated official MCP Python SDK"]
fn official_sdk_stdio_catalog_tools_resources_prompts_and_revocation() {
    exercise(false);
}

#[test]
#[ignore = "requires isolated official MCP Python SDK"]
fn official_sdk_http_catalog_tools_resources_prompts_and_revocation() {
    exercise(true);
}
