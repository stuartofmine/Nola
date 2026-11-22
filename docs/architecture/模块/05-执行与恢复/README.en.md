# MOD-05 · Execution, cancellation and recovery

[简体中文](README.md) · English · [Architecture](../../README.en.md)

The driver delivers requests and returns observations through its port. The supervisor manages workers and process handles. The executing Host owns the physical operation. A lease fences competing deliveries and can expire while external work is still running.

## Runtime behavior

Application coordinates Goals. The driver claims committed work and the worker acts under a lease and generation. After a disconnect, reconcile the original effect with the executing Host. Confirm cancellation after receiving evidence that execution stopped.

## Failure and verification

Exercise restart after dispatch, cancellation racing a result and delivery by an expired worker. When an external effect cannot be inspected, keep its uncertain outcome visible. A new execution requires the appropriate recovery decision.

## Implementation

- [nola-runtime-driver/src/lib.rs](../../../../core/nola-runtime-driver/src/lib.rs)
- [nola-runtime-driver/src/task_execution_authority.rs](../../../../core/nola-runtime-driver/src/task_execution_authority.rs)
- [cli/src/native_host_entry.rs](../../../../apps/cli/src/native_host_entry.rs)

## Detailed references

[Durable Runtime Design](DURABLE_RUNTIME_DESIGN.md) · [Harness Control Scheduling](HARNESS_CONTROL_SCHEDULING.md) · [Host Composition Boundary](HOST_COMPOSITION_BOUNDARY.md) · [Recovery And Authority](RECOVERY_AND_AUTHORITY.md) · [Terminal Protocol](TERMINAL_PROTOCOL.md) · [Wait Protocol](WAIT_PROTOCOL.md)
