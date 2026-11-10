// SPDX-License-Identifier: Apache-2.0
fn model_connection_inventory_response() -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        let mut connections: Value = application.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::ListConnections })?;
        let profiles: Value = application.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::ListProfiles })?;
        let selections: Vec<Value> = profiles.as_array().into_iter().flatten().filter_map(|profile| {
            if !profile["deleted_at_ms"].is_null() { return None; }
            let source: Value = serde_json::from_str(profile["source_json"].as_str()?).ok()?;
            Some(json!({"connection_id":source["connectionID"].as_str()?, "catalog_key":source["catalogKey"].as_str()?,
                "enabled":profile["enabled"].as_bool().unwrap_or(false), "selected":profile["selected"].as_bool().unwrap_or(false)}))
        }).collect();
        let mut accounts: Value = application.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::ListAccounts })?;
        // Even opaque vault references have no presentation purpose.
        if let Some(values) = accounts.as_array_mut() {
            for value in values { if let Some(account) = value.get_mut("account").and_then(Value::as_object_mut) { account.remove("credential_ref"); } }
        }
        let store = NolaStore::open_default()?;
        if let Some(items) = connections.as_array_mut() {
            for item in items {
                if let Ok(snapshot) = serde_json::from_value::<nola_domain::model_connection::ModelConnectionSnapshot>(item.clone()) {
                    let mut bindings = serde_json::Map::new();
                    for protocol in &snapshot.document.connection.protocols {
                        if let Ok((binding, _)) = crate::model_capability_probe::credential_binding(&store, &snapshot.document.connection, *protocol) {
                            bindings.insert(protocol.as_str().into(), json!(binding));
                        }
                    }
                    item["probe_credential_bindings"] = Value::Object(bindings);
                }
            }
        }
        Ok(json!({"connections":connections, "accounts":accounts, "selections":selections}))
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(500), json!({"code":"model_connections_unavailable", "error":"Could not load model connections"})),
    }
}

fn model_gateway_origins(config: &Config) -> Vec<String> {
    let mut origins = vec![format!("http://localhost:{}", config.web_port), format!("http://127.0.0.1:{}", config.web_port),
        format!("http://[::1]:{}", config.web_port)];
    if !matches!(config.web_host.as_str(), "0.0.0.0" | "::") {
        let host = if config.web_host.contains(':') { format!("[{}]", config.web_host) } else { config.web_host.clone() };
        origins.push(format!("http://{host}:{}", config.web_port));
    }
    origins
}

fn model_gateway_access_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        let store = NolaStore::open_default()?;
        if request.method() == &Method::Get {
            return serde_json::to_value(crate::model_gateway_host::access_snapshot(&store)?).map_err(|_| "invalid gateway state".into());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input { expected_revision: i64, revoke: bool }
        let input: Input = serde_json::from_value(read_json_value(request).map_err(|_| "invalid gateway input")?).map_err(|_| "invalid gateway input")?;
        if input.revoke {
            serde_json::to_value(crate::model_gateway_host::revoke_access_token(&store, input.expected_revision)?).map_err(|_| "invalid gateway state".into())
        } else {
            serde_json::to_value(crate::model_gateway_host::issue_access_token(&store, input.expected_revision)?).map_err(|_| "invalid gateway token".into())
        }
    })();
    let response = match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"error":"gateway_access_unconfirmed"})),
    };
    response.with_header(Header::from_bytes("Cache-Control", "no-store").expect("static header"))
}

fn model_client_connection_response(request: &mut Request, config: &Config, action: &str) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let store = NolaStore::open_default()?;
        match action {
            "plan" => {
                let input = serde_json::from_value(value).map_err(|_| "invalid client plan")?;
                let base = format!("http://localhost:{}/gateway/v1", config.web_port);
                serde_json::to_value(crate::model_client_adapter::plan_client_connection(&store, input, &base)?).map_err(|_| "invalid plan".into())
            },
            "commit" => {
                #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Input { plan_id: String }
                let input: Input = serde_json::from_value(value).map_err(|_| "invalid client confirmation")?;
                serde_json::to_value(crate::model_client_adapter::commit_client_connection(&store, &input.plan_id)?).map_err(|_| "invalid receipt".into())
            },
            "disconnect" => {
                #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Input { receipt_id: String }
                let input: Input = serde_json::from_value(value).map_err(|_| "invalid disconnect")?;
                serde_json::to_value(crate::model_client_adapter::disconnect_client_connection(&store, &input.receipt_id)?).map_err(|_| "invalid receipt".into())
            },
            _ => Err("unknown client operation".into()),
        }
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"client_connection_unconfirmed","error":"Configuration update could not be confirmed. Review the file and request a fresh plan."})),
    }
}

