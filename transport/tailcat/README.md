# Optional Tailcat transport

This component carries Nola's existing HTTP/SSE and encrypted `/api/mobile`
protocol. Rust Core remains the authority for authentication, permissions, Runs,
results and recovery. Closing a transport is never a Task cancellation ACK.

It pins official `tailscale/tailcat` commit
`e00cdde6ff787f801361ff85ed085775c775d4c5` through Go's checksum-verified module
version `v0.7.1-0.20261105151022-e00cdde6ff78` (BSD-3-Clause). It requires Go
>=1.27.2; development verification used official Go1.27.3. Upstream is experimental
and does not promise stable APIs/wire format. This is optional, not an automatic
installer, login service or system VPN.

## Implemented boundary

* Host forwards one virtual TCP service5001 to one exact `http://127.0.0.1:port`.
  Before starting it verifies the expected Nola Host ID and explicit
  `transportIngress:tailcat`, `authenticated:false`, `localTrust:false`.
* Start Nola's existing Web/Server listener with `--tailcat-ingress`; this disables
  loopback local trust, keeps Web login/CSRF and mobile authentication, and refuses
  local-only account authorization over the tunnel. The flag itself starts no
  tunnel and creates no device keys.
* An explicit client public-key allowlist is mandatory. Upstream's nil
  `AllowClient` allows everyone; this adapter never uses that default.
* Keys are ephemeral and memory-only. No saved Tailcat key, account environment,
  filesystem/SSH/exec/exit-node service, OS route or DNS configuration is used.
* Client completes the encrypted path and verifies the expected Nola Host before
  returning a local origin. A listening port alone is not “connected”. HTTP/SSE
  bytes are proxied without render buffering or application-state interpretation.
* Client's endpoint is loopback-only. Owned pipes/connections close on parent EOF,
  stop control, interrupt or a listener failure. Errors are stage-only; no raw
  credential/endpoint dump. A failed close returns failure, not a Run stop fact.

## Private operator IPC

Build the standalone adapter with approved Go, using the checked-in go.mod/sum:

```
GOTOOLCHAIN=local go build -mod=readonly -trimpath -o <private-output>/nola-tailcat ./cmd/nola-tailcat
```

On macOS, give the compiler a private writable TMPDIR if it cannot obtain its
normal Darwin temporary directory. This doesn't change runtime security policy.
The repository's existing `nola` CLI can launch the selected adapter:

```
nola tailcat-bridge /absolute/private-output/nola-tailcat <SHA256-of-that-binary>
```

The Rust entry rechecks the executable hash/path, clears ambient environment and
replaces itself with that exact program on Unix (no detached child/daemon). The
Go adapter **requires parent-owned stdin/stdout pipes**. It will not print a
secret address to a terminal or redirected ordinary log. Do not use the model's
shell, argv, URLs, DNS, environment or diagnostic logs to carry control/addresses.

A private caller coordinates protocolVersion1 frames:

1. Client input `{"version":1,"kind":"client"}`; response is an `identity` event
   containing `clientPublicKey`. That key is public, never the private identity.
2. Host input `{"version":1,"kind":"host","host":{"origin":"http://127.0.0.1:5001","expectedHostId":"<explicit-Host-ID>","allowedClientKeys":["<client-public-key>"]}}`.
   Host `ready` contains the secret `address` (may contain PSK); use private IPC.
3. Client input `{"version":1,"kind":"connect","address":"<secret-from-private-IPC>","expectedHostId":"<explicit-Host-ID>"}`.
   Only after encrypted Host verification succeeds does `ready` contain the local
   HTTP `origin`, usable by existing API clients with their normal Nola login.
4. `{"version":1,"kind":"stop"}` or owner EOF closes transport only.

A trusted caller may specify `derpMapUrl` in Host config or `relay` in the connect
frame, restricted to a credential-free HTTPS URL. Otherwise upstream's public
DERP map/relays are used. Relays may rate-limit; direct connectivity isn't promised.
No persistent-device onboarding or automatic trust-on-first-use is implemented.

## Actual checks and remaining platform work

Offline `go test -mod=readonly ./...` uses metadata/in-memory pipes only. One
explicit opt-in test additionally uses real WireGuard/DERP and a synthetic HTTP
protocol origin (`NOLA_TAILCAT_NETWORK_TEST=1`); it is **not** a real Nola execution
Host or UI test. Do not enable it automatically on every CI build.

