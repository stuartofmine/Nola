# Optional Anthropic process backend

The manifest pins `@anthropic-ai/sandbox-runtime` to **0.0.81** (Apache-2.0,
Node >=22.12). No dependency has been installed by this change. A transitive
lockfile is not yet generated; this is not a claim of reproducible packaging.

`src/process_sandbox.rs` prepares a `Command` for the package's own CLI with an
explicit private settings file. Preparation performs read-only installation
identity checks; it does not initialize a sandbox, open proxies or start a
command. Missing/wrong dependencies, a changed frozen CLI, unsupported platforms
and unsafe environment/PATH settings are errors. It never falls back to a raw
command, looks up an ambient `srt`, starts Windows setup or inherits credentials.

The input policy is a **trusted Host execution snapshot**, not model arguments,
an access grant or a replacement permission engine. Core must already have
admitted the resources. Read access defaults to denial, paths use literal entries,
network domains are explicit, and HOME/TMP are private. The Host supplies system
runtime read roots and shared Git/secret denials rather than opening the whole
filesystem. The optional backend covers only its command and descendants.

Both production Bash paths consume the same frozen policy: foreground commands
in `tools/parts/background.rs` and durable/PTY commands in
`task_runtime_parts/execution.rs`. Missing dependencies refuse execution before
spawn, without a raw-command fallback. The trusted Host-only configuration key is
`nativeShellSandbox`, with explicit scope `native_shell`; project configuration
cannot set or clear it. Installation identity and policy are frozen into execution
and task identity. Network domains currently must be empty (deny network); no
credentials are inherited. There is no automatic installer or configuration wizard.

Prepared private resources are retained until physical settlement. Durable tasks
record their resource descriptor using the existing attempt/lease CAS before
spawn; the descriptor is observation metadata, never stop/cleanup authority.
Unknown physical outcomes keep the resources. The outer Host uses fixed system shell/utility paths and a cleared, minimal
environment, and writes status outside the sandbox; inner shell exit/pipeline evidence is read without following
symlinks. ASRT exit alone never manufactures Nola's stop ACK. Direct formatters
refuse required policy and direct execution through supervised Bash.

Native file tools, language/Python/PDF helpers, MCP/plugin processes and external
executors are outside this **native shell** scope. An external executor with this
required policy is rejected rather than launched without equivalent protection.
The retained unused Web script helper has no production callers and is not claimed
covered. External primary-session worktree preparation is a separate Core-owned,
persistent binding; worktree is not an OS sandbox. Windows is not enabled.

The focused Rust test uses synthetic **metadata files only**, never executing a
fake runtime or probing the OS sandbox. It verifies argv preservation, stable
policy identity, deny-by-default settings, environment filtering and refusal.
It is not an OS enforcement or live process-lifecycle test. Separate focused tests
verify both production entries refuse missing dependencies before any command,
and actual Git tests verify normal/bare worktree identity and source protection.

Sources: [upstream](https://github.com/anthropics/sandbox-runtime),
[CLI](https://github.com/anthropics/sandbox-runtime/blob/main/src/cli.ts),
[Claude sandbox scope](https://code.claude.com/docs/en/sandboxing).
