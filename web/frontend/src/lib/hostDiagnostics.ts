export interface HostDiagnostics { service: string; mode: string; hostId: string; permissionProtocolVersion: number }
export function hostDiagnostics(value: unknown): HostDiagnostics {
    const data = value as { ok?: unknown; service?: unknown; mode?: unknown; host?: { id?: unknown; permissionProtocolVersion?: unknown } };
    if (data?.ok !== true || typeof data.service !== 'string' || !data.service.trim() || typeof data.mode !== 'string' || !data.mode.trim()
        || typeof data.host?.id !== 'string' || !data.host.id.trim() || !Number.isSafeInteger(data.host.permissionProtocolVersion) || Number(data.host.permissionProtocolVersion) < 1) throw new Error('Invalid host diagnostics');
    return { service: data.service, mode: data.mode, hostId: data.host.id, permissionProtocolVersion: Number(data.host.permissionProtocolVersion) };
}
export function hostDiagnosticReport(snapshot: HostDiagnostics): string {
    // Deliberate allowlist: never copy the raw response, configuration or logs.
    return JSON.stringify({ service: snapshot.service, mode: snapshot.mode, hostId: snapshot.hostId, permissionProtocolVersion: snapshot.permissionProtocolVersion }, null, 2);
}
