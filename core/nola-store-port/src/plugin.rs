// SPDX-License-Identifier: Apache-2.0
use nola_domain::plugin::PluginInstallation;

pub trait PluginRepository {
    type Error: std::fmt::Display;
    fn has_write_unit_of_work(&self) -> bool;
    /// Includes tombstones so reinstall cannot reset optimistic revisions.
    fn get(&self, id: &str) -> Result<Option<PluginInstallation>, Self::Error>;
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<PluginInstallation>, Self::Error>;
    fn save(&mut self, record: &PluginInstallation, expected_revision: Option<i64>) -> Result<(), Self::Error>;
}

pub trait PluginRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: PluginRepository<Error = Self::Error> where Self: 'a;
    fn plugin_repository(&mut self) -> Self::Repository<'_>;
}
