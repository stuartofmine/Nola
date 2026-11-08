// SPDX-License-Identifier: Apache-2.0
import test from "node:test";
import assert from "node:assert/strict";
import { readReplicaBody, replicaPageEnd, REPLICA_BODY_BYTES } from "../src/replica_budget.ts";

function request(bytes, headers = {}) {
    return new Request("http://localhost/v1/sync/replicas", { method: "POST", body: bytes, headers });
}
test("replica request parses Unicode without relying on Content-Length", async () => {
    const data = new TextEncoder().encode(JSON.stringify({ value: "中文" }));
    const stream = new ReadableStream({ start(controller) {
        for (const byte of data) controller.enqueue(new Uint8Array([byte]));
        controller.close();
    }});
    const req = new Request("http://localhost", { method: "POST", body: stream, duplex: "half" });
    assert.deepEqual(await readReplicaBody(req), { value: "中文" });
});
test("declared and actual oversized request bodies are rejected", async () => {
    await assert.rejects(readReplicaBody(request("{}", { "content-length": String(REPLICA_BODY_BYTES + 1) })), { status: 413 });
    await assert.rejects(readReplicaBody(request("x".repeat(REPLICA_BODY_BYTES + 1))), { status: 413 });
});
test("malformed JSON, UTF-8 and non-object requests fail explicitly", async () => {
    for (const data of ["{", "null", "[]", new Uint8Array([255])]) {
        await assert.rejects(readReplicaBody(request(data)), { status: 400 });
    }
});
test("page reads only a bounded ordered prefix and can resume", () => {
    const rows = [1, 2, 3].map(cursor => ({ cursor, payloadBytes: 2 * 1024 * 1024 }));
    assert.equal(replicaPageEnd(rows, 0), 1);
    assert.equal(replicaPageEnd(rows.slice(1), 1), 2);
    assert.equal(replicaPageEnd([], 3), 3);
});
test("oversized head is not skipped or acknowledged", () => {
    assert.throws(() => replicaPageEnd([{ cursor: 5, payloadBytes: REPLICA_BODY_BYTES }], 4), { status: 413 });
});
test("corrupt lengths or ordering fail before fetching bodies", () => {
    for (const row of [{ cursor: 1, payloadBytes: -1 }, { cursor: 0, payloadBytes: 1 }, { cursor: 1, payloadBytes: NaN }]) {
        assert.throws(() => replicaPageEnd([row], 0), { status: 500 });
    }
});
