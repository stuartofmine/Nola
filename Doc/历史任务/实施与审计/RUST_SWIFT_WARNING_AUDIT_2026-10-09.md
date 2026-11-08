> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Rust and Swift warning audit — 2026-10-09

Current status: source correction and the native Rust/Apple validation matrix
are complete. Earlier “pending” entries below are
chronological preparation records, not the current result. Final scope and
remaining external diagnostics are recorded in the closing section.

## Rust scope

Inventory: `cargo metadata --no-deps --locked --format-version 1`, 19 workspace packages, no package-defined feature switches. Target counts include each declared integration test and example (and the CLI build script); library unit tests are compiled by `--all-targets` too.

| Package | Manifest | Declared targets | Target kinds |
| --- | --- | ---: | --- |
| `nola_core` | `core/Cargo.toml` | 39 | rlib, test |
| `nola_domain` | `core/nola-domain/Cargo.toml` | 4 | lib, test |
| `nola_host_protocol` | `core/nola-host-protocol/Cargo.toml` | 1 | lib |
| `nola_kernel` | `core/nola-kernel/Cargo.toml` | 2 | lib, test |
| `nola_application` | `core/nola-application/Cargo.toml` | 85 | lib, example, test |
| `nola_legacy_agent_adapter` | `core/nola-legacy-agent-adapter/Cargo.toml` | 1 | lib |
| `nola_protocol` | `core/nola-protocol/Cargo.toml` | 1 | lib |
| `nola_store_port` | `core/nola-store-port/Cargo.toml` | 1 | lib |
| `nola_store` | `core/nola-store/Cargo.toml` | 42 | lib, test |
| `nola_application_bootstrap` | `core/nola-application-bootstrap/Cargo.toml` | 2 | lib, example |
| `nola-code-memory` | `core/nola-code-memory/Cargo.toml` | 1 | lib |
| `nola_credential_host` | `core/nola-credential-host/Cargo.toml` | 1 | lib |
| `nola_mobile_bindings` | `core/nola-mobile-bindings/Cargo.toml` | 1 | staticlib, cdylib, rlib |
| `nola_model_runtime_host` | `core/nola-model-runtime-host/Cargo.toml` | 1 | lib |
| `nola_runtime_driver` | `core/nola-runtime-driver/Cargo.toml` | 2 | lib, test |
| `nola_runtime_supervisor` | `core/nola-runtime-supervisor/Cargo.toml` | 1 | lib |
| `nola_service_client` | `core/nola-service-client/Cargo.toml` | 2 | lib, test |
| `nola_service` | `core/nola-service/Cargo.toml` | 27 | lib, bin, test |
| `nola_cli` | `cli/Cargo.toml` | 20 | bin, test, custom-build |

The patched `cli/vendor/tiny_http` is excluded from workspace membership, but remains a compiled dependency. Native host coverage does not claim Android/iOS target validation. The mobile-bindings library has rlib, staticlib, and cdylib outputs. Service and CLI binary targets are included in both checks below.

## Rust source fixes

Baseline evidence: `/tmp/nola-goal-model-domain-rust-schema-retry-20261009.log` and recent macOS build logs. This is a warning cleanup, not blanket lint suppression.

- Core TaskProjection: removed crate-private legacy queue/acknowledgement and retry/reclaim mutators and their private backoff constants/helper. Workspace call-site search found only their obsolete unit-test callers (the same-named memory-runtime retry method is unrelated and unchanged). Production uses accepted guidance and lease-recovery facts. Migrated guidance completion/idempotency coverage to `reconcile_guidance`; expanded authoritative lease-recovery coverage to assert retry count/deadline and reject early claims. No public API removed.
- Application: removed unused private guidance conversion function; removed unused projection-test imports and bounded-serializer test formatting import. Production guidance imports remain because canonical delivery still uses them.
- Service: use atomic `try_update` in place of deprecated `fetch_update`, retaining orderings and bounded admission closure.
- CLI: signal-handler conversion now explicitly casts through a function pointer; checkpoint-reason and tool-schema imports are test-only. Production tool classification and device-bridge imports remain required by included source files.