Mac development process testing has exercised Rust entry → Go adapter → encrypted
stream with auth401, incremental SSE and normal teardown against a protocol fixture.
A separate normal isolated Nola Server/Application Service test has also passed:
encrypted Host identity inspection, unauthenticated conversation query401, existing
Web login200, authenticated Core-backed conversation query200 and missing-CSRF403.
It used a temporary private application home and temporary test credentials, no
model request or user account. The Host and both transport processes exited; this
does not constitute an app UI or cross-machine connectivity test.
App-side automatic upgrade is implemented for the optional linked native SDK.
Mac/iOS share `Shared/Protocol/NolaTailcatTransport.swift`; Android uses the same
Go Session through its optional AAR and Ktor adapter. It applies only to an
already-paired, root-mounted private HTTP Host with an expected Host identity and
no HTTPS certificate pin. Existing HTTPS/pinned/custom-prefix connections keep
their existing transport; no TLS trust rule or stored Host URL is rewritten.
Missing SDK means no Tailcat upgrade, not a claimed tunnel.

### Authenticated temporary grants

A trusted Host operator explicitly selects the helper:

```
nola server --host 127.0.0.1 --port 5001 --tailcat-ingress --tailcat-helper /absolute/nola-tailcat --tailcat-helper-sha256 <SHA256>
```

The selected program is verified before startup and again before each launch.
`POST /api/mobile/transport/tailcat` uses existing mobile authentication and an
encrypted mobile envelope. `connect` requires the expected Host ID and one
client's ephemeral public key. Host returns an encrypted version1 receipt with
leaseId, address, hostId and expiresIn; it never prints the address. The bridge is
owned by private pipes, closes on owner exit and has a20-minute in-memory lease.
A bounded32 transient bridges protects Host resources; it is not a model budget.
`close` requires the same credential owner and exact leaseId. The response says
closed only after process exit; unknown stop evidence remains stopUnconfirmed.
No lease or private key is stored in Core tables or client preferences.

Native requests first inspect the existing Host, automatically exchange a grant,
verify the encrypted path and route subsequent API bytes through the local SDK
endpoint. Logical Host/model identities, credentials, HMAC paths, Core ownership
and event cursors are unchanged. There is no application request replay or
fallback after an advertised Tailcat connection fails. Disabling Tailcat on the
Host is explicit; an unconfigured Host continues its normal existing transport.

Mac/iOS route HTTP/SSE through one shared URLProtocol that forwards received
chunks without render buffering; redirects never forward signed credentials to
another origin. Android routes authenticated requests at HttpSend after verifying
that their proof belongs to the selected Host configuration; both normal and
independent control clients use it. HTTPS pins are excluded from rewriting.
Credential deletion/change closes obsolete connections. Foreground return can
reuse a still-valid memory-only connection; OS background suspension does not
promise continued execution. In-flight startup is cancelled on background;
established ephemeral identity/grant is retained in memory, not persisted. Full
close releases the local engine and best-effort Host lease. Lease expiry or app
termination requires a fresh authenticated bootstrap, so cold cross-network
reconnect is not promised without separately approved durable device identity.

### Actual platform checks

* Real production Rust Host + shared Apple native SDK adapter: automatic grant,
  authenticated requests, logical URL retained, explicit park/resume, full close
  and reconnect passed with temporary test identities. This is a native network
  adapter test, not an actual Mac/iOS window or OS suspension test.
* Final Mac Test app, iOS arm64 simulator Test app and Android arm64 debug APK
  builds passed. The APK contains the optional JNI SDK. No actual native window,
  Android device, iOS OS-suspension, Intel/Windows/Linux runtime claim.
* Android actual device testing stopped when normal adb service startup returned
  smartsocket5037 EPERM. No alternate port, permission change or retry.
* Mac's existing authenticated Web-management handoff now opens the App-owned
  loopback origin. Real Web login and a Core-backed conversation query passed
  through that origin. Mobile external browsers are not mapped to an App-owned
  loopback listener because their owner may be OS-suspended.
* Browser settings validate the authenticated ingress state and explain that an
  ingress marker does not establish a browser tunnel. Existing same-origin Web
  login/CSRF works through an owner-controlled local bridge. Browser-native WASM is intentionally outside this product path: Web is
  launched by CLI, which owns transport; no fetch/token shim is needed.


## Reproducing mobile bindings

The official build tool is pinned by the module's `tool` directive:
`golang.org/x/mobile/cmd/gobind@v0.0.0-20261011034528-197770444867`.
Use that matching gomobile/gobind version with the approved Go toolchain, existing
Xcode/Android SDK and NDK, and an output directory outside the source tree. This
does not install tools, apps, devices or keys automatically.

