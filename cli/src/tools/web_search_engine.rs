use serde_json::{Map, Value};

use super::{error, ok, ToolOutcome};


pub(super) fn execute(args: &Map<String, Value>) -> ToolOutcome {
    // HTTP search never implicitly opens or controls the user's browser.
    // Browser interaction must be a separate, authorized tool call.
    let outcome = super::web_search_core::execute(args);
    let mut mapped = if outcome.error {
        let mut mapped = error(outcome.text);
        if !outcome.display.is_empty() {
            mapped.display = outcome.display;
        }
        mapped
    } else {
        ok(outcome.text, outcome.display)
    };
    mapped.data = outcome.evidence;
    mapped
}
