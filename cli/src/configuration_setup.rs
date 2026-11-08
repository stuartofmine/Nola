// SPDX-License-Identifier: Apache-2.0
//! Optional configuration setup capability, injected by the process composition root.
//! Data-only/no-op Host composition cannot acquire terminal I/O by default.
pub(crate) struct GeneratedConfiguration {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}
pub(crate) trait ConfigurationSetup {
    fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String>;
}
pub(crate) struct NoConfigurationSetup;
impl ConfigurationSetup for NoConfigurationSetup {
    fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String> { Ok(None) }
}