No Swift source changes in this Rust cleanup.

## Rust validation gate

Source preparation complete; compilation/tests are pending coordinated execution. Metadata inventory is not compilation proof. Do not infer zero warnings from earlier targeted tests.

Run sequentially with the repository's existing shared cache and bounded workers:

```sh
. scripts/lib/rust_build_cache.sh
nola_rust_build_cache_init
export CARGO_BUILD_JOBS=2
nola_cargo check --locked --workspace --all-targets
nola_cargo check --locked --workspace --release --lib --bins
nola_cargo test --locked -p nola_core --lib agent::task::tests
nola_cargo test --locked -p nola_service --lib connection_budget::tests
```

Record terminal exit codes and warning diagnostics for each profile; fix any newly exposed source warnings and repeat affected gates. Do not change RUSTFLAGS, clean caches, or add blanket allows. Cross-target checks and dependency warnings should be reported separately from workspace source warnings.

## Swift scope and validation

Project inventory includes `NolaMac`, `NolaMacTests`, `NolaMLXRuntime`, `Nola`,
`NolaTests`, `NolaUITests`, `NolaMessageFilter`, and the `NolaCore` dependency.
Mac builds must cover Debug build-for-testing and Release; iOS builds must cover
Debug build-for-testing on an installed simulator and Release on generic iOS
(code signing disabled). The standalone MLX request/response test scripts compile
four additional Swift test sources outside the Xcode test target.

`xcrun simctl list devices available` on 2026-10-09 confirmed iPhone 17 Pro,
UUID `B7D92F7F-2A00-432A-9032-EC7B57B382D4`, iOS 26.0. Use an explicit simulator
destination, never a connected physical device. Reuse the existing Mac derived data
at `/tmp/nola-r43-mac-derived.9hojKS`; serialize heavy builds and cap Cargo workers
at two. Full compilation coverage must be established from build actions, not from
an incremental log's absence of diagnostics.

Initial Swift test fixes preserve isolation and test semantics: parser closures
declare their actual MainActor/Sendable contract, the suspended preparation fixture
is MainActor-isolated, asynchronous construction crosses the actor explicitly,
unused results are intentionally discarded, and weak deallocation probes remain
weak. External E2E marker cleanup now defers to the test scope rather than exiting
an immediately nested scope. No blanket warning suppression is used.

The AppIntents metadata tool reports that extraction was skipped for targets with
no AppIntents dependency. Track this tooling diagnostic separately; do not add an
unused framework merely to hide it. Product Swift source fixes and full matrix
results are pending. No Swift warning-cleanliness claim is made yet.

### Product Swift preparation

The first source cleanup is complete, awaiting compilation:

- Workspace resolves actor-owned default services inside its MainActor initializer;
  injected readiness closures declare the same actor as their implementation.
- Capability decoding, context command DTOs, backend policy, remote task conversion,
  and data-source JSON/date conversion are explicitly nonisolated where they only
  transform values. UI/service ownership is not moved off the main actor.
- The default-model notification name is immutable and nonisolated.
- Transcript scroll monitoring returns the original AppKit event outside
  `MainActor.assumeIsolated`; the actor closure handles UI state and returns Void,
  rather than carrying a non-Sendable NSEvent across the generic result boundary.

### First Rust diagnostic sweep

`/tmp/nola-warning-rust-matrix-20261009.log` records successful Debug all-targets
and Release library/binary checks. The all-targets sweep exposed additional test-only
unused imports, variables, mutability, and shared fixture helpers. These are now
cleaned up as a coherent test-only batch, pending repeat validation. The changed
core task unit suite passed 11 tests and service admission passed 3 tests.

Release additionally exposed a compiler-tool installation problem: `rust-objcopy`
cannot load its relative `libLLVM.dylib` while stripping the CLI build script.
The toolchain root contains LLVM, but the tool's expected adjacent library is absent.
This is not a source warning and is not resolved by suppressing diagnostics or
disabling stripping. The subsequent official component repair is recorded below.

