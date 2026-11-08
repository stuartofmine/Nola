> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Built-in Skill catalog ownership

Status: partially adopted; execution preparation and override editing remain open.

## Implemented boundaries

- Application synchronizes a bounded batch in one receipt transaction, preserving
  current activation inside that transaction. Invalid entries roll back the batch.
- Identical packages preserve editor revisions even across new startup requests.
- Mac prepares the full package manifest before submitting the batch, rather
  than committing one package at a time or inferring changes from text alone.
  Application owns metadata comparison and no-op decisions inside the transaction.
- The interactive application startup gate awaits catalog synchronization and
  routes failures through its existing retry UI.
- Missing bundles or individual instruction files reject synchronization before
  any write; an incomplete package cannot silently replace the installed catalog.
- Built-in gallery and active descriptor queries read canonical records and committed instruction URLs;
  they no longer install packages, inspect override files, or hash instructions.
- Built-in editor reads return committed instruction text and its canonical
  revision rather than reading a mutable file that may contain uncommitted edits.
- Chat freezes one Skill snapshot for its directory and capability arguments.
  Automation reads its own Application Port rather than the global store.

These changes do not yet meet all adoption gates below. Chat request preparation
still explicitly synchronizes the package on every request, although its queries
are read-only. Automation initialization also explicitly synchronizes its catalog.
Override save/reset submits synchronization explicitly but still writes files
before a checked canonical receipt. Immutable executable snapshots and checked
override edits remain required.

## Original production boundary violation

`NolaMac/Services/MacBuiltInSkillRegistry.swift` calls `canonicalRecords` from
`extensionItems`, `activeDescriptors`, and availability mutations. Despite its
query-like name, this helper reads every instruction file, hashes its contents,
and submits individual unchecked `upsertSkill` commands before querying again.

Consequences:

- Rendering or preparing a run can install or update canonical resources.
- A failure partway through the catalog leaves only some package versions updated.
- Availability is copied from an earlier query into later upserts; a concurrent
  user toggle can be overwritten by a package refresh.
- Repeated catalog reads incur file IO and two complete database list queries.
- Instruction overrides are changed on disk before any canonical edit receipt.

Existing Skill editor tests do not establish this catalog boundary. Passing them
must not be reported as completion of built-in package adoption.

## Required command and query separation

1. A platform package adapter reads bundled resources and submits an explicit
   `SynchronizeBuiltInCatalog` command. It does not write Store.
2. Application validates the entire bounded manifest before mutating any record:
   unique stable IDs, built-in source, valid immutable content versions, complete
   tool references, and explicit package identity.
3. Application installs the manifest in one durable command transaction. A failed
   entry rolls back all entries and their sync changes. Request replay returns the
   original receipt without reinstalling resources.
4. Package synchronization updates package-owned metadata and instructions only.
   Existing activation and hidden-state choices belong to the user and are read
   and preserved inside that transaction, never copied from a client query.
5. Absence from a manifest is not implicit deletion. Package removal must be an
   explicit, separately validated change.
6. Startup/package update awaits synchronization before publishing catalog-ready
   state. Failure is visible and retryable, not an apparently empty library.
7. `extensionItems` and `activeDescriptors` consume a revisioned catalog query.
   They perform no package installation, override writes, or per-file hashing.
8. Override editing requires its own checked command and immutable content
   reference. Writing a file and hoping a later list query discovers it is not a
   commit protocol. Reset follows the same rule.

The platform adapter may supply package files; Application is the sole authority
for installed versions and activation. Do not replace query-time installation
with a process-local boolean cache: that hides the race and fails on cold reopen.

## Adoption gates

### Instruction selection model required before editor adoption

The current single `version` pointer cannot represent both the latest installed
package and a user-selected override. A checked instruction edit alone is not
sufficient: a later package synchronization currently selects the package again.

The durable Skill aggregate must distinguish package version and optional user
override version, referencing immutable content versions. Package synchronization
may advance the package pointer and metadata but must preserve override selection.
A checked reset removes that selection and activates the latest installed package,
not the package present when editing began. Reset and save both check the editor
revision in the same transaction as pointer changes and receipt persistence.

Do not infer override ownership from filename prefixes, a process-local cache, or
the presence of a mutable override file. Runtime materialization follows committed
version references. Old active files must remain intact for in-flight runs; failed
edits may leave unreferenced staged content but cannot change active selection.

`builtin_package_refresh_must_not_replace_user_instruction_override` reproduces
this missing ownership boundary through Application commands and cold reopen.
Mac's save UI must not adopt the checked command until this gate is satisfied.

### Store implementation contract

The audited schema is version 11. `skill_definitions.current_version` is the only
selection pointer; `skill_versions` already stores immutable instruction content,
tool references, and execution mode under `(skill_id, version)`.

Add an explicit built-in selection relation keyed by Skill identity, with
`package_version` and nullable `override_version`, both referencing existing
immutable versions. The active projection resolves `override_version` when set,
otherwise `package_version`. Do not store another copy of instruction text in
settings or selection metadata.

The repository must support registering an immutable version without selecting
it. Package synchronization registers the package version, updates package-owned
metadata and its selection pointer, and preserves the override pointer. Saving
an override registers its version and changes only the override pointer. Reset
clears that pointer. Each operation is one admitted Application transaction with
one authoritative final editor projection, not two upserts that temporarily
select the package and then select the override again.

Changing either selection pointer advances the editor revision, including a
package update while an override remains active: a stale reset must not select a
new package the user has never observed. Identical package synchronization remains
a no-op. Activation stays independently user-owned.

Selection references must also be represented in Skill sync projections and
validated on import; otherwise another replica cannot reconstruct the same
active version. Reopen, sync round-trip, stale reset, invalid version rollback,
and package upgrade during an active run all need regression coverage before
the Mac editor is switched over.

This requires a canonical schema version change, not an implicit table creation
on read. Per repository policy, no legacy migration is planned. Validation must
use fresh isolated databases and must not delete or recreate a user's live store.
The schema change has not been applied yet.

### Remaining acceptance gates

- Querying an installed catalog leaves all revisions and sync records unchanged.
- Repeated reads do not reread or hash package instructions.
- An invalid final entry leaves earlier entries and availability unchanged.
- Toggle a Skill after manifest preparation but before synchronization: the
  committed catalog keeps the toggle.
- Replaying a committed synchronization after restart is a no-op.
- An override conflict preserves the draft, active version, and executable files.
- Gallery, automation, and chat startup use the same committed catalog revision.
- Exercise empty-install startup, package updates, missing/corrupt resources,
  cancellation, and restart through the real Application transport.

Completion requires changing all production entry points and their initialization
fixtures, not just renaming `canonicalRecords` or adding a cache around it.
