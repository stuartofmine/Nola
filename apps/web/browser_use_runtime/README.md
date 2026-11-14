# Browser Use execution runtime

Pinned Browser Use 1.0.2 supplies local browser execution behind Nola's existing
managed-browser protocol. Nola retains its model loop, permission admission,
conversation ownership, snapshots and preview; Playwright retains provisioning
and observation. The fixed Python worker exposes no general REPL or cloud agent.

The Mac resource build bundles Python 3.12.12 and all dependencies from
`requirements.lock`; CLI release packaging requires the same complete layout.
Development, compatibility and validation details are in the
[browser architecture](../../../docs/architecture/模块/06-工具与证据/BROWSER_AUTOMATION.md).

`test_worker.py` runs with standard Python and no external dependency, browser,
credentials or inference. Real local Chrome tests are `e2e.cjs` and
`browser_use.e2e.cjs` in the neighboring `playwright_runtime` directory.

Managed `browser_type_text` fills an owned form control. Text inputs retain their
clear/append/delay behavior. For native single-select controls, `text` must match
an existing enabled option value exactly, or a unique option label. Duplicate
labels require a value; disabled, missing or ambiguous choices fail without
changing the selection. This fixed Browser Use operation dispatches input/change
without pressing Enter or submitting a form. It retains the same owner, element
binding, revision, focus, inert and coverage checks; no model-supplied JavaScript
or general execution operation is exposed. Attached Chrome text input is
unchanged.