The initial chain exited 0, including all 11 core task and 3 service admission
tests. Read-only inspection confirmed rust-objcopy's sole LLVM rpath points to
the host-local library directory. Installed the matching official `llvm-tools`
component with rustup (exit 0); it supplies that library, and the exact previously
failing `rust-objcopy --version` now succeeds with LLVM 23.1.1 / Rust 1.99.0.
`rust-toolchain.toml` now declares this build prerequisite so another checkout
does not require an undocumented local repair. No hand-made symlink, DYLD override,
global compiler flag, or stripping disablement was added. Release stripping still
requires a subsequent actual build verification; a cached prior warning is not
new compiler execution evidence.

Actual tool verification: the repaired objcopy successfully stripped debug data
from the current native `target/debug/nola-service` into an isolated temporary
output (`/tmp/nola-objcopy-validation.Jcxiha/nola-service-stripped`, exit 0).
The original executable was not modified. Cargo's check-profile build-script path
is an mbx shell shim, not an object file, so it is not a valid stripping fixture.

### Mac full-module diagnostic pass

`/tmp/nola-warning-mac-debug-20261009.log` completed build-for-testing with exit 0
using `SWIFT_COMPILATION_MODE=wholemodule`, including the MLX tool and Mac tests.
It revealed one additional source diagnostic: synthesized Equatable on
`MacDataSourceContract` inherited MainActor isolation. The pure contract, read
recipe and related value enums now declare nonisolated consistently; the mutable
registry itself remains MainActor-owned. Final full-module rerun is in progress
at `/tmp/nola-warning-mac-debug-final-20261009.log`.

The installed legacy XCBuild and SwiftBuild AppIntents specifications expose no
supported extraction opt-out for these targets. Deployment-aware processing is
not an opt-out. The two metadata-skipped messages are recorded as informational
tooling diagnostics, not source warnings; no guessed setting or dummy framework
dependency has been added.

Final Mac Debug full-module build-for-testing exited 0. Its log contains no Swift
source warning or error, only the two classified AppIntents tooling messages.
The same artifacts then passed the combined UI/domain regression at
`/tmp/nola-warning-mac-ui-regression-20261009.log`: 443 executed, 0 failures,
1 explicitly opt-in system-picker skip. This includes streaming reveal and
completion scroll, model editing/default submission, data-source contracts,
remote journal ingestion, preparation cancellation, process ownership, and
long-conversation performance. No timeout or performance threshold was relaxed.

The remaining Apple matrix has started sequentially: Mac Release, iOS simulator
Debug build-for-testing (both test targets), generic iOS Release, and both standalone
MLX scripts. Logs use `/tmp/nola-warning-{mac-release,ios-debug,ios-release,
mlx-generation,mlx-response}-20261009.log`; they are not yet passing evidence.

Mac Release has now completed successfully (`nola-warning-mac-release-20261009.log`).
No Swift/Rust source diagnostic appeared. It did expose seven strict-prototype
warnings in our hand-maintained C FFI header, pending the next frozen-source edit
batch: four valid zero-argument exports need `(void)`; three header-only declarations
have neither implementation nor call sites and should be removed, not presented as
working APIs. The iOS Debug build is running; the whole chain is not terminal yet.

The same Release pass reports four C++17-extension diagnostics inside mlx-swift
0.31.3 (`61b9e011e09a62b489f6bd647958f1555bdf2896`) generated Metal headers and
one AppIntents metadata-skip message. Local Metal preprocessing on the exact SDK,
compiler and unchanged macOS 14 deployment target proved that both the default and
explicit `-std=metal3.1` define `__METAL_VERSION__=310`, `__cplusplus=201402L`.
Thus adding that setting cannot fix the dependency's use of `if constexpr`.
Do not raise deployment requirements, patch a transient package checkout, or
suppress diagnostics to disguise these dependency-owned C++/Metal warnings.

