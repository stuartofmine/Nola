// SPDX-License-Identifier: Apache-2.0
//! Shared retained MCP transport. Callers own Core admission; transport never grants authority.
pub mod mcp_host;
pub mod mcp_stdio;
pub mod mcp_connection_authority;

pub trait Cancellation: Sync { fn is_requested(&self) -> bool; }
#[derive(Clone, Default)]
pub struct CancellationToken(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl CancellationToken {
    pub fn request(&self) { self.0.store(true, std::sync::atomic::Ordering::SeqCst); }
    pub fn is_requested(&self) -> bool { self.0.load(std::sync::atomic::Ordering::SeqCst) }
}
impl Cancellation for CancellationToken { fn is_requested(&self) -> bool { self.is_requested() } }
pub mod catalog;
pub mod mobile;
