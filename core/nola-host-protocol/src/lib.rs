// SPDX-License-Identifier: Apache-2.0
//! Transport-only contracts shared by Agent Core, Application, and Hosts.
//!
//! This crate deliberately owns no reducer state, persistence, permission
//! policy, or platform execution. Payloads stay generic so protocol envelopes
//! cannot acquire a reverse dependency on the Core that consumes them.

pub mod tool_result;
pub mod workspace_access;
mod runtime;
pub use runtime::{prioritize_code_cell_controls, BoundaryInputSource, HostDriver, HostExecutionProtocol, RuntimeRequestPayload, RuntimeResponsePayload};

pub use nola_domain::PermissionInstruction;
// Transport uses the canonical domain schema, not a parallel wire definition.
pub use nola_domain::host_effect::*;

mod worker_process;
pub use worker_process::{HostWorkerProcessIdentity, HostWorkerIdentity, HostWorkerLeaseIdentity, HostProcessObservation};
mod host_invocation;
mod mac_launch;
pub use mac_launch::{HostMacInputOrigin, HostMacLaunchDescriptor, HostMacLaunchAttachment};
mod host_secret;
pub use host_invocation::{
    HostRunRecoveryReference, HostSiwcAccountBinding,
    HostGenerationBinding, HostHttpModelRouteBinding, HostInvocationBinding,
    HostModelProfileReference, HostModelRouteBinding, HostNativeModelAdapter,
    HostNativeModelRouteBinding, HostOutputAdmission, HostRouteTokenLimits, ModelOutputTokenField,
    HOST_INVOCATION_BINDING_VERSION, HOST_OUTPUT_ADMISSION_VERSION,
};
pub use host_secret::{
    HostSecretAuthority, HostSecretPurpose, HostSecretSource, EMPTY_CREDENTIAL_SHA256,
};

pub mod siwc_tools;

pub mod delegation;