iOS simulator Debug full-module build-for-testing also completed successfully
(`nola-warning-ios-debug-20261009.log`), including both test targets. The message
filter target does not appear in this actual build graph; do not infer its coverage
from the name of the empty embedding phase. Unlike the Mac target this build exposed source warnings:
value DTO conformances inheriting MainActor, actor information lost at injected
profile closures, actor-bound configuration/default helpers, generic streaming
iterators crossing concurrent protocol requirements, unnecessary awaits, and
eight test diagnostics. These are not treated as passing warning-clean evidence.
Product contracts, streaming adapters, and tests are being audited separately;
source remains frozen until the running iOS Release/MLX chain exits.

The initial Apple chain exited 0 in full: iOS Release built successfully and both
standalone MLX scripts passed (generation request, response/tool envelope, HTTP
writer lifecycle, and streaming request contracts). iOS Release repeated the same
product isolation warnings found in Debug; success is not a warning-clean claim.
With the chain terminal, the coordinated iOS source/test correction batch began.

FFI header cleanup is now implemented: the four real zero-argument exports use
`(void)`; three header-only, unimplemented and unreferenced model-selection/service
getter declarations were removed. No implemented export or ABI layout changed.
Both `xcrun clang -std=c11 -Werror=strict-prototypes -fsyntax-only -Icore/swift_shims
NolaCore/NolaCore.c` and the C++17 syntax check passed. The next Apple rebuild must
still verify the actual importer and framework packaging.

Rust final native compilation and all affected shared-fixture consumer regressions
are running at `/tmp/nola-warning-rust-final-20261009.log`, independently of the
Swift source edits. No simultaneous Xcode build is running during this phase.

The final native Rust chain exited 0: Debug all-targets is source-warning clean;
the six application fixture suites passed 47 tests, store ledger passed 15, and
service cold-restart context usage passed 1 (63 total). Release completed without
compiling new work and replayed its old objcopy diagnostic from cache in 0.25 s;
this is tracked separately from the successfully repaired tool and fresh Mac
Release build. Investigate a scoped recompile rather than deleting diagnostics
or changing source merely to alter a cache key.

The iOS correction batch is now implemented and frozen. Pure payloads/settings/
history/transport/render-block values declare nonisolated contracts; configuration
resolver callbacks preserve MainActor ownership, and runtime adapters match their
existing locked service. Generic byte iterators use `next(isolation:)`, with the
bounded collector inheriting caller isolation explicitly. Parsing, byte limits and
cancellation are unchanged. Tests retain their original nonisolated consumption
where meaningful and explicitly await actor-bound setup; notification assertions
were strengthened instead of deleting fixture work. Full iOS Debug revalidation
is running at `/tmp/nola-warning-ios-debug-final-20261009.log`.

That iOS revalidation exited 65: the resolved application compiler target is
`arm64-apple-ios17.0-simulator`, despite a project-level 26.0 setting. Four uses of
`next(isolation:)` require iOS 18. This gate correctly rejected the proposed API;
the iterator correction is being revised for the existing deployment contract,
not by increasing the minimum OS or adding warning suppression. Other source/test
isolation fixes remain in place, but are not yet fully revalidated.

The stale Rust diagnostic was traced precisely to `target/check/release`, not
`target/release` (the fresh native Release artifacts were already clean). Official
Cargo dry-run identified 29 CLI check-only generated files, 944.8 KiB. Removed
only that package/profile under the explicit check target directory, preserving
dependencies, ordinary executables and the shared mbx cache. A direct Cargo check
with process-local empty wrappers is now rebuilding those generated artifacts at
`/tmp/nola-warning-rust-cli-fresh-20261009.log`; no diagnostic file was edited to
manufacture a clean result. Generated check files are recoverable by rebuilding.

That direct CLI Release check exited 0 in 1m20s with no warnings or errors.
The normal mbx-enabled workspace Release check remains the final cache replay gate.

