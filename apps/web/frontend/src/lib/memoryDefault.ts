export type MemoryDefault = { enabled: boolean; hostAllowed: boolean };
export function memoryDefault(value: unknown): MemoryDefault {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid memory default');
    const data = value as Record<string, unknown>;
    if (typeof data.enabled !== 'boolean' || typeof data.hostAllowed !== 'boolean') throw new Error('Invalid memory default');
    return { enabled: data.enabled, hostAllowed: data.hostAllowed };
}
export function memoryDefaultReceipt(value: unknown, enabled: boolean): MemoryDefault {
    const snapshot = memoryDefault(value);
    if (snapshot.enabled !== enabled) throw new Error('Unconfirmed memory default');
    return snapshot;
}
