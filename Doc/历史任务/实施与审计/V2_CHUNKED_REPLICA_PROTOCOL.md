> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# V2 canonical text replica facts

The existing authenticated Worker replica log transports bounded content facts using
its normal operation identity, revision and cursor contract. It does not receive
an inline body hidden inside conversation metadata.

## Ordered facts and acknowledgement

For every canonical text object, the source commits its original stored chunks as
`canonical_text_chunk` facts, followed by one `canonical_text_seal` fact, before the
`conversation_append` fact containing Part references. Chunk entity identity is
`<content_id>:<offset_bytes>`; seal identity is `content_id`. Both are immutable
revision 1 facts. The chunk payload contains the validated canonical reference,
byte offset and at most 32 KiB of UTF-8 text. The seal contains only the reference.
Preserving stored chunk boundaries preserves the reference's chunk count.

The Store's pending-page reader withholds a referenced append until its exact
seal has a durable acknowledged/unchanged outbox record. The CLI additionally
ends an HTTP push batch at each seal. Conflict responses leave the requested
prefix pending. Reopening the Store retains settled seal facts because their
acknowledgements are dependencies of later message publication.

## Receiving and visibility

Incoming chunks enter `canonical_text_replica_chunks` in the same transaction as
the replica page cursor. Each new chunk must preserve contiguous offsets, owner,
length and declared chunk count. Replays compare the actual stored bytes.
Admission reserves the declared object size, counting canonical objects and
incomplete staged objects only once, with a 256 MiB combined capacity. Chunk and
reference reads limit selected SQL text before allocating Rust strings.

A seal verifies all offsets, complete byte length and original chunk count, then
atomically installs the immutable canonical object and chunks. Existing objects
require equal metadata and equal bytes. Staging is retained; successful sealing
is not lifecycle authorization to erase transfer state. A referenced append
requires the complete canonical object, exact conversation/message/Part ownership,
a declared text Part, and no inline text or JSON second source. Messages, Part
bindings, content revision and remote cursor share the canonical transaction.

No visible message points into incomplete staging. An interrupted page rolls
back its chunks and cursor together; previously committed pages resume after both
endpoints restart. Receiving content creates no echo outbox records.

## Evidence and remaining acceptance

`canonical_text_replica` Store tests transfer one 10.2 MB Part through production
pending-page and apply-page paths into a distinct canonical database, restarting
both endpoints after each 37-fact page, and compare every original chunk. They
also exercise missing-tail rollback, altered replay, foreign ownership and shared
staging quota. The CLI test verifies that a seal ends the HTTP batch before a
reference append.

The actual isolated Worker + local D1 acceptance command is:

```sh
python3 scripts/test_worker_replica_e2e.py --with-cli
```

The 2026-09-08 run passed. The additional CLI case synchronized one logical Part
containing exactly 10,000,000 characters (15,000,000 UTF-8 bytes, including CJK,
backslashes, quotes and newlines) in 500 original chunks. It required eight loop
iterations, reopening both canonical databases between push/pull rounds. Each
round checked that a referenced Part was absent until content acknowledgement;
all original bytes were then compared through the canonical Application range
reader. The projection remained bounded and another pull created no echo.

The same HTTP run invoked the existing Application branch command, transferred
504 branch facts, restarted both endpoints between rounds, and verified the
received branch's exact lineage intent, original length and final 6,000 bytes.
The original Worker tests also passed pagination, immutable operation replay,
account isolation, oversized request rejection and corrupt-payload rejection.

The source large Part is an explicitly prepared and published Store fixture.
This Worker fixture establishes the actual CLI transport, Worker persistence,
receiver Store and Application readers. The separate production Service upload
and restart acceptance is recorded below; the Mac restore entry point remains
a separate gate.
Worker log validation is generic; visibility dependency validation belongs to
the receiving canonical Store.

## Production publication IPC acceptance

The separate 2026-09-08 production publication acceptance also passed:

```sh
cargo test -p nola_service --test uploaded_transcript_publication_process_e2e -- --nocapture
```

This test creates no canonical body fixture. The real SDK creates the conversation,
begins an owner-bound upload, appends 500 chunks, seals it and invokes
`PublishUploadedTranscript` against the actual Service subprocess. It restarts the
Service halfway through upload, after sealing and after publication. Same-request
replay returns the identical durable receipt; a different request for the same
import returns the same result; both forms of changed replay are rejected. The
database contains exactly one message, one Part, revision 2, one import publication
and 503 sync facts. Search locates the tail, the viewport stays bounded, and 458
original range reads plus 458 export reads compare every original UTF-8 byte.

The measured input was exactly 10,000,000 characters / 14,999,994 UTF-8 bytes.
A transparent local socket observer measured 2,862 actual wire exchanges, a
45,304-byte maximum request frame and 99,317-byte maximum response frame. Across
84 RSS samples, the Service peaked at 22,970,368 bytes and the test client at
25,985,024 bytes. The complete run took 6.217 seconds. These are sampled RSS values,
not allocator-instrumented absolute peaks. Mac restore remains a separate gate.

The same Worker acceptance now also includes atomic archive replacement:
507 facts / nine loop iterations retain the old visible reference and branch
until one separately acknowledged final commit. It then verifies the replacement
original through bounded Application reads. Details are recorded in
[V2 atomic archive replacement replica protocol](V2_ATOMIC_ARCHIVE_RESTORE_SYNC.md).
