# MOD-03 · Application commands and transactions

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Application coordinates business writes through Store Port. Service and embedded clients reach the same use cases. A read can return a snapshot and cursor. A mutation validates its source, applies the transition and records the relevant state and follow-up work in a unit of work.

## Runtime behavior

If a connection drops after submission, query the original request_id. On a revision conflict, reload the affected object. A model catalog illustrates a cross-object command. Each connection revision and the default selection are checked in one transaction.

## Failure and verification

Exercise duplicate submissions, conflicting payloads under one request ID, a late failure and a lost acknowledgement. Check the stored objects as well as the receipt. An accepted command can still have execution ahead of it.

## Implementation

- [nola-application/src/application_command.rs](../../../../core/nola-application/src/application_command.rs)
- [nola-application/src/lib.rs](../../../../core/nola-application/src/lib.rs)
- [nola-application-bootstrap/src/runtime_owner.rs](../../../../core/nola-application-bootstrap/src/runtime_owner.rs)

## Detailed references

[Application Port Contract](APPLICATION_PORT_CONTRACT.md) · [Canonical Conversation Transactions](CANONICAL_CONVERSATION_TRANSACTIONS.md) · [Conversation Mutation Contract](CONVERSATION_MUTATION_CONTRACT.md)
