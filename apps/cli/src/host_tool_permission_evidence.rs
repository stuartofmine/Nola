// SPDX-License-Identifier: Apache-2.0
//! Synchronous projection of the exact Core-admitted permission catalog.
use serde_json::Value;
thread_local! {
    static EVIDENCE: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    static MCP_DESCRIPTOR: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct EvidenceScope(Option<Value>, Option<Value>);
impl Drop for EvidenceScope {
    fn drop(&mut self) {
        EVIDENCE.with(|slot| *slot.borrow_mut() = self.0.take());
        MCP_DESCRIPTOR.with(|slot| *slot.borrow_mut() = self.1.take());
    }
}
pub(crate) fn enter(value: Value) -> EvidenceScope {
    enter_with_mcp(value, None)
}
pub(crate) fn enter_with_mcp(value: Value, descriptor: Option<Value>) -> EvidenceScope {
    EvidenceScope(EVIDENCE.with(|slot| slot.replace(Some(value))), MCP_DESCRIPTOR.with(|slot| slot.replace(descriptor)))
}
pub(crate) fn current_mcp_descriptor() -> Option<Value> { MCP_DESCRIPTOR.with(|slot| slot.borrow().clone()) }
pub(crate) fn current() -> Option<Value> { EVIDENCE.with(|slot| slot.borrow().clone()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_is_nested_and_thread_bound() {
        assert!(current().is_none());
        let _outer=enter(serde_json::json!({"catalog_revision":1}));
        {
            let _inner=enter(serde_json::json!({"catalog_revision":2}));
            assert_eq!(current().unwrap()["catalog_revision"],2);
            assert!(std::thread::spawn(current).join().unwrap().is_none());
        }
        assert_eq!(current().unwrap()["catalog_revision"],1);
        drop(_outer);
        assert!(current().is_none());
    }
}