```
gomobile bind -target=ios/arm64,iossimulator/arm64 -tags=ts_omit_ssh -prefix=NolaTailcat -o <private-output>/NolaTailcat.xcframework ./mobile
gomobile bind -target=android/arm64 -androidapi=26 -tags=ts_omit_ssh -o <private-output>/nola-tailcat.aar ./mobile
```

For the checked-in App consumers, run `build-sdk.sh /absolute/go
/absolute/pinned/gomobile /absolute/pinned/gobind` with the approved pinned tools
and existing SDK/NDK. It generates ignored `build/apple` and `build/android`
outputs. Xcode optionally includes `Tailcat.xcconfig`; Gradle optionally consumes
the local AAR. Neither App build installs software or downloads a transport binary.
Currently generated slices are arm64 macOS/iOS/simulator and arm64-v8a Android;
use an arm64 simulator destination rather than a universal Intel simulator build.

The mobile tooling participates in Go's module graph; its selected `golang.org/x`
versions are fixed in go.mod/sum. `ts_omit_ssh` excludes the optional upstream SSH
feature; this component never enables SSH/files/exec services. No binary is checked
in, and the generated bindings alone are not a distributable app integration.

No credentials or binary artifacts belong in the repository. Redistributed
binaries must include the upstream/transitive license notices; this change keeps
source/module pins and primary dependency notice, not a full distribution bundle.

## CLI-launched remote Web client

The browser remains an ordinary same-origin HTTP/SSE client. A local CLI owns
the temporary Tailcat connection, rather than embedding WASM into the page:

```sh
nola web --connect-host https://trusted-host.example \
  --expected-host-id <paired-host-id> \
  --tailcat-helper /absolute/nola-tailcat \
  --tailcat-helper-sha256 <SHA256> --token-stdin
```

Supply the **existing Nola Host access token** through private piped stdin,
never argv, a URL or an environment variable. This is not a provider API key.
The remote Host must explicitly enable the verified helper and authenticated
Tailcat ingress. The CLI verifies Host identity, obtains an encrypted temporary
grant and prints only the loopback browser origin. Normal Web login, session
cookies and CSRF still apply; the transport grant does not log the browser in.
No local business Core/database starts in this mode. Plain `nola web` retains
its existing local Host behavior.

Ctrl-C closes the CLI-owned transport; it does not cancel Host tasks. Automated
owners may add `--control-stdin`, where stop/EOF ends the connection. Without
that flag, credential-pipe EOF does not close the foreground CLI. Connections
currently expire after at most 20 minutes; reconnect through the authenticated
bootstrap. There is no automatic permanent key, durable access or hidden renewal.

Validation boundary: CLI build/check and isolated origin-policy checks passed.
The original confined attempt and its explicitly authorized one-time retry
failed at loopback bind with EPERM; those failures remain recorded. A subsequent
normal per-command execution permission request was approved, without changing
system security settings. Real CLI → Tailcat → Rust Host/Application Service
checks then passed in 12.682s: exact identity/no local trust, unauthenticated
conversation401, Web login200, Core conversation query200, missing-CSRF write403,
CLI exit0 and isolated Host exit after intentional SIGINT. The client created
no local Core database. Root before login returned303; no actual browser-page
render or full SSE UI interaction was exercised by this HTTP test. Zero model
requests and no durable access credentials. Earlier failures do not count as
passes and this does not validate cross-machine networking or native UI.

Follow-up: the actual in-app browser loaded the real CLI-proxied login page,
validated short input and displayed Host rejection of a synthetic wrong token;
its original screenshot was inspected. Successful browser credential entry was
not exercised (successful HTTP login remains a separate result). Real production
read-only task-event SSE also traversed the CLI bridge: decrypted projection
at0.022798s, checkpoint at1.581031s, then EOF. This proves incremental task-event
transport, not model-generated chat/animation. A separate real WireGuard/DERP
protocol-fixture SSE test passed12.15s. Cross-machine validation requires a second
authorized device/runner, not available in the current local execution interface.

