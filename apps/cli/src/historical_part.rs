// SPDX-License-Identifier: Apache-2.0
//! Bounded historical detail query shared by explicit CLI and Web actions.
use nola_protocol::HistoricalProjectionCommand;
use nola_protocol::{CanonicalPartOwner, HistoricalPartProjection};

pub fn read(
    application: &mut crate::application_port::CliApplicationPort,
    owner: CanonicalPartOwner,
    revision: i64,
) -> Result<HistoricalPartProjection, String> {
    if revision < 1 {
        return Err("historical detail requires a positive conversation revision".into());
    }
    let result: Option<HistoricalPartProjection> =
        application.execute(HistoricalProjectionCommand::ReadHistoricalPart {
            owner: owner.clone(),
            expected_conversation_revision: revision,
            maximum_output_bytes: 32768,
        })?;
    let result = result.ok_or("historical Part has no structured original")?;
    if result.owner != owner || result.canonical_revision != revision {
        return Err("historical detail response changed owner or revision".into());
    }
    if serde_json::to_vec(&result)
        .map_err(|error| error.to_string())?
        .len()
        > 32768
    {
        return Err("historical detail response exceeds byte budget".into());
    }
    Ok(result)
}
