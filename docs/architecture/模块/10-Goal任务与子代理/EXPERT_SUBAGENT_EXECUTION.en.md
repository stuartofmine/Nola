# Main-agent dispatch and expert child conversations

[简体中文](EXPERT_SUBAGENT_EXECUTION.md) · English

The main agent owns user interaction and clarification. An expert is a reusable role with a prompt, model configuration, reasoning selection, focused tools and Skills. A temporary subagent and an expert share Task admission, worker claim, cancellation, recovery and result delivery. The expert uses the frozen member configuration and a persistent child conversation under its parent.

**Mention and dispatch.** An inline mention identifies the selected member in the original message. The main agent receives the message and can delegate through delegate_expert. The Host supplies trusted source conversation, run and effect identities. After validating the room and member binding, Core prepares a Task. A worker is woken after durable promotion acknowledgement. An uncertain acknowledgement is reconciled under its original identity.

**While executing.** The expert can use its admitted tools and return a result or a request for clarification to the main agent. Core rejects new expert delegation and ordinary child spawning from the expert context. The main agent can ask the user. A timeout while waiting does not stop the worker. A cancellation needs evidence from the actual execution.

**History and editing.** Opening an expert follows its parent/member relationship and scoped child history. Template edits affect subsequent admissions. Resending edited content uses the same source conversation and checks affected child histories. The supported rewind validates the full set before committing. Old messages lacking inline range data retain their original text. A client facing a Host without the optional mention capability must retain the draft and expose its available choices.

Verify a second turn reuses the child history, two parent conversations stay isolated, expert-to-expert dispatch is denied, and cancellation survives reconnection. Verify the actual main-agent result and conversation ledger, in addition to the visible mention. [Main selection](../../../../crates/nola-modules/src/rooms_peers.rs) · [Expert tasks](../../../../crates/nola-modules/src/rooms_peers.rs) · [Real-model fixture](../../../../crates/nola-modules/src/rooms_peers.rs)