fn model_catalog_discover_response(request: &mut Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input { request_id: String, connection_id: String, expected_revision: i64 }
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let input: Input = serde_json::from_value(value).map_err(|_| "invalid discovery request")?;
        let snapshot = crate::model_connection_host::discover_catalog_request(&NolaStore::open_default()?, &input.connection_id,
            input.expected_revision, &model_gateway_origins(config), &input.request_id)?;
        serde_json::to_value(snapshot).map_err(|_| "invalid discovery result".into())
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"catalog_unavailable","error":"Catalog discovery unavailable; saved models are unchanged"})),
    }
}

fn model_catalog_cancel_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Input { request_id: String, connection_id: String, expected_revision: i64 }
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<bool, String> {
        let input: Input = serde_json::from_value(value).map_err(|_| "invalid cancellation request")?;
        crate::model_catalog_discovery::cancel(&NolaStore::open_default()?, &input.request_id,
            &input.connection_id, input.expected_revision)
    })();
    match result {
        Ok(cancelled) => json_response(StatusCode(200), json!({"cancelled":cancelled})),
        Err(_) => json_response(StatusCode(409), json!({"code":"catalog_cancel_unconfirmed","error":"Cancellation could not be confirmed. Refresh the saved catalog before retrying."})),
    }
}

fn source_management_peer_allowed(peer: Option<&std::net::SocketAddr>, tailcat_ingress: bool) -> bool {
    web_loopback_is_trusted(peer, tailcat_ingress)
}

fn source_management_request_allowed(request: &Request, tailcat_ingress: bool) -> bool {
    if tailcat_ingress { return false; }
    let unique = |name: &str| {
        let mut headers = request.headers().iter().filter(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name));
        let value = headers.next()?.value.as_str();
        if headers.next().is_some() {None} else {Some(value)}
    };
    let Some(host) = unique("Host") else {return false};
    let Some(origin) = unique("Origin") else {return false};
    let Some(content_type) = unique("Content-Type") else {return false};
    let Ok(host_url) = reqwest::Url::parse(&format!("http://{host}")) else {return false};
    // This listener is plain HTTP; TLS reverse proxies cannot assert a local
    // browser origin. Do not equate localhost with any other host spelling.
    request.method() == &Method::Post
        && source_management_peer_allowed(request.remote_addr(), tailcat_ingress)
        && matches!(host_url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))
        && host_url.username().is_empty() && host_url.password().is_none()
        && host_url.path() == "/" && host_url.query().is_none() && host_url.fragment().is_none()
        && origin == format!("http://{host}")
        && web_request_is_same_origin(request)
        && content_type.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json")
}

#[cfg(test)]
mod source_management_security_tests {
    use super::*;
    #[test]
    fn model_source_management_requires_exact_local_browser_origin_and_json() {
        use std::io::{Read, Write};
        assert!(!source_management_peer_allowed(None, false));
        assert!(!source_management_peer_allowed(Some(&"192.0.2.1:1234".parse().unwrap()), false));
        assert!(source_management_peer_allowed(Some(&"[::1]:1234".parse().unwrap()), false));
        let cases = [
            ("POST", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\n", "application/json", true),
            ("POST", "localhost:9911", "Origin: http://localhost:9911\r\n", "application/json; charset=utf-8", true),
            ("POST", "attacker.invalid:9911", "Origin: http://attacker.invalid:9911\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: https://127.0.0.1:9911\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: http://127.0.0.1:9912\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: http://localhost:9911\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: null\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\n", "text/plain", false),
            ("POST", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\nOrigin: http://attacker.invalid\r\n", "application/json", false),
            ("POST", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\nSec-Fetch-Site: cross-site\r\n", "application/json", false),
            ("GET", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\n", "application/json", false),
            ("HEAD", "127.0.0.1:9911", "Origin: http://127.0.0.1:9911\r\n", "application/json", false),
        ];
        for (method, host, origin, content_type, allowed) in cases {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let address = server.server_addr().to_ip().unwrap();
            let worker = std::thread::spawn(move || {
                let mut request = server.recv().unwrap();
                assert_eq!(source_management_request_allowed(&request, false), allowed);
                // Malformed body fails before opening any user store. Every
                // source handler shares this gate and redacted parser.
                let response = model_opencode_login_response(&mut request, false, "owners");
                assert_eq!(response.status_code().0, if allowed {400} else {403});
                request.respond(response).unwrap();
            });
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            write!(stream, "{method} / HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: {content_type}\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{{").unwrap();
            let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
            assert!(response.to_ascii_lowercase().contains("cache-control: no-store"));
            worker.join().unwrap();
        }
    }
}

fn source_management_denied() -> Response<Box<dyn Read + Send>> {
    let mut response = json_response(StatusCode(403), json!({"code":"source_login_local_same_origin_required"}));
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    response
}

fn read_source_management_json(request: &mut Request) -> Result<Value, Response<Box<dyn Read + Send>>> {
    let mut bytes = Vec::new();
    let result = request.as_reader().take(256 * 1024 + 1).read_to_end(&mut bytes);
    if result.is_ok() && bytes.len() <= 256 * 1024 {
        if let Ok(value) = serde_json::from_slice(&bytes) { return Ok(value); }
    }
    let mut response = json_response(StatusCode(400), json!({"code":"source_management_invalid_json"}));
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    Err(response)
}

fn model_source_login_response(request: &mut Request, tailcat_ingress: bool, action: &str) -> Response<Box<dyn Read + Send>> {
    // Source app-server authentication is a local-only integration, not a
    // commercial/hosted credential relay. Keep existing Web auth and CSRF too.
    if !source_management_request_allowed(request, tailcat_ingress) {
        return source_management_denied();
    }
    let value = match read_source_management_json(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let store = NolaStore::open_default()?;
        let snapshot = if action == "start" {
            crate::model_source_login::start(&store, serde_json::from_value(value).map_err(|_| "invalid login request")?)?
        } else {
            crate::model_source_login::inspect(&store, serde_json::from_value(value).map_err(|_| "invalid login handle")?, action == "cancel")?
        };
        let mut public = serde_json::to_value(snapshot).map_err(|_| "invalid login result")?;
        if let Some(account) = public.get_mut("account").and_then(|v|v.get_mut("account")).and_then(Value::as_object_mut) { account.remove("credential_ref"); }
        Ok(public)
    })();
    let mut response = match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"source_login_unconfirmed","error":"Source login could not be confirmed. No inference was enabled."})),
    };
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    response
}