Latest browser follow-up: normal form login with a public synthetic token on an
isolated temporary Host passed. A real production model form saved a loopback
replay profile, the session selected it, and one synthetic Chat Completions SSE
request rendered partial text/running state, then complete Python/table/end
marker; browser reload retained committed content/model. This is actual Nola
browser/CLI/Host plumbing with **local replay output**, not GPT inference. Small
Web fixes refresh the read-only catalog when opening the menu, preserve drafts
when no model is selected, and accept Core's resolved default-model receipt.
Final Svelte check/build,23 focused tests and localization gates passed. Both
CLI/Host test lifetimes and replay service exited normally under test control.
Cross-machine networking, real GPT over Tailcat and continuous animation timing
remain separate, unverified scopes. No persistent remote device access created.

Actual iOS simulator follow-up passed on the preserved QA device: current native
Test app completed a local SSE replay, exited, cold launched into the same
conversation and completed a second reply in60.619s. Bootstrap-only logical
origin refused direct business endpoints, so these replies used the shared
Swift/Go tunnel. Normal Core catalog configuration was required; a CLI endpoint
alone is not a default model profile. Existing ephemeral test setup supplied
the synthetic Host credential after each launch; secure-store restoration is
not implied. Two local replay requests, zero GPT calls. Initial stale-locator
and missing-catalog failures retained. Both final screenshots inspected. The
controller summary had a timer-name collision after Host cleanup; exact final
Host exit code/direct-request counter were not serialized. QA simulator shut
down normally without erase. Android's adb approval request was rejected and
not executed. No cross-machine, real network handover or in-flight recovery claim.

### Android platform interfaces

The Android app enumerates actual interfaces through Java `NetworkInterface`,
including index, MTU, flags and assigned IPv4/IPv6 prefixes. Before native
connection admission it supplies that process-local snapshot through the mobile
binding. The Android-only Go initializer registers Tailscale's supported
`netmon.RegisterInterfaceGetter`; it does not call Android-blocked Go netlink
interface enumeration or register a synthetic fallback interface.

Foreground `ConnectivityManager` callbacks refresh snapshots. Changed snapshots
invalidate existing transport entries; subsequent authorized requests obtain a
fresh connection. Background removes callbacks and retains no background service;
foreground and every request refresh again. Enumeration failure clears the native
snapshot, closes affected entries and fails explicitly. No socket probe, route,
VPN, network permission, identity or Host authentication policy is added.

Offline tests verify address preservation and rejection of malformed, empty or
loopback-only snapshots. The prior Android connection failed during Host
inspection with an EPERM/EACCES classification. The exact rejected syscall was
not retained, so interface enumeration is a source-backed diagnosis, not a
runtime-proven syscall trace. After explicit approval, an isolated Android API35
QA emulator using the rebuilt JNI SDK passed native Host identity inspection,
Core conversation projection/checkpoint SSE, an independent control request while
the stream remained open, and actual Activity/Process foreground suspension and
return. A second instrumentation process reconnected to the same Host and Core
conversation after a normal QA-app force-stop; the public test credential was
resupplied in its own namespace, not restored from a user credential store.
The logical-origin gateway accepted only bootstrap/grant requests, so
business requests could not silently fall back to the original HTTP origin.
This is a native transport-component test against a real Rust Host on the same
Mac, with no model calls; it is not production chat UI, a physical cross-machine
test, or an in-flight network-loss test.

### Additional production iOS UI replay check

The actual iOS Test app passed one long-stream UI flow: foreground/background
while receiving, cold launch after a newly admitted second reply had begun, and
Stop on a third reply with a `cancelled` journal projection. Each complete replay
contained 40 paragraphs plus code/table content over about 31–33 seconds of Host
execution. The cancelled provider connection closed after 11 of 44 chunks, with
partial content preserved and “Response stopped” visible. Screenshots were
examined locally. This proves the exercised UI/transport lifecycle, not animation
frame quality, a physical network outage, cross-machine operation or GPT output.
An earlier temporary test falsely reused the preceding reply's start marker and
interrupted a prepared, unadmitted submission; that failed trace is retained and
is not counted as a completed recovery/cancellation pass.

### Sealed remote conversation ownership

A completed remote Run remains an admitted source owner for the next message.
Permission-control projections correctly become inactive, and the pending-intent
query correctly excludes sealed Runs; neither is a historical source identity.
`read_admitted_remote_run_identity` is a read-only Application command over the
existing Core/store admitted-identity reader. It checks source Host, client,
conversation, Run and assistant against the stored receipt-derived identity.
Android locates the displayed root in its bounded canonical journal window, reads
that identity, then uses the existing authenticated Host submission frontier.
The Host still checks the admitted owner and atomic configuration/parent snapshot;
missing, foreign and stale ownership never becomes permission to submit.
