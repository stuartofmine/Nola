// SPDX-License-Identifier: Apache-2.0
// Independent JavaScript verification of the shared journal v1 golden vectors.
// This is a protocol check, not a Web journal consumer implementation.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const fixture = JSON.parse(readFileSync(new URL(
    '../core/nola-domain/tests/fixtures/assistant_journal_v1.json', import.meta.url), 'utf8'));
const maxU64 = (1n << 64n) - 1n;

function parseU64(value) {
    assert.equal(typeof value, 'string');
    assert.match(value, /^(0|[1-9][0-9]{0,19})$/);
    const integer = BigInt(value);
    assert.ok(integer <= maxU64);
    return integer;
}

function integer(value) {
    const bytes = Buffer.alloc(8);
    bytes.writeBigUInt64BE(parseU64(value));
    return bytes;
}

function string(value) {
    const bytes = Buffer.from(value, 'utf8');
    const length = Buffer.alloc(4);
    length.writeUInt32BE(bytes.length);
    return Buffer.concat([length, bytes]);
}

const prefix = kind => Buffer.concat([Buffer.from('NOLA-AJ'), Buffer.from([0, 1, kind])]);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const owner = ['source_store_epoch', 'conversation_id', 'run_id', 'assistant_message_id']
    .map(key => string(fixture.identity[key]));
const genesis = hash(Buffer.concat([prefix(0), ...owner]));
assert.equal(genesis, fixture.genesis_digest);

const chunk = Buffer.concat([prefix(1), string(fixture.chunk_text)]);
assert.equal(chunk.toString('hex'), fixture.chunk_canonical_hex);
assert.equal(hash(chunk), fixture.chunk_digest);

function record(sequence, revision) {
    return Buffer.concat([prefix(2), ...owner, integer(sequence), Buffer.from(genesis, 'hex'),
        string('operation-1'), integer(revision), Buffer.alloc(32, 0x11)]);
}
assert.equal(record('1', '7').toString('hex'), fixture.record_canonical_hex);
assert.equal(hash(record('1', '7')), fixture.record_digest);
assert.equal(parseU64(fixture.u64_max_wire), maxU64);
assert.equal(hash(record(fixture.u64_max_wire, fixture.u64_max_wire)), fixture.u64_max_record_digest);

for (const invalid of [1, null, true, '+1', '-1', '01', '1.0', '1e3', ' 1', '',
    '18446744073709551616']) {
    assert.throws(() => parseU64(invalid));
}
// Independently encode typed operation and manifest vectors, without invoking
// the Rust implementation or hashing JSON's platform-dependent formatting.
const operations = JSON.parse(readFileSync(new URL(
    '../core/nola-domain/tests/fixtures/assistant_operations_v1.json', import.meta.url), 'utf8'));
const operationOwner = ['epoch', 'conversation', 'run', 'assistant'].map(string);
const removeBatch = Buffer.concat([prefix(3), ...operationOwner,
    Buffer.from([0, 0, 0, 1]), // one operation, u32 BE
    Buffer.from([4]), // Remove
    string('text'), integer('1'), integer('2')]);
assert.equal(removeBatch.toString('hex'), operations.remove_batch_hex);
assert.equal(hash(removeBatch), operations.remove_batch_digest);
const emptyRoot = hash(Buffer.concat([prefix(4), ...operationOwner,
    string('text'), Buffer.from([0]), integer('1')])); // Markdown, generation 1
assert.equal(emptyRoot, operations.empty_content_root);
const xDigest = hash(Buffer.concat([prefix(1), string('x')]));
const appendedRoot = hash(Buffer.concat([prefix(5), Buffer.from(emptyRoot, 'hex'),
    Buffer.from(xDigest, 'hex'), integer('1')]));
assert.equal(appendedRoot, operations.appended_x_root);
// An identical chunk must not erase the manifest's ownership/generation binding.
for (const changedOwner of [['other-epoch', 'conversation', 'run', 'assistant'],
    ['epoch', 'conversation', 'other-run', 'assistant']]) {
    const changedRoot = hash(Buffer.concat([prefix(4), ...changedOwner.map(string),
        string('text'), Buffer.from([0]), integer('1')]));
    assert.notEqual(changedRoot, emptyRoot);
    assert.notEqual(hash(Buffer.concat([prefix(5), Buffer.from(changedRoot, 'hex'),
        Buffer.from(xDigest, 'hex'), integer('1')])), appendedRoot);
}
const fields = JSON.parse(readFileSync(new URL(
    '../core/nola-domain/tests/fixtures/assistant_field_operations_v1.json', import.meta.url), 'utf8'));
const fieldRoot = hash(Buffer.concat([prefix(4), ...operationOwner,
    string('tool'), Buffer.from([3]), integer('2')])); // ToolOutput, independent field generation
assert.equal(fieldRoot, fields.empty_tool_output_generation_2_root);
const fieldReference = Buffer.concat([Buffer.from([3]), integer('2'), integer('0'),
    integer('0'), Buffer.from(fieldRoot, 'hex')]);
const removeOutput = Buffer.concat([prefix(3), ...operationOwner,
    Buffer.from([0, 0, 0, 1, 5]), // one operation: ReplaceContent
    string('tool'), integer('1'), integer('3'), // Part generation 1, head revision 3
    Buffer.from([3, 1]), fieldReference, Buffer.from([0])]); // field, Some(expected), None(result)
assert.equal(removeOutput.toString('hex'), fields.remove_tool_output_batch_hex);
assert.equal(hash(removeOutput), fields.remove_tool_output_batch_digest);
process.stdout.write('Journal v1 JavaScript vectors verified: identity, UTF-8, header, u64 precision, typed operations, field generations and owner-bound manifest roots.\n');
