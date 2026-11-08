// SPDX-License-Identifier: Apache-2.0
//! Payload-free command identity for diagnostics, never authorization or routing.
pub trait CommandMetadata {
    fn wire_command_name(&self) -> &'static str;
}
