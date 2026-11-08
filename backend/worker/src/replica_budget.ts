// SPDX-License-Identifier: Apache-2.0
// Bounded replica transport, independent of authentication and persistence.
export const REPLICA_BODY_BYTES = 4 * 1024 * 1024;
const RECORD_ENVELOPE_BYTES = 8192;
const PAGE_ENVELOPE_BYTES = 1024;

export class ReplicaTransportError extends Error {
    readonly status: number;
    constructor(status: number, message: string) {
        super(message);
        this.status = status;
    }
}

export async function readReplicaBody(request: Request): Promise<Record<string, unknown>> {
    const declared = request.headers.get("content-length");
    if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > REPLICA_BODY_BYTES)) {
        throw new ReplicaTransportError(413, "Replica request exceeds byte budget; chunked transfer required");
    }
    const reader = request.body?.getReader();
    if (!reader) throw new ReplicaTransportError(400, "JSON object required");
    const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });
    let used = 0;
    let text = "";
    try {
        while (true) {
            const result = await reader.read();
            if (result.done) break;
            used += result.value.byteLength;
            if (used > REPLICA_BODY_BYTES) {
                await reader.cancel();
                throw new ReplicaTransportError(413, "Replica request exceeds byte budget; chunked transfer required");
            }
            text += decoder.decode(result.value, { stream: true });
        }
        text += decoder.decode();
    } catch (error) {
        if (error instanceof ReplicaTransportError) throw error;
        throw new ReplicaTransportError(400, "Invalid replica request stream or UTF-8");
    } finally {
        reader.releaseLock();
    }
    let value: unknown;
    try { value = JSON.parse(text); }
    catch { throw new ReplicaTransportError(400, "Invalid replica JSON"); }
    if (!value || typeof value !== "object" || Array.isArray(value)) {
        throw new ReplicaTransportError(400, "JSON object required");
    }
    return value as Record<string, unknown>;
}

export function replicaPageEnd(
    rows: readonly { cursor: number; payloadBytes: number }[], after: number,
): number {
    let remaining = REPLICA_BODY_BYTES - PAGE_ENVELOPE_BYTES;
    let end = after;
    for (const row of rows) {
        if (!Number.isSafeInteger(row.cursor) || row.cursor <= end
            || !Number.isSafeInteger(row.payloadBytes) || row.payloadBytes < 0) {
            throw new ReplicaTransportError(500, "Invalid stored replica metadata");
        }
        const cost = row.payloadBytes + RECORD_ENVELOPE_BYTES;
        if (cost > remaining) {
            if (end === after) {
                throw new ReplicaTransportError(413, "Replica record exceeds byte budget; chunked transfer required; cursor unchanged");
            }
            break;
        }
        remaining -= cost;
        end = row.cursor;
    }
    return end;
}
