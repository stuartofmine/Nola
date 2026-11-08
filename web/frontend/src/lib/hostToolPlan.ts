// SPDX-License-Identifier: Apache-2.0
// A tool catalog is issued by the execution Host, never inferred from client settings.
// The issuing Host checks freshness at admission; client clocks are not its clock.
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const identity = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0
    && new TextEncoder().encode(value).length <= 512 && !/[\u0000-\u001f\u007f-\u009f]/.test(value);
const fields = (value: Record<string, unknown>, names: string[]) => Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name));
export function hostToolPlanReference(response: unknown, hostId: string, clientId: string, sessionId: string): Record<string, unknown> {
    if (![hostId, clientId, sessionId].every(identity) || !object(response) || response.source_host_id !== hostId || !object(response.result)) throw new Error('Invalid execution Host tool plan');
    const plan = response.result;
    const ref = plan.reference;
    if (!fields(plan, ['schema_version', 'reference', 'issued_at_ms', 'expires_at_ms', 'tools']) || plan.schema_version !== 1
        || !object(ref) || !fields(ref, ['manifest_id', 'host_id', 'client_id', 'session_id', 'catalog_revision'])
        || ref.host_id !== hostId || ref.client_id !== clientId || ref.session_id !== sessionId
        || !identity(ref.manifest_id) || typeof ref.catalog_revision !== 'string' || !/^[0-9a-f]{64}$/.test(ref.catalog_revision)
        || !Number.isSafeInteger(plan.issued_at_ms) || !Number.isSafeInteger(plan.expires_at_ms)
        || (plan.issued_at_ms as number) <= 0 || (plan.expires_at_ms as number) <= (plan.issued_at_ms as number)
        || (plan.expires_at_ms as number) - (plan.issued_at_ms as number) > 300_000
        || !Array.isArray(plan.tools)) throw new Error('Invalid execution Host catalog identity');
    const ids = new Set<string>();
    for (const tool of plan.tools) {
        if (!object(tool) || !fields(tool, ['tool_id', 'route', 'schema']) || !identity(tool.tool_id) || ids.has(tool.tool_id) || tool.route !== 'agent_host'
            || !object(tool.schema) || tool.schema.type !== 'function' || !object(tool.schema.function)
            || tool.schema.function.name !== tool.tool_id || !object(tool.schema.function.parameters)) throw new Error('Invalid execution Host tool schema');
        ids.add(tool.tool_id);
    }
    return { ...ref };
}

export async function freezeHostToolPlan(request: Record<string, unknown>,
    api: (path: string, options: RequestInit) => Promise<unknown>): Promise<Record<string, unknown>> {
    // Capture the complete submission before yielding to transport. Caller-owned
    // drafts (including nested messages/configuration) must not change the body
    // associated with the Host reference while preparation is in flight.
    const snapshot = structuredClone(request);
    const host = snapshot.expected_host_id, client = snapshot.client_id, session = snapshot.conversationId;
    if (!identity(host) || !identity(client) || !identity(session) || snapshot.tool_plan_ref !== undefined)
        throw new Error('Tool plan requires an unbound Host conversation');
    const response = await api('/api/agent/tool-plan/prepare', { method: 'POST',
        body: JSON.stringify({ expected_host_id: host, client_id: client, session_id: session }) });
    return { ...snapshot, tool_plan_ref: hostToolPlanReference(response, host, client, session) };
}
