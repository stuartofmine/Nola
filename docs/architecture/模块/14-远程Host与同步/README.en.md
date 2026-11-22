# MOD-14 · Remote Hosts and synchronization

[简体中文](README.md) · English · [Architecture](../../README.en.md)

The source Host owns the admitted run and committed journal. The receiving device holds a verified projection. Tailcat is an optional transport to the same Host. Web sessions and paired mobile requests retain their own authentication on that path.

## Runtime behavior

Host connectivity, user authentication and authorization for an operation have separate state. Remote work binds the source Host, Store and run. A client catches up from its cursor. Leaving the screen ends observation. Stopping work requires a cancellation request against the original task.

## Failure and verification

Disconnect after sending and recover with the original request identity. Test revoked pairing, a stale cursor and a receipt from a different Host or epoch. An observed remote task must never launch a local copy.

## Implementation

- [src/web_parts/routes.rs](../../../../apps/cli/src/web_parts/routes.rs)
- [transport/tailcat/README.md](../../../../transport/tailcat/README.md)

## Detailed references

[Remote Host Projection Contract](REMOTE_HOST_PROJECTION_CONTRACT.md) · [Web Chat Transport Contract](WEB_CHAT_TRANSPORT_CONTRACT.md)
