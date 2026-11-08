> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Atomic archive replacement replica protocol

Archive replacement uses the existing authenticated replica log and one final
visibility transaction. Its scope is all canonical conversation history. It
preserves retained conversation, message and Part identities; retained conversation
revisions advance monotonically, while excluded conversations receive tombstones.

## Fact order

A completed local restore enqueues an immutable revision-1 `archive_restore_begin`,
then ordered `archive_restore_entry` facts. Each referenced body contributes bounded
`canonical_text_chunk` and `canonical_text_seal` dependencies before its entry.
Bounded `archive_restore_target` facts describe each affected conversation's
expected local record revision, expected content revision, resulting revision and
deletion state. One `archive_restore_commit` contains the exact staged manifest
count/digest and expected target count; it never embeds the entire archive.

Composite target identities encode the restore-ID length before the two IDs, so
IDs containing colons cannot collide. An altered replay with the same operation ID
is checked against the staged bytes rather than being blindly skipped.

The source pending-page reader withholds final commit until every begin, entry,
target and content-seal dependency has a durable acknowledgement. Settled staging
facts are retained across reopening the source database because they prove that
admission condition. Garbage collection requires separate lifecycle accounting.

## Receiver admission and visibility

Remote begin captures the receiver's current directory revision; the source's
numeric directory revision is not interpreted as a version of another database.
Remote entries, targets and body chunks only populate staging or immutable content
objects. They do not replace visible messages or publish incomplete references.

Final commit verifies the exact durable manifest, target count, complete body
objects, receiver directory stability, complete coverage of current live
conversations and staged profiles, all target base revisions and absence of active
Runs. An extra conversation created on the receiver, a concurrent rename, a
missing target, an altered replay or incomplete content rejects the transaction.
Old history and the last applied replica cursor remain intact.

A successful final transaction replaces history, applies retained revisions and
excluded-conversation tombstones, installs the original Part bindings and advances
the replica cursor. It emits no echo outbox records. Replaying the committed final
fact returns its saved restore result without advancing conversation revisions.

## Verified Store acceptance

```sh
cargo test -p nola_store archive_restore_sync --lib --quiet
```

Tests transfer a 10,000,000-byte logical Part through real pending/apply replica
pages into a separate database, reopening both databases after each 37-fact page.
Every pre-final read sees the old original. Only a separately acknowledged final
fact replaces it. The test compares all transferred original bytes, verifies
retained revision 2 → 3, the excluded conversation's revision-2 tombstone, no echo
and idempotent final replay. Additional cases exercise missing targets, receiver-only
conversations, altered same-operation staging replay and concurrent rename.

## Actual Worker HTTP acceptance

The 2026-09-08 run of `python3 scripts/test_worker_replica_e2e.py --with-cli`
also passed archive-wide replacement against an isolated local Worker and D1.
It transferred 507 archive facts for one 10,000,000-character / 15,000,000-byte
Part in nine loop iterations, reopening both canonical databases each round.
Every stage round retained the old content reference, title and branch. After
all stage, target and body acknowledgements, an independent one-fact final batch
atomically replaced the retained conversation's reference/title and removed the
excluded branch. Application range reads then compared all 15 MB of original
UTF-8 text, and another synchronization produced no echo.

This HTTP case prepares the local archive through Store repository APIs as an
explicit fixture. It exercises the actual CLI transport, Worker persistence,
receiver Application and canonical Store, including original reads. Native Mac
whole-backup restore and the new archive command's full Service IPC path still
require separate acceptance; the earlier production uploaded-Part publication
IPC test is not a substitute for those archive-wide client gates.