The iOS 17 correction now uses a shared `IOSHTTPByteIterator` in `HTTPClient.swift`,
binding `AsyncIteratorProtocol & SendableMetatype` directly and calling `next()`.
The exact contract passed a standalone iOS 17 warnings-as-errors probe. No minimum
OS change, unchecked conformance or availability fallback was added. The second
full build compiled product code, then failed on six actor diagnostics in two
hybrid capability persistence tests. Those two test methods now explicitly run
on MainActor, matching the existing persistence API; all assertions remain.
The third consolidated build is `/tmp/nola-warning-ios-debug-final3-20261009.log`.

That build passed without source warnings. Its regression run passed all 131
XCTest cases but failed one of 26 Swift Testing cases: URL history cold reopen
hit the single-writer guard. The test now bounds Foundation/assertion temporary
lifetimes with an autorelease pool and verifies the first runtime weak reference
is nil before reopening. No production lock or persistence behavior was changed.
The fourth Debug build passed; final test replay is recorded separately at
`/tmp/nola-warning-ios-regression-final2-20261009.log`.

The release assertion rejected the pool-only hypothesis: the runtime was still
retained at the assertion, although the later reopen succeeded. `URLHistoryStore`
is MainActor-owned under the app's default isolation, so off-actor test teardown
can enqueue its destruction. The test now runs on MainActor and retains the weak
release assertion, instead of adding a sleep or weakening the writer guard.
Full Debug recompile is `/tmp/nola-warning-ios-debug-final5-20261009.log`.

That build passed with no source warnings. The complete affected iOS regression
selection then passed: 131 XCTest cases plus 26 Swift Testing cases (157 total),
including the explicit cold-reopen release assertion. Evidence:
`/tmp/nola-warning-ios-regression-final3-20261009.log`. Release, Mac header rebuilds
and the normal shared-cache Rust check run sequentially after this successful gate.

### All-targets test warning cleanup

- Removed unused imports in committed-runtime, cancellation, atomic ownership,
  terminal receipt, and context usage read-model tests; removed an unused SQLite
  connection in late-cancellation coverage and unnecessary store mutability in
  tool receipts, turn transactions, and assistant-journal read tests.
- Split committed usage streaming fixture implementation into
  `support/committed_usage_stream.rs`; streaming WAL tests include it directly,
  while usage tests retain their small convenience wrappers. Removed the two
  pre-existing dead-code allows from this fixture during the split.
- Split receipt-only remote admission into `support/remote_submission_from.rs`.
  Existing source-producing fixtures delegate to that implementation, while
  canonical-stream and turn-transaction tests compile only the receipt helper
  they actually use. No fixture behavior or assertions were removed.
- No production Rust or Swift source changed in this second Rust fixture batch.
  Repeat native all-targets compilation passed, as did its 63 affected consumer
  regressions recorded above; `git diff --check` passes.

### Platform coverage boundaries

The 19-package matrix is native macOS coverage, not every target triple. Linux
`/proc` branches in CLI process identity/liveness and Linux-only tests, Windows
executor/supervision and windows-sys integration, and the non-Unix service-client
fallback remain outside native compilation. Mobile-bindings has no Android/iOS
source cfg split, but its cross-target dependencies and FFI linking still need
separate Apple/Android validation. No architecture-specific Rust source cfg was
found in the workspace inventory.

## Final Apple validation

- iOS Debug WMO build-for-testing: `nola-warning-ios-debug-final5-20261009.log`,
  exit 0, no source diagnostics. iOS affected regressions: 157 passed, as above.
- iOS Release: `/tmp/nola-warning-ios-release-final-20261009.log`, exit 0,
  no warnings/errors, including rebuilt FFI imports.
- Mac Debug WMO build-for-testing after header correction:
  `/tmp/nola-warning-mac-debug-header-final-20261009.log`, exit 0,
  two AppIntents metadata-skip tool notices, no source diagnostics.
