# Layering and deployment lifetimes

[简体中文](CORE_HOST_LAYERING.md) · English

The agent engine owns reducer state, recovery and trace cursors. Its inputs contain explicit time and source identity. The pure decision path cannot choose a filesystem, process or provider. Application uses the reducer port and adopts candidate state after the backing transaction succeeds. Bootstrap assembles the concrete implementation.

| Artifact or component | Lifetime and boundary |
| --- | --- |
| nola_core | Rust facade around engine types and helper functions. Built as an rlib. |
| nola_mobile_bindings | Packages the native C ABI and JNI access to embedded Application. Retains required helper symbols and reuses a runtime for the same canonical database identity. |
| nola_application / bootstrap / service | Use cases coordinate writes, bootstrap opens the concrete Store and the local Service accepts desktop process requests. Service lifetime follows the database writer contract. |
| nola_runtime_driver | Claims and delivers committed work through transport ports. Production dependencies include Domain and Host Protocol. Agent engine is used by driver test fixtures. |
| nola-agent-host | Performs admitted native Host work for desktop integrations. Process exit requires reconciliation with the run record. |
| Platform presentation | The view and its observation can close independently of a running task. A client draft can survive navigation without managing the canonical conversation. |

Goal coordination lives in Application. A run controller can observe and issue controls for one bound run. A worker claim confers bounded execution ownership. Restoring either object on another device requires its own authority checks.

To verify a boundary, inspect the actual declaration graph and follow a specific command, including its failure path. A Rust dependency test can detect a reversed edge. It cannot establish OS permission, upstream model behavior or remote execution. [Dependencies](DEPENDENCIES.en.md) · [Platform deployment](CURRENT_IMPLEMENTATION_MAP.en.md)
