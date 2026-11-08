// SPDX-License-Identifier: Apache-2.0
//! Canonical branch publication is atomic with its dependency facts and caller receipt.
pub use nola_domain::{CanonicalBranchIntent, CanonicalBranchResult};
pub trait CanonicalBranchRepository {
    type Error: std::fmt::Display;
    fn create_conversation_branch(
        &mut self,
        intent: &CanonicalBranchIntent,
    ) -> Result<CanonicalBranchResult, Self::Error>;
}
