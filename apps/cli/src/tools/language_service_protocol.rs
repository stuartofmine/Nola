// SPDX-License-Identifier: Apache-2.0
//! One language-service connection's protocol state. The process owner handles
//! transport deadlines and termination; this module never executes server requests.

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Query {
    Definition,
    References,
    Hover,
    DocumentSymbols,
    WorkspaceSymbols,
    Implementation,
}

impl Query {
    fn method(self) -> &'static str {
        match self {
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::Hover => "textDocument/hover",
            Self::DocumentSymbols => "textDocument/documentSymbol",
            Self::WorkspaceSymbols => "workspace/symbol",
            Self::Implementation => "textDocument/implementation",
        }
    }

    fn capability(self) -> &'static str {
        match self {
            Self::Definition => "definitionProvider",
            Self::References => "referencesProvider",
            Self::Hover => "hoverProvider",
            Self::DocumentSymbols => "documentSymbolProvider",
            Self::WorkspaceSymbols => "workspaceSymbolProvider",
            Self::Implementation => "implementationProvider",
        }
    }
}

#[derive(Debug)]
pub(super) enum Incoming {
    Initialized(Value),
    Result(Value),
    Failure(Value),
    Reply(Value),
    Notification,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { New, Initializing, Ready, Closing, Closed, Failed }

pub(super) struct Protocol {
    state: State,
    next_id: u64,
    pending: Option<u64>,
    capabilities: Value,
}

impl Default for Protocol {
    fn default() -> Self {
        Self { state: State::New, next_id: 1, pending: None, capabilities: Value::Null }
    }
}

impl Protocol {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self.pending.is_some() { return Err("language service already has a pending request".into()); }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or("language service request IDs exhausted")?;
        self.pending = Some(id);
        Ok(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
    }

    pub(super) fn initialize(&mut self, root_uri: &str) -> Result<Value, String> {
        if self.state != State::New { return Err("language service initialization already attempted".into()); }
        let request = self.request("initialize", json!({
            "processId": std::process::id(), "rootUri": root_uri,
            "capabilities": {
                "general": {"positionEncodings":["utf-16"]},
                "workspace": {"applyEdit":false, "workspaceFolders":false},
                "textDocument": {"synchronization":{"dynamicRegistration":false}}
            }
        }))?;
        self.state = State::Initializing;
        Ok(request)
    }

    pub(super) fn query(&mut self, query: Query, params: Value) -> Result<Value, String> {
        if self.state != State::Ready { return Err("language service is not ready".into()); }
        if !self.supports(query) { return Err(format!("language service does not support {}", query.method())); }
        self.request(query.method(), params)
    }

    pub(super) fn supports(&self, query: Query) -> bool {
        self.capabilities.get(query.capability())
            .is_some_and(|value| value == &Value::Bool(true) || value.is_object())
    }

    pub(super) fn require_document_sync(&self) -> Result<(), String> {
        if self.state != State::Ready { return Err("language service is not ready".into()); }
        let sync = self.capabilities.get("textDocumentSync").ok_or("server does not declare document synchronization")?;
        let kind = if sync.is_object() {
            if sync.get("openClose").and_then(Value::as_bool) != Some(true) {
                return Err("server does not support document open/close synchronization".into());
            }
            sync.get("change").and_then(Value::as_u64)
        } else { sync.as_u64() };
        if !matches!(kind, Some(1 | 2)) {
            return Err("server does not support document content synchronization".into());
        }
        Ok(())
    }

    pub(super) fn shutdown(&mut self) -> Result<Value, String> {
        if self.state != State::Ready { return Err("language service is not ready for shutdown".into()); }
        let request = self.request("shutdown", Value::Null)?;
        self.state = State::Closing;
        Ok(request)
    }

    /// Timeout/cancellation retires this connection. Late responses must never
    /// satisfy a different request or authorize an automatic query replay.
    pub(super) fn abort(&mut self) -> Option<Value> {
        self.state = State::Failed;
        self.pending.take().map(|id| json!({"jsonrpc":"2.0", "method":"$/cancelRequest", "params":{"id":id}}))
    }

    pub(super) fn receive(&mut self, message: Value) -> Result<Incoming, String> {
        let result = self.receive_checked(message);
        if result.is_err() { self.state = State::Failed; self.pending = None; }
        result
    }

