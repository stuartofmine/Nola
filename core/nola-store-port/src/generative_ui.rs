// SPDX-License-Identifier: Apache-2.0
//! Surface persistence joins the existing command receipt/publication transaction.
use nola_domain::generative_ui::{UiSurfaceIdentity, UiSurfaceState, UiSurfaceOwner};

pub trait GenerativeUiRepository {
    type Error: std::fmt::Display;
    fn ui_authority_id(&self) -> Result<String, Self::Error>;
    fn ui_tool_owner(&self, run_id: &str, turn_id: &str, call_id: &str) -> Result<UiSurfaceOwner, Self::Error>;
    /// Includes deleted generations; called within the publication transaction.
    fn next_ui_generation(&self, owner: &UiSurfaceOwner, surface_id: &str) -> Result<u64, Self::Error>;
    fn ui_tool_publication(&self, effect_id: &str) -> Result<Option<(String, String)>, Self::Error>;
    fn record_ui_tool_publication(&mut self, effect_id: &str, request_digest: &str, result_json: &str) -> Result<(), Self::Error>;
    fn read_ui_surface(&self, identity: &UiSurfaceIdentity) -> Result<Option<UiSurfaceState>, Self::Error>;
    /// Exact immutable historical version; never substitutes the current state.
    fn read_ui_surface_revision(&self, identity: &UiSurfaceIdentity, revision: u64) -> Result<Option<UiSurfaceState>, Self::Error>;
    /// Caller must own a command unit of work. Never commits an outer receipt.
    fn write_ui_surface(&mut self, expected_revision: u64, state: &UiSurfaceState) -> Result<(), Self::Error>;
    /// Publish only a reference and readable summary inside the Surface transaction.
    fn write_ui_surface_part(&mut self, snapshot: &nola_domain::generative_ui::UiSnapshot, occurred_at_ms: i64) -> Result<(), Self::Error>;
    fn list_ui_surfaces(&self, authority_id: &str, conversation_id: &str) -> Result<Vec<UiSurfaceState>, Self::Error>;
}