- Mac Release after header correction:
  `/tmp/nola-warning-mac-release-final-20261009.log`, exit 0,
  one AppIntents notice, no source diagnostics.
- Mac UI/source contracts: 443 executed, 1 explicit opt-in system-picker skip,
  0 failures in `/tmp/nola-warning-mac-ui-regression-20261009.log`.
  The later header edit changes zero-argument prototypes/removes unimplemented
  declarations only; both Apple configurations rebuilt its actual imports.
- Both standalone MLX request/response script suites passed. Earlier full vendor
  compilation exposed four upstream Metal C++17-extension warnings; a later
  incremental log does not erase that dependency finding.

No global warning suppression, actor-unsafe escape hatch, deployment-target bump,
dummy AppIntents dependency or transient vendor checkout edit was used.

## Final native Rust validation and closure

`/tmp/nola-warning-rust-cache-final-20261009.log` exited 0 through the normal
repository mbx wrapper: Debug workspace all-targets finished in 0.63s and Release
workspace libraries/binaries in 48.71s. Neither reports Rust source warnings or
errors, and the old objcopy/dyld warning is no longer replayed. One mbx tool notice
states that a C compiler invocation without an output file bypassed caching; this
is not a source/compiler failure. Prior task/connection-budget tests passed 14,
and the affected application/store/service fixture suites passed 63.

The warning stage is complete for the enumerated native and Apple build matrix.
AppIntents metadata notices, the four dependency-owned Metal dialect warnings,
and the mbx compiler-probe cache notice remain classified external diagnostics,
not hidden or claimed fixed. Android/Linux/Windows cross-target builds, the empty
MessageFilter target, and real physical-device execution are not claimed covered.

## Follow-up: remaining build warnings

The previous AppIntents conclusion was too narrow: inspecting only the extraction
tool's `.xcspec` missed a task-producer guard. Swift Build's
[AppIntentsMetadataTaskProducer](https://github.com/swiftlang/swift-build/blob/main/Sources/SWBApplePlatform/AppIntentsMetadataTaskProducer.swift)
checks `LM_SKIP_METADATA_EXTRACTION` before constructing any tasks. The installed
Xcode's SWBCore binary contains that macro too. NolaMac and NolaMacTests declare
no App Intents, so their Debug/Release/Test configurations now set it to YES.
This removes an inapplicable build step, not compiler diagnostics; iOS and package
targets are unchanged. If App Intents are introduced in either target, remove
the override for that target so its metadata is generated normally.

`/tmp/nola-warning-mac-no-intents-20261009.log`: Debug build-for-testing exited 0,
no warnings/errors and no ExtractAppIntentsMetadata task for NolaMac or
NolaMacTests. Dependency targets retain their own extraction settings. Xcode removed its own
obsolete generated extraction lists. Release validation is recorded separately
in `/tmp/nola-warning-mac-no-intents-release-20261009.log`.

The four MLX Metal warnings remain real dependency diagnostics. Upstream's
[issue 342](https://github.com/ml-explore/mlx-swift/issues/342) is still open, and
the current upstream header still uses `if constexpr`. A local compiler probe
confirms Metal 4.0 selects C++17, while Metal 3.2 still selects C++14; raising the
shader language without considering supported OS versions is not a safe fix.
No dependency checkout, deployment floor or global warning flags were changed.
The user explicitly excluded MLX from this follow-up ("不用管mlx的"). Leave these
dependency warnings as an acknowledged exclusion; do not vendor/patch MLX or
describe its warnings as fixed. The mbx cache-probe notice is not a compiler
diagnostic and remains visible.

Follow-up validation complete: both Mac Debug build-for-testing and Release build
exited 0 with no warnings/errors and no metadata extraction task for the two
configured Mac targets. Project plist
validation and `git diff --check` passed. This follow-up changes only the six Mac
target configurations and this audit; the earlier native Rust/iOS validation
remains applicable because no Rust/iOS source or configuration changed. MLX is
explicitly outside the requested cleanup scope.