    fn receive_checked(&mut self, message: Value) -> Result<Incoming, String> {
        if matches!(self.state, State::New | State::Closed | State::Failed) {
            return Err("message outside active language service connection".into());
        }
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("invalid JSON-RPC version".into());
        }
        if message.get("method").is_some() {
            if message["method"].as_str().is_none() || message.get("result").is_some() || message.get("error").is_some() {
                return Err("malformed server request/notification".into());
            }
            if let Some(id) = message.get("id") {
                if !id.is_string() && !id.is_number() { return Err("invalid server request ID".into()); }
                // No server-initiated workspace edits, shell commands or
                // dynamic registrations are executed by this client.
                return Ok(Incoming::Reply(json!({"jsonrpc":"2.0", "id":id,
                    "error":{"code":-32601, "message":"Client does not support server requests"}})));
            }
            return Ok(Incoming::Notification);
        }
        let id = message.get("id").and_then(Value::as_u64).ok_or("response lacks numeric request ID")?;
        if self.pending != Some(id) { return Err("unmatched language service response".into()); }
        if message.get("result").is_some() == message.get("error").is_some() {
            return Err("response must contain exactly one result or error".into());
        }
        if let Some(error) = message.get("error") {
            if error.get("code").and_then(Value::as_i64).is_none() || error.get("message").and_then(Value::as_str).is_none() {
                return Err("malformed language service error".into());
            }
            self.pending = None;
            if self.state != State::Ready { self.state = State::Failed; }
            return Ok(Incoming::Failure(error.clone()));
        }
        self.pending = None;
        let result = message["result"].clone();
        match self.state {
            State::Initializing => {
                let capabilities = result.get("capabilities").filter(|value| value.is_object())
                    .ok_or("initialize result lacks capabilities")?;
                if capabilities.get("positionEncoding").is_some_and(|value| value.as_str() != Some("utf-16")) {
                    return Err("server selected unsupported position encoding".into());
                }
                self.capabilities = capabilities.clone();
                self.state = State::Ready;
                Ok(Incoming::Initialized(json!({"jsonrpc":"2.0", "method":"initialized", "params":{}})))
            }
            State::Closing => { self.state = State::Closed; Ok(Incoming::Shutdown) }
            State::Ready => Ok(Incoming::Result(result)),
            _ => Err("response in invalid protocol state".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Protocol {
        let mut protocol = Protocol::default();
        protocol.initialize("file:///workspace").unwrap();
        assert!(matches!(protocol.receive(json!({"jsonrpc":"2.0","id":1,"result":{
            "capabilities":{"definitionProvider":true}
        }})).unwrap(), Incoming::Initialized(_)));
        protocol
    }

    #[test]
    fn capabilities_and_response_identity_are_required() {
        let mut protocol = ready();
        assert!(protocol.query(Query::References, json!({})).is_err());
        assert_eq!(protocol.query(Query::Definition, json!({})).unwrap()["id"], 2);
        assert!(protocol.query(Query::Definition, json!({})).is_err());
        assert!(protocol.receive(json!({"jsonrpc":"2.0","id":3,"result":[]})).is_err());
        assert!(protocol.query(Query::Definition, json!({})).is_err());
    }

    #[test]
    fn reverse_requests_do_not_complete_or_modify_pending_query() {
        let mut protocol = ready();
        protocol.query(Query::Definition, json!({})).unwrap();
        let Incoming::Reply(reply) = protocol.receive(json!({"jsonrpc":"2.0","id":2,
            "method":"workspace/applyEdit","params":{"edit":{}}})).unwrap() else { panic!("expected rejection") };
        assert_eq!(reply["error"]["code"], -32601);
        assert!(matches!(protocol.receive(json!({"jsonrpc":"2.0","id":2,"result":[]})).unwrap(), Incoming::Result(_)));
    }

    #[test]
    fn cancellation_retires_connection_and_shutdown_is_explicit() {
        let mut protocol = ready();
        protocol.query(Query::Definition, json!({})).unwrap();
        assert_eq!(protocol.abort().unwrap()["params"]["id"], 2);
        assert!(protocol.receive(json!({"jsonrpc":"2.0","id":2,"result":[]})).is_err());
        let mut protocol = ready();
        protocol.shutdown().unwrap();
        assert!(matches!(protocol.receive(json!({"jsonrpc":"2.0","id":2,"result":null})).unwrap(), Incoming::Shutdown));
        assert!(protocol.query(Query::Definition, json!({})).is_err());
    }
}