fn model_opencode_login_response(request: &mut Request, tailcat_ingress: bool, action: &str) -> Response<Box<dyn Read + Send>> {
    if !source_management_request_allowed(request, tailcat_ingress) {
        return source_management_denied();
    }
    let value = match read_source_management_json(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        use crate::model_opencode_login as source;
        let store = NolaStore::open_default()?;
        let result = match action {
            "owners" => serde_json::to_value(source::owners(&store)?),
            "connect" => serde_json::to_value(source::connect(&store, serde_json::from_value(value).map_err(|_| "invalid owner")?)?),
            "inspect" => serde_json::to_value(source::inspect(&store, serde_json::from_value(value).map_err(|_| "invalid owner")?)?),
            "authorize" => serde_json::to_value(source::authorize(&store, serde_json::from_value(value).map_err(|_| "invalid authorization")?)?),
            "callback" | "forget" => serde_json::to_value(source::callback(&store, serde_json::from_value(value).map_err(|_| "invalid callback")?, action == "forget")?),
            _ => return Err("invalid operation".into()),
        };
        let mut public = result.map_err(|_| "invalid source result")?;
        if let Some(account) = public.get_mut("account").and_then(|v| v.get_mut("account")).and_then(Value::as_object_mut) { account.remove("credential_ref"); }
        Ok(public)
    })();
    let mut response = match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"opencode_login_unconfirmed","error":"OpenCode owner or authorization could not be confirmed. Do not retry an uncertain authorization automatically. No inference was enabled."})),
    };
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    response
}

fn model_kimi_login_response(request: &mut Request, tailcat_ingress: bool, action: &str) -> Response<Box<dyn Read + Send>> {
    if !source_management_request_allowed(request, tailcat_ingress) {
        return source_management_denied();
    }
    let value = match read_source_management_json(request) {Ok(value) => value, Err(response) => return response};
    let result = (|| -> Result<Value,String> {
        let store = NolaStore::open_default()?;
        if action == "owners" { return serde_json::to_value(crate::model_kimi_login::owners(&store)?).map_err(|_| "invalid Kimi owners".into()); }
        let result = if action == "connect" {
            crate::model_kimi_login::connect(&store, serde_json::from_value(value).map_err(|_| "invalid Kimi connection")?)?
        } else {
            crate::model_kimi_login::operate(&store, action, serde_json::from_value(value).map_err(|_| "invalid Kimi operation")?)?
        };
        let mut public = serde_json::to_value(result).map_err(|_| "invalid Kimi result")?;
        if let Some(account) = public.get_mut("account").and_then(|v|v.get_mut("account")).and_then(Value::as_object_mut) {account.remove("credential_ref");}
        Ok(public)
    })();
    let mut response = match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"kimi_owner_unconfirmed","error":"Kimi service identity or login could not be confirmed. Check the selected local service. No inference was enabled."})),
    };
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    response
}

