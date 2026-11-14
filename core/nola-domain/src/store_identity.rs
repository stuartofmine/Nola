// SPDX-License-Identifier: Apache-2.0
//! Canonical local store format identity shared with update compatibility checks.
//!
//! Owning these baseline constants in Domain lets distribution Hosts verify
//! manifest compatibility without linking SQLite or concrete Store repositories.

pub const STORE_SCHEMA_EPOCH: i64 = 1;
pub const STORE_SCHEMA_VERSION: i64 = 1;
pub const STORE_APPLICATION_ID: i64 = 0x4e4f4c41;
