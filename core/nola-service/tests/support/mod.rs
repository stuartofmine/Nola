// SPDX-License-Identifier: Apache-2.0
//! Reuse the real Core TaskPromoted source fixture, never a caller-authored proof.
pub type ApplicationRuntime = nola_application::ApplicationRuntime<nola_store::NolaStore>;
#[allow(dead_code)]
#[path = "../../../nola-application/tests/support/agent_task_admission.rs"]
pub mod agent_task_admission;

pub fn open_runtime(path: impl AsRef<std::path::Path>) -> Result<ApplicationRuntime, String> {
    let path = path.as_ref();
    let store = nola_store::NolaStore::open(path).map_err(|error| error.to_string())?;
    let uploads = Box::new(nola_store::content_upload_port::LazyContentUploadStore::for_application(
        path, nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES,
    ));
    Ok(nola_application::ApplicationRuntime::from_store(store, uploads))
}
