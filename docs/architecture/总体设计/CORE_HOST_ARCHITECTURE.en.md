# Core, Application and Host boundaries

[简体中文](CORE_HOST_ARCHITECTURE.md) · English

Consider a model asking to read a file. Core consumes typed input, frozen configuration and explicit facts. It can propose an effect or a permission wait. Application verifies the source and commits that transition. The file becomes readable to the model only after the executing Host has read the admitted resource and Application has accepted the matching observation.

| Boundary | Owns | Input and output |
| --- | --- | --- |
| Domain / Kernel / agent engine | Invariants, deterministic decisions and reducer state | Committed state plus typed input produces candidate state, events and effect intents. |
| Application / Store Port | Admission, idempotency, transaction scope and public projections | A command yields a receipt. A read yields a snapshot or cursor. |
| SQLite Store | Durable messages, effects, credential references, receipts and checkpoints | A unit of work commits the relevant objects and successor work together. The Host-private vault holds the secrets. |
| Driver / supervisor | Claims, leases, delivery, process handles and stop observations | Dispatch committed work and return observations bound to the original execution. |
| Provider / tool / platform Host | Physical provider I/O, tool operation and OS interaction | Execute within current permissions and report the outcome or remaining uncertainty. |
| Client | Drafts, focus, disclosure and painting | Submit explicit actions and project committed parts. |

### Events and restart

A committed part or journal cursor advances product history. A Host observation reports what happened during a particular attempt. A diagnostic trace can be sampled or discarded. Persisting a trace does not grant it ownership of a message.

Restore execution from its checkpoint and original configuration. Use the ledger for visible history. Revalidate the owner and generation before accepting late work. When the observer is on another device, import the source Host’s committed projection under its original identity. Credentials and leases stay on the appropriate Host.

[Artifact and deployment map](CURRENT_IMPLEMENTATION_MAP.en.md) · [Command outcomes](../MODULE_CONTRACTS.en.md) · [Recovery paths](WORKFLOWS_AND_RECOVERY.en.md)