fn model_source_account_response(request: &mut Request, tailcat_ingress: bool, refresh: bool) -> Response<Box<dyn Read + Send>> {
    if !source_management_request_allowed(request, tailcat_ingress) { return source_management_denied(); }
    let value = match read_source_management_json(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let store = NolaStore::open_default()?;
        if refresh {
            #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Input { account_id: String, expected_revision: i64 }
            let input: Input = serde_json::from_value(value).map_err(|_| "invalid source account")?;
            serde_json::to_value(crate::model_source_account::refresh_source_account(&store, &input.account_id, input.expected_revision)?).map_err(|_| "invalid account state".into())
        } else {
            let input = serde_json::from_value(value).map_err(|_| "invalid source account")?;
            serde_json::to_value(crate::model_source_account::bind_source_account(&store, input)?).map_err(|_| "invalid account state".into())
        }
    })();
    let mut response = match result {
        Ok(mut value) => {
            if let Some(account) = value.get_mut("account").and_then(|value|value.get_mut("account")).and_then(Value::as_object_mut) { account.remove("credential_ref"); }
            json_response(StatusCode(200), value)
        },
        Err(_) => json_response(StatusCode(409), json!({"code":"source_account_unconfirmed","error":"Account status could not be confirmed"})),
    };
    response.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    response
}

fn model_import_response(request: &mut Request, config: &Config, commit: bool) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let origins = model_gateway_origins(config);
    let result = (|| -> Result<Value, String> {
        let store = NolaStore::open_default()?;
        if commit {
            let input = serde_json::from_value(value).map_err(|_| "invalid import confirmation")?;
            serde_json::to_value(crate::model_connection_host::commit_import(&store, input, &origins)?).map_err(|_| "invalid import result".into())
        } else {
            let input = serde_json::from_value(value).map_err(|_| "invalid import plan")?;
            serde_json::to_value(crate::model_connection_host::plan_import(&store, input, &origins)?).map_err(|_| "invalid import plan result".into())
        }
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"model_import_unconfirmed", "error":"Import could not be confirmed. Review the source and request a fresh plan."})),
    }
}

fn model_capability_probe_response(request: &mut Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let result = (|| -> Result<Value, String> {
        let input = serde_json::from_value(value).map_err(|_| "invalid probe confirmation")?;
        serde_json::to_value(crate::model_capability_probe::probe(&NolaStore::open_default()?, input,
            &model_gateway_origins(config))?).map_err(|_| "invalid probe result".into())
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"model_probe_unconfirmed",
            "error":"Text capability was not confirmed. The provider may have charged for a request; no automatic retry was sent. Reload before trying again."})),
    }
}

fn public_model_connection_command(value: Value) -> Result<nola_application::ModelCommand, String> {
    let command: nola_application::ModelCommand = serde_json::from_value(value).map_err(|_| "invalid connection command")?;
    // Authentication lifecycle, credentials and provider observations belong to
    // the Host adapters; browser input cannot assert those facts.
    if !matches!(command, nola_application::ModelCommand::ConfigureConnectionModels { .. }
        | nola_application::ModelCommand::SetConnectionCapabilityOverride { .. }) {
        return Err("connection command is not exposed to the browser".into());
    }
    Ok(command)
}

fn model_connection_command_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let command = match public_model_connection_command(value) {
        Ok(command) => command,
        Err(_) => return json_response(StatusCode(422), json!({"code":"invalid_connection_command", "error":"Invalid connection command"})),
    };
    match crate::application_port::CliApplicationPort::persistent_default().and_then(|mut application|
        application.execute::<Value>(nola_application::ApplicationCommand::Model { command })) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(_) => json_response(StatusCode(409), json!({"code":"connection_command_unconfirmed", "error":"Refresh the connection before retrying"})),
    }
}

#[cfg(test)]
mod model_connection_http_contract_tests {
    use super::*;
    #[test]
    fn browser_cannot_assert_credentials_or_discovery_evidence() {
        for value in [json!({"type":"list_accounts"}), json!({"type":"save_account", "input":{
            "id":"account", "display_name":"Account", "source":"codex", "credential_ref":"vault-ref", "state":"ready"
        }, "expected_revision":null, "updated_at_ms":1}), json!({"type":"refresh_connection_catalog", "connection_id":"connection",
            "expected_revision":1, "models":[], "observed_at_ms":1})] {
            assert!(public_model_connection_command(value).is_err());
        }
        assert!(public_model_connection_command(json!({"type":"configure_connection_models", "connection_id":"connection",
            "expected_revision":1, "enabled_model_ids":["responses:model"], "selected_model_id":null, "updated_at_ms":1})).is_ok());
    }
}
