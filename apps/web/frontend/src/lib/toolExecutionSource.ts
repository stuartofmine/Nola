/** Canonical provenance only. Never use these IDs as an execution grant. */
export interface CodeCellIdentity {
    host_id: string; conversation_id: string; turn_id: string;
    parent_call_id: string; cell_id: string; generation: string;
}
export type ToolExecutionSource = { kind: 'model' } | { kind: 'code_cell'; cell: CodeCellIdentity };
const object = (value: unknown): value is Record<string, unknown> =>
    value !== null && typeof value === 'object' && !Array.isArray(value);
export function parseToolExecutionSource(value: unknown): ToolExecutionSource {
    if (!object(value)) throw new Error('Missing tool execution source');
    if (value.kind === 'model' && Object.keys(value).length === 1) return { kind: 'model' };
    if (value.kind !== 'code_cell' || Object.keys(value).length !== 2 || !object(value.cell)) {
        throw new Error('Invalid tool execution source');
    }
    const keys = ['host_id', 'conversation_id', 'turn_id', 'parent_call_id', 'cell_id', 'generation'] as const;
    if (Object.keys(value.cell).length !== keys.length) throw new Error('Invalid code cell identity fields');
    const cell = {} as CodeCellIdentity;
    for (const key of keys) {
        const id = value.cell[key];
        if (typeof id !== 'string' || !id.trim() || new TextEncoder().encode(id).length > 512
            || /[\u0000-\u001f\u007f-\u009f]/u.test(id)) throw new Error('Invalid code cell identity');
        cell[key] = id;
    }
    return { kind: 'code_cell', cell };
}
